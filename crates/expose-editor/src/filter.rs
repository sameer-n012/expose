//! Gaussian blur and unsharp mask on a pixel layer or a layer mask.
//!
//! # Use
//!
//! A filter dialog shows a live preview while the user changes the values.
//! For this, use a [`FilterSession`]:
//!
//! 1. [`FilterSession::begin`] keeps a copy of the original values.
//! 2. [`FilterSession::apply`] filters the **original** values with a
//!    [`FilterSpec`] and writes the result into the layer. Call it again
//!    for each new value: the result does not add to the result of the
//!    call before.
//! 3. [`FilterSession::commit`] returns one undo [`Edit`], or
//!    [`FilterSession::cancel`] restores the original values.
//!
//! [`Document::apply_filter`] does all three steps in one call.
//!
//! # Target
//!
//! - [`FilterTarget::Pixels`]: the pixels of a pixel layer.
//! - [`FilterTarget::Mask`]: the mask values of a layer that has a mask.
//!
//! # Area
//!
//! Only the values inside the canvas and inside the layer (or mask) buffer
//! can change. If the document has a selection, only the values inside the
//! bounding box of the selection can change, and the result is
//! `lerp(original, filtered, selection)`. The session keeps the selection
//! of [`FilterSession::begin`]: a later change of the selection has no
//! effect on the session.
//!
//! The filter reads the values around the area (also values outside the
//! canvas), as far as the filter kernel reaches. Outside the buffer, the
//! filter uses the nearest edge value of the buffer (clamp to edge).
//!
//! The session keeps the original values of the area only.
//!
//! # Gaussian blur
//!
//! [`FilterSpec::GaussianBlur`] blurs with a Gaussian kernel with the
//! standard deviation `radius` (pixels). The blur works on the
//! premultiplied linear values. Thus a transparent pixel adds no color,
//! and there is no dark edge at a transparent border. A mask blur works on
//! the mask values.
//!
//! With the transparency lock, the alpha does not change: the result is
//! the straight color of the blurred pixel times the original alpha. A
//! pixel with the alpha 0 does not change.
//!
//! # Kernel
//!
//! - For `radius <` [`EXACT_MAX_SIGMA`] (2.0), the blur uses the sampled
//!   Gaussian kernel, one pass on each axis. The kernel ends at
//!   `ceil(3 * radius)` pixels and its weights are normalized to the sum
//!   1. The cost per pixel increases with the radius.
//! - For a larger radius, the blur uses [`BOX_PASSES`] (4) extended box
//!   blurs in sequence on each axis (a box with a fractional weight on its
//!   two end pixels). Their variance is exactly the variance of the
//!   Gaussian, and the result changes smoothly with the radius. The cost
//!   per pixel does not change with the radius (running sums). The
//!   difference to the exact Gaussian is less than 0.8 % of the step
//!   height on a step edge, and less than 4 % of the peak value of the
//!   impulse response (tests `box_blur_step_is_close_to_exact` and
//!   `box_blur_impulse_is_close_to_exact`). Below 2.0 the difference
//!   increases (6 % of the peak at 1.0), so the switch is at 2.0.
//!
//! The filter is deterministic: the same input and values give the same
//! bits.
//!
//! # Unsharp mask
//!
//! [`FilterSpec::UnsharpMask`] works on the straight color, encoded in the
//! document [`BlendSpace`] (`x`):
//!
//! ```text
//! b = blur(x)                  (alpha-weighted Gaussian blur)
//! d = x - b                    (for each color channel)
//! y = x + amount * d           if |d| * 255 > threshold
//! y = x                        else
//! ```
//!
//! The blur of `x` is `blur(alpha * x) / blur(alpha)`, so that transparent
//! pixels (which have no color) do not change the mean. The alpha does not
//! change. A pixel with the alpha 0 does not change. The result `y` is
//! decoded and premultiplied with the original alpha. A channel that does
//! not change keeps its original value exactly. The result is not clamped
//! (the working space permits values outside `0..=1`). For
//! [`BlendSpace::Perceptual`], the encode and the decode use tables of the
//! sRGB curve with a relative error of less than `1e-6`.
//!
//! For a mask, the same rule applies to the mask values (`b = blur(x)`),
//! and the result is clamped to `0..=1`.
//!
//! # Locks and conflicts
//!
//! - The pixels lock or the `all` lock (on the layer or on a parent group)
//!   gives [`EditorError::Locked`] for both targets.
//! - If the layer changes outside the session (a new pixel or mask version,
//!   a new offset, size, or new locks), the session calls give
//!   [`EditorError::StrokeConflict`].

use std::fmt;

use expose_core::buffer::{Rect, Rgba};
use expose_core::color::TransferFunction;

use crate::blend::{BlendSpace, premultiply, unpremultiply};
use crate::document::Document;
use crate::error::{EditorError, EditorResult, check_range};
use crate::layer::{Layer, LayerId, LayerKind, LockKind, Locks, MaskImage};
use crate::ops::Edit;

#[cfg(test)]
mod tests;

/// The smallest filter radius (Gaussian standard deviation) in pixels.
pub const MIN_FILTER_RADIUS: f32 = 0.1;
/// The largest filter radius (Gaussian standard deviation) in pixels.
pub const MAX_FILTER_RADIUS: f32 = 1000.0;
/// The smallest unsharp mask amount (`1.0` = 100 %).
pub const MIN_UNSHARP_AMOUNT: f32 = 0.01;
/// The largest unsharp mask amount (`1.0` = 100 %).
pub const MAX_UNSHARP_AMOUNT: f32 = 5.0;
/// The largest unsharp mask threshold, in 8-bit levels.
pub const MAX_UNSHARP_THRESHOLD: f32 = 255.0;

/// Below this radius, the blur uses the sampled Gaussian kernel. At and
/// above it, the blur uses box blurs (see "Kernel" in the
/// [module documentation](self)).
pub const EXACT_MAX_SIGMA: f32 = 2.0;
/// The number of box blurs on each axis for a large radius.
pub const BOX_PASSES: usize = 4;

/// The sampled kernel ends at `ceil(KERNEL_EXTENT * sigma)` pixels.
const KERNEL_EXTENT: f32 = 3.0;

/// A filter and its values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterSpec {
    /// A Gaussian blur.
    GaussianBlur {
        /// Standard deviation in pixels
        /// ([`MIN_FILTER_RADIUS`]`..=`[`MAX_FILTER_RADIUS`]).
        radius: f32,
    },
    /// An unsharp mask.
    UnsharpMask {
        /// The strength (`1.0` = 100 %,
        /// [`MIN_UNSHARP_AMOUNT`]`..=`[`MAX_UNSHARP_AMOUNT`]).
        amount: f32,
        /// Standard deviation of the blur in pixels
        /// ([`MIN_FILTER_RADIUS`]`..=`[`MAX_FILTER_RADIUS`]).
        radius: f32,
        /// The smallest difference that changes a value, in 8-bit levels
        /// (`0..=`[`MAX_UNSHARP_THRESHOLD`]).
        threshold: f32,
    },
}

impl FilterSpec {
    /// Returns an error if a value is not finite
    /// ([`EditorError::NonFinite`]) or out of its range
    /// ([`EditorError::OutOfRange`]).
    pub fn validate(&self) -> EditorResult<()> {
        match *self {
            Self::GaussianBlur { radius } => {
                check_range("radius", radius, MIN_FILTER_RADIUS, MAX_FILTER_RADIUS)
            }
            Self::UnsharpMask {
                amount,
                radius,
                threshold,
            } => {
                check_range("amount", amount, MIN_UNSHARP_AMOUNT, MAX_UNSHARP_AMOUNT)?;
                check_range("radius", radius, MIN_FILTER_RADIUS, MAX_FILTER_RADIUS)?;
                check_range("threshold", threshold, 0.0, MAX_UNSHARP_THRESHOLD)
            }
        }
    }

    /// The Gaussian standard deviation in pixels.
    const fn sigma(&self) -> f32 {
        match *self {
            Self::GaussianBlur { radius } | Self::UnsharpMask { radius, .. } => radius,
        }
    }
}

/// The values that a filter changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterTarget {
    /// The pixels of a pixel layer.
    Pixels,
    /// The mask values of a layer.
    Mask,
}

impl FilterTarget {
    /// The number of `f32` values per pixel.
    const fn channels(self) -> usize {
        match self {
            Self::Pixels => 4,
            Self::Mask => 1,
        }
    }
}

/// The layer state that the session expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Expected {
    version: u64,
    offset: (i32, i32),
    size: (u32, u32),
    locks: Locks,
}

/// An active filter session. See the [module documentation](self).
pub struct FilterSession {
    layer: LayerId,
    target: FilterTarget,
    expected: Expected,
    /// The area that the filter can change, in buffer coordinates, or
    /// `None` if it is empty.
    region: Option<Rect>,
    /// The original values of `region` (row by row, channels interleaved).
    original: Vec<f32>,
    /// The selection values (clamped to `0..=1`) of `region`, or `None`
    /// without a selection.
    selection: Option<Vec<f32>>,
}

impl fmt::Debug for FilterSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilterSession")
            .field("layer", &self.layer)
            .field("target", &self.target)
            .field("expected", &self.expected)
            .field("region", &self.region)
            .field("original_len", &self.original.len())
            .field("has_selection", &self.selection.is_some())
            .finish()
    }
}

impl FilterSession {
    /// Starts a filter session on a layer. The session keeps the original
    /// values of the area that the filter can change.
    ///
    /// Errors:
    /// - [`EditorError::LayerNotFound`].
    /// - [`EditorError::NotAPixelLayer`] for [`FilterTarget::Pixels`] on a
    ///   layer that is not a pixel layer.
    /// - [`EditorError::NoMask`] for [`FilterTarget::Mask`] on a layer
    ///   without a mask.
    /// - [`EditorError::Locked`] if the pixels lock (or `all`) is set on
    ///   the layer or on a parent group.
    pub fn begin(doc: &Document, layer: LayerId, target: FilterTarget) -> EditorResult<Self> {
        let l = doc.layer(layer).ok_or(EditorError::LayerNotFound(layer))?;
        let expected = expected_state(l, target, doc.effective_locks(layer)?)?;
        expected.locks.check(layer, LockKind::Pixels)?;
        let region = filter_region(doc, expected);
        let (original, selection) = match region {
            None => (Vec::new(), None),
            Some(r) => {
                let (buf, _) = buffer(l, target).ok_or(EditorError::StrokeConflict(layer))?;
                let original = copy_rect(buf, expected.size.0, target.channels(), r);
                let selection = doc
                    .selection()
                    .map(|s| selection_weights(s, doc.width(), r, expected.offset));
                (original, selection)
            }
        };
        Ok(Self {
            layer,
            target,
            expected,
            region,
            original,
            selection,
        })
    }

    /// The layer of the session.
    pub const fn layer(&self) -> LayerId {
        self.layer
    }

    /// The target of the session.
    pub const fn target(&self) -> FilterTarget {
        self.target
    }

    /// Filters the original values (the values at
    /// [`FilterSession::begin`]) with `spec` and writes the result into the
    /// layer.
    ///
    /// Returns the canvas rectangle (clipped to the canvas) of the values
    /// that changed in this call, or `None` if no value changed. If a value
    /// changed, the layer gets a new pixel (or mask) version and the
    /// document a new revision.
    ///
    /// Errors: bad values ([`FilterSpec::validate`]);
    /// [`EditorError::StrokeConflict`] if the layer changed outside the
    /// session. On an error, nothing changes.
    pub fn apply(&mut self, doc: &mut Document, spec: &FilterSpec) -> EditorResult<Option<Rect>> {
        spec.validate()?;
        self.check(doc)?;
        let Some(region) = self.region else {
            return Ok(None);
        };
        let space = doc.blend_space();
        let canvas = (doc.width(), doc.height());
        let keep_alpha = self.expected.locks.transparency;
        let conflict = EditorError::StrokeConflict(self.layer);
        let l = doc.layer_mut(self.layer)?;
        let (buf, buf_w) = buffer_mut(l, self.target).ok_or(conflict)?;
        let plan = Plan::new(spec.sigma());
        let buf_h = self.expected.size.1;
        let win = Window::around(region, plan.margin(), buf_w, buf_h);
        let c = self.target.channels();
        let mut src = source_window(buf, buf_w, c, win.rect, region, &self.original);
        let ctx = WriteContext {
            region,
            win: win.rect,
            original: &self.original,
            selection: self.selection.as_deref(),
            buf_w,
        };
        let changed = match (self.target, *spec) {
            (FilterTarget::Pixels, FilterSpec::GaussianBlur { .. }) => {
                let blurred = blur::<4>(src, win.size(), &plan);
                ctx.write::<4>(buf, |i, o| {
                    let f = &blurred[i..i + 4];
                    let f: Rgba = [f[0], f[1], f[2], f[3]];
                    if !keep_alpha {
                        return f;
                    }
                    if o[3] <= 0.0 {
                        return o;
                    }
                    premultiply(unpremultiply(f).0, o[3])
                })
            }
            (
                FilterTarget::Pixels,
                FilterSpec::UnsharpMask {
                    amount, threshold, ..
                },
            ) => {
                let transfer = Transfer::new(space);
                // The straight encoded colors `x` (for the rule) and the
                // premultiplied encoded colors (for the blur).
                let mut straight = vec![0.0_f32; src.len() / 4 * 3];
                for (p, x) in src.chunks_exact_mut(4).zip(straight.chunks_exact_mut(3)) {
                    let (c, a) = unpremultiply([p[0], p[1], p[2], p[3]]);
                    let e = transfer.encode(c);
                    x.copy_from_slice(&e);
                    p.copy_from_slice(&premultiply(e, a));
                }
                let blurred = blur::<4>(src, win.size(), &plan);
                let rule = Unsharp { amount, threshold };
                ctx.write::<4>(buf, |i, o| {
                    let x = &straight[i / 4 * 3..i / 4 * 3 + 3];
                    rule.pixel(o, [x[0], x[1], x[2]], &blurred[i..i + 4], &transfer)
                })
            }
            (FilterTarget::Mask, FilterSpec::GaussianBlur { .. }) => {
                let blurred = blur::<1>(src, win.size(), &plan);
                ctx.write::<1>(buf, |i, _| [blurred[i]])
            }
            (
                FilterTarget::Mask,
                FilterSpec::UnsharpMask {
                    amount, threshold, ..
                },
            ) => {
                let blurred = blur::<1>(src, win.size(), &plan);
                let rule = Unsharp { amount, threshold };
                ctx.write::<1>(buf, |i, o| [rule.value(o[0], blurred[i])])
            }
        };
        let Some(rect) = changed else {
            return Ok(None);
        };
        self.touch(doc, Some(rect))?;
        Ok(canvas_rect(rect, self.expected.offset, canvas))
    }

    /// Ends the session and returns one undo edit for it. Give the edit to
    /// [`History::push`](crate::History::push). The edit holds the original
    /// values of the changed area only. Returns `None` if no value is
    /// different from the original.
    ///
    /// Returns [`EditorError::StrokeConflict`] if the layer changed outside
    /// the session. Then the layer keeps the filtered values without an
    /// undo edit.
    pub fn commit(self, doc: &mut Document) -> EditorResult<Option<Edit>> {
        self.check(doc)?;
        let Some(region) = self.region else {
            return Ok(None);
        };
        let conflict = EditorError::StrokeConflict(self.layer);
        let l = doc.layer(self.layer).ok_or(conflict.clone())?;
        let (buf, buf_w) = buffer(l, self.target).ok_or(conflict)?;
        let c = self.target.channels();
        let Some(rect) = diff_rect(buf, buf_w, c, region, &self.original) else {
            return Ok(None);
        };
        let tile = copy_rect(&self.original, region.width, c, local(rect, region));
        let edit = match self.target {
            FilterTarget::Pixels => {
                let tile = tile
                    .chunks_exact(4)
                    .map(|p| [p[0], p[1], p[2], p[3]])
                    .collect();
                Edit::pixel_tiles(self.layer, vec![(rect, tile)])
            }
            FilterTarget::Mask => Edit::mask_tiles(self.layer, vec![(rect, tile)]),
        };
        Ok(Some(edit))
    }

    /// Ends the session and restores the original values of the layer.
    ///
    /// Returns [`EditorError::StrokeConflict`] (and changes nothing) if the
    /// layer changed outside the session.
    pub fn cancel(mut self, doc: &mut Document) -> EditorResult<()> {
        self.check(doc)?;
        let Some(region) = self.region else {
            return Ok(());
        };
        let conflict = EditorError::StrokeConflict(self.layer);
        let l = doc.layer_mut(self.layer)?;
        let (buf, buf_w) = buffer_mut(l, self.target).ok_or(conflict)?;
        let c = self.target.channels();
        let changed = diff_rect(buf, buf_w, c, region, &self.original);
        if let Some(rect) = changed {
            let tile = copy_rect(&self.original, region.width, c, local(rect, region));
            paste_rect(buf, buf_w, c, rect, (0, 0), &tile);
            self.touch(doc, Some(rect))?;
        }
        Ok(())
    }

    /// Returns [`EditorError::StrokeConflict`] if the layer is not in the
    /// state that the session expects.
    fn check(&self, doc: &Document) -> EditorResult<()> {
        let conflict = EditorError::StrokeConflict(self.layer);
        let l = doc.layer(self.layer).ok_or(conflict.clone())?;
        let locks = doc.effective_locks(self.layer)?;
        match expected_state(l, self.target, locks) {
            Ok(state) if state == self.expected => Ok(()),
            _ => Err(conflict),
        }
    }

    /// Gives the layer a new content version and the document a new
    /// revision, and records the new version. `rect` is the changed area
    /// in buffer coordinates.
    fn touch(&mut self, doc: &mut Document, rect: Option<Rect>) -> EditorResult<()> {
        let layer = doc.layer_mut(self.layer)?;
        let version = match self.target {
            FilterTarget::Mask => {
                match rect {
                    Some(r) => layer.touch_mask_in(r),
                    None => layer.touch_mask(),
                }
                layer.mask_version()
            }
            FilterTarget::Pixels => {
                match rect {
                    Some(r) => layer.touch_pixels_in(r),
                    None => layer.touch_pixels(),
                }
                layer.pixel_version()
            }
        };
        self.expected.version = version;
        doc.touch();
        Ok(())
    }
}

impl Document {
    /// Applies a filter to a layer in one step (see the
    /// [module documentation](crate::filter)) and returns one undo edit, or
    /// `None` if no value changed.
    ///
    /// Errors: see [`FilterSession::begin`] and [`FilterSpec::validate`].
    /// On an error, nothing changes.
    pub fn apply_filter(
        &mut self,
        layer: LayerId,
        target: FilterTarget,
        spec: &FilterSpec,
    ) -> EditorResult<Option<Edit>> {
        spec.validate()?;
        let mut session = FilterSession::begin(self, layer, target)?;
        session.apply(self, spec)?;
        session.commit(self)
    }
}

/// Returns the layer state for a target, or an error if the layer does
/// not fit the target.
fn expected_state(l: &Layer, target: FilterTarget, locks: Locks) -> EditorResult<Expected> {
    let id = l.id();
    match target {
        FilterTarget::Mask => {
            let m = l.mask().ok_or(EditorError::NoMask(id))?;
            Ok(Expected {
                version: l.mask_version(),
                offset: m.offset,
                size: (m.image.width(), m.image.height()),
                locks,
            })
        }
        FilterTarget::Pixels => {
            let p = l.as_pixel().ok_or(EditorError::NotAPixelLayer(id))?;
            Ok(Expected {
                version: l.pixel_version(),
                offset: p.offset(),
                size: (p.image().width(), p.image().height()),
                locks,
            })
        }
    }
}

/// The values of the target as a flat slice, and the buffer width.
fn buffer(l: &Layer, target: FilterTarget) -> Option<(&[f32], u32)> {
    match target {
        FilterTarget::Pixels => {
            let p = l.as_pixel()?;
            Some((p.image().pixels().as_flattened(), p.image().width()))
        }
        FilterTarget::Mask => {
            let m = l.mask()?;
            Some((m.image.data(), m.image.width()))
        }
    }
}

/// The values of the target as a flat mutable slice, and the buffer width.
fn buffer_mut(l: &mut Layer, target: FilterTarget) -> Option<(&mut [f32], u32)> {
    match target {
        FilterTarget::Pixels => {
            let LayerKind::Pixel(p) = l.kind_mut() else {
                return None;
            };
            let w = p.image().width();
            Some((p.image_mut().pixels_mut().as_flattened_mut(), w))
        }
        FilterTarget::Mask => {
            let m = l.mask_mut()?;
            let w = m.image.width();
            Some((m.image.data_mut(), w))
        }
    }
}

/// The area that a filter can change, in buffer coordinates: the buffer
/// inside the canvas, and inside the selection bounds if the document has
/// a selection. `None` if the area is empty.
fn filter_region(doc: &Document, e: Expected) -> Option<Rect> {
    let (cw, ch) = (i64::from(doc.width()), i64::from(doc.height()));
    let (x0, y0, x1, y1) = match doc.selection() {
        Some(s) => selection_bounds(s)?,
        None => (0, 0, cw, ch),
    };
    let (ox, oy) = (i64::from(e.offset.0), i64::from(e.offset.1));
    let bx0 = (x0 - ox).max(0);
    let by0 = (y0 - oy).max(0);
    let bx1 = (x1 - ox).min(i64::from(e.size.0));
    let by1 = (y1 - oy).min(i64::from(e.size.1));
    if bx1 <= bx0 || by1 <= by0 {
        return None;
    }
    // All four values are in `0..=buffer size` (u32) after the clamps.
    Some(Rect::new(
        u32::try_from(bx0).ok()?,
        u32::try_from(by0).ok()?,
        u32::try_from(bx1 - bx0).ok()?,
        u32::try_from(by1 - by0).ok()?,
    ))
}

/// The bounds `(x0, y0, x1, y1)` (end exclusive) of the selection values
/// more than 0, or `None` if all values are 0.
fn selection_bounds(s: &MaskImage) -> Option<(i64, i64, i64, i64)> {
    let w = s.width() as usize;
    if w == 0 {
        return None;
    }
    let mut b: Option<(usize, usize, usize, usize)> = None;
    for (y, row) in s.data().chunks_exact(w).enumerate() {
        let Some(first) = row.iter().position(|&v| v > 0.0) else {
            continue;
        };
        // `first` exists, so `rposition` finds a value.
        let last = row.iter().rposition(|&v| v > 0.0).unwrap_or(first);
        b = Some(match b {
            None => (first, y, last + 1, y + 1),
            Some((x0, y0, x1, _)) => (x0.min(first), y0, x1.max(last + 1), y + 1),
        });
    }
    let (x0, y0, x1, y1) = b?;
    let i = |v: usize| i64::try_from(v).ok();
    Some((i(x0)?, i(y0)?, i(x1)?, i(y1)?))
}

/// The selection values (clamped to `0..=1`) for the buffer area `region`
/// of a buffer at the canvas position `offset`. The region is inside the
/// canvas (see [`filter_region`]).
// Canvas positions in the region are in `0..canvas size` (u32).
#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
fn selection_weights(s: &MaskImage, canvas_w: u32, region: Rect, offset: (i32, i32)) -> Vec<f32> {
    let data = s.data();
    let cw = canvas_w as usize;
    let mut out = Vec::with_capacity(pixels_in(region));
    for y in region.y..region.y + region.height {
        let cy = (i64::from(y) + i64::from(offset.1)) as usize;
        let cx = (i64::from(region.x) + i64::from(offset.0)) as usize;
        let start = cy * cw + cx;
        let row = &data[start..start + region.width as usize];
        out.extend(row.iter().map(|v| v.clamp(0.0, 1.0)));
    }
    out
}

/// The number of pixels in `rect`.
const fn pixels_in(rect: Rect) -> usize {
    rect.width as usize * rect.height as usize
}

/// Copies the area `rect` of a flat buffer (width `buf_w`, `c` values per
/// pixel) into a new vector. `rect` must be inside the buffer.
fn copy_rect(buf: &[f32], buf_w: u32, c: usize, rect: Rect) -> Vec<f32> {
    let bw = buf_w as usize;
    let rw = rect.width as usize * c;
    let mut out = Vec::with_capacity(pixels_in(rect) * c);
    for y in rect.y as usize..rect.y as usize + rect.height as usize {
        let start = (y * bw + rect.x as usize) * c;
        out.extend_from_slice(&buf[start..start + rw]);
    }
    out
}

/// The input of the filter: the values of the window `win` (buffer
/// coordinates) of the current buffer, with the original values in
/// `region` (inside `win`).
fn source_window(
    buf: &[f32],
    buf_w: u32,
    c: usize,
    win: Rect,
    region: Rect,
    original: &[f32],
) -> Vec<f32> {
    let bw = buf_w as usize;
    let (wx, ww) = (win.x as usize, win.width as usize);
    let (rx, rw) = (region.x as usize, region.width as usize);
    let (ry, rh) = (region.y as usize, region.height as usize);
    let mut out = Vec::with_capacity(pixels_in(win) * c);
    for y in win.y as usize..win.y as usize + win.height as usize {
        let row = &buf[(y * bw + wx) * c..(y * bw + wx + ww) * c];
        if (ry..ry + rh).contains(&y) {
            let (left, right) = ((rx - wx) * c, (rx - wx + rw) * c);
            let orig = &original[(y - ry) * rw * c..(y - ry + 1) * rw * c];
            out.extend_from_slice(&row[..left]);
            out.extend_from_slice(orig);
            out.extend_from_slice(&row[right..]);
        } else {
            out.extend_from_slice(row);
        }
    }
    out
}

/// Writes `data` (the values of the area `rect`) into a flat buffer of
/// width `buf_w` whose top-left value is at the position `origin` (in the
/// coordinates of `rect`). `rect` must be inside the buffer area.
fn paste_rect(buf: &mut [f32], buf_w: u32, c: usize, rect: Rect, origin: (u32, u32), data: &[f32]) {
    let bw = buf_w as usize;
    let rw = rect.width as usize * c;
    let (dx, dy) = ((rect.x - origin.0) as usize, (rect.y - origin.1) as usize);
    for (row, src) in data.chunks_exact(rw).enumerate() {
        let start = ((dy + row) * bw + dx) * c;
        buf[start..start + rw].copy_from_slice(src);
    }
}

/// `rect` in the coordinates of `outer` (`rect` must be inside `outer`).
const fn local(rect: Rect, outer: Rect) -> Rect {
    Rect::new(rect.x - outer.x, rect.y - outer.y, rect.width, rect.height)
}

/// The bounds (buffer coordinates) of the pixels in `region` whose values
/// have other bits than `data`, or `None` if all values are the same.
fn diff_rect(buf: &[f32], buf_w: u32, c: usize, region: Rect, data: &[f32]) -> Option<Rect> {
    let bw = buf_w as usize;
    let rw = region.width as usize * c;
    let mut b = Bounds::default();
    for (row, old) in data.chunks_exact(rw).enumerate() {
        let y = region.y as usize + row;
        let start = (y * bw + region.x as usize) * c;
        let cur = &buf[start..start + rw];
        for (px, (a, o)) in cur.chunks_exact(c).zip(old.chunks_exact(c)).enumerate() {
            if !same_bits(a, o) {
                b.add(region.x as usize + px, y);
            }
        }
    }
    b.rect()
}

/// `true` if two value slices have the same bits.
fn same_bits(a: &[f32], b: &[f32]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

/// Bounds of changed pixels (end exclusive).
#[derive(Debug, Default, Clone, Copy)]
struct Bounds {
    b: Option<(usize, usize, usize, usize)>,
}

impl Bounds {
    fn add(&mut self, x: usize, y: usize) {
        self.b = Some(match self.b {
            None => (x, y, x + 1, y + 1),
            Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1)),
        });
    }

    fn rect(self) -> Option<Rect> {
        let (x0, y0, x1, y1) = self.b?;
        let u = |v: usize| u32::try_from(v).ok();
        Some(Rect::new(u(x0)?, u(y0)?, u(x1 - x0)?, u(y1 - y0)?))
    }
}

/// Converts a buffer rectangle to a canvas rectangle, clipped to the
/// canvas. `None` if it is fully outside the canvas.
fn canvas_rect(r: Rect, offset: (i32, i32), canvas: (u32, u32)) -> Option<Rect> {
    let (ox, oy) = (i64::from(offset.0), i64::from(offset.1));
    let x0 = (i64::from(r.x) + ox).max(0);
    let y0 = (i64::from(r.y) + oy).max(0);
    let x1 = (i64::try_from(r.right()).ok()? + ox).min(i64::from(canvas.0));
    let y1 = (i64::try_from(r.bottom()).ok()? + oy).min(i64::from(canvas.1));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(Rect::new(
        u32::try_from(x0).ok()?,
        u32::try_from(y0).ok()?,
        u32::try_from(x1 - x0).ok()?,
        u32::try_from(y1 - y0).ok()?,
    ))
}

/// The area that the filter reads: the region plus a margin, clamped to
/// the buffer.
struct Window {
    rect: Rect,
}

impl Window {
    fn around(region: Rect, margin: usize, buf_w: u32, buf_h: u32) -> Self {
        let m = u32::try_from(margin).unwrap_or(u32::MAX);
        let x0 = region.x.saturating_sub(m);
        let y0 = region.y.saturating_sub(m);
        // `region` is inside the buffer, so `right() <= buf_w` (u32).
        let x1 = u32::try_from(region.right()).unwrap_or(buf_w);
        let y1 = u32::try_from(region.bottom()).unwrap_or(buf_h);
        let x1 = x1.saturating_add(m).min(buf_w);
        let y1 = y1.saturating_add(m).min(buf_h);
        Self {
            rect: Rect::new(x0, y0, x1 - x0, y1 - y0),
        }
    }

    const fn size(&self) -> (usize, usize) {
        (self.rect.width as usize, self.rect.height as usize)
    }
}

/// The data to write the filter result into the buffer.
struct WriteContext<'a> {
    /// The area to write (buffer coordinates).
    region: Rect,
    /// The area of the filter result (buffer coordinates).
    win: Rect,
    /// The original values of `region`.
    original: &'a [f32],
    /// The selection values of `region`.
    selection: Option<&'a [f32]>,
    buf_w: u32,
}

impl WriteContext<'_> {
    /// Writes `lerp(original, f(i, original), selection)` for each pixel
    /// of the region into `buf` (`C` values per pixel). `i` is the index of
    /// the first value of the pixel in the filter result (window).
    ///
    /// Returns the bounds (buffer coordinates) of the pixels whose values
    /// changed, or `None`.
    fn write<const C: usize>(
        &self,
        buf: &mut [f32],
        f: impl Fn(usize, [f32; C]) -> [f32; C],
    ) -> Option<Rect> {
        let bw = self.buf_w as usize;
        let ww = self.win.width as usize;
        let rw = self.region.width as usize;
        let rx = self.region.x as usize;
        let (dx, dy) = (
            (self.region.x - self.win.x) as usize,
            (self.region.y - self.win.y) as usize,
        );
        let mut b = Bounds::default();
        for row in 0..self.region.height as usize {
            let y = self.region.y as usize + row;
            let orig = &self.original[row * rw * C..(row + 1) * rw * C];
            let start = (y * bw + rx) * C;
            let cur_row = &mut buf[start..start + rw * C];
            let sel = self.selection.map(|s| &s[row * rw..(row + 1) * rw]);
            let fi0 = ((dy + row) * ww + dx) * C;
            let mut first: Option<usize> = None;
            let mut last = 0;
            for (px, (o, cur)) in orig
                .chunks_exact(C)
                .zip(cur_row.chunks_exact_mut(C))
                .enumerate()
            {
                let s = sel.map_or(1.0, |s| s[px]);
                let mut old = [0.0_f32; C];
                old.copy_from_slice(o);
                let new = if s <= 0.0 {
                    old
                } else {
                    let f = f(fi0 + px * C, old);
                    if s >= 1.0 {
                        f
                    } else {
                        std::array::from_fn(|ch| old[ch] + (f[ch] - old[ch]) * s)
                    }
                };
                if !same_bits(cur, &new) {
                    cur.copy_from_slice(&new);
                    first.get_or_insert(px);
                    last = px;
                }
            }
            if let Some(x0) = first {
                b.add(rx + x0, y);
                b.add(rx + last, y);
            }
        }
        b.rect()
    }
}

/// The unsharp mask rule (see the [module documentation](self)).
#[derive(Debug, Clone, Copy)]
struct Unsharp {
    amount: f32,
    threshold: f32,
}

impl Unsharp {
    /// The sharpened value of `x` with the blurred value `b`, or `None` if
    /// the difference is not more than the threshold.
    fn sharpen(self, x: f32, b: f32) -> Option<f32> {
        let d = x - b;
        (d.abs() * 255.0 > self.threshold).then_some(x + self.amount * d)
    }

    /// The result for a mask value (clamped to `0..=1`).
    fn value(self, x: f32, b: f32) -> f32 {
        self.sharpen(x, b).map_or(x, |y| y.clamp(0.0, 1.0))
    }

    /// The result for a premultiplied linear pixel `o`. `x` is its straight
    /// color in the blend space, `b` the blur of the window, premultiplied
    /// in the blend space.
    fn pixel(self, o: Rgba, x: [f32; 3], b: &[f32], t: &Transfer) -> Rgba {
        let a = o[3];
        if a <= 0.0 || b[3] <= 0.0 {
            return o;
        }
        let ib = 1.0 / b[3];
        let (c, _) = unpremultiply(o);
        let mut out = c;
        let mut changed = false;
        for ch in 0..3 {
            if let Some(v) = self.sharpen(x[ch], b[ch] * ib) {
                out[ch] = t.decode_one(v);
                changed = true;
            }
        }
        if changed { premultiply(out, a) } else { o }
    }
}

/// The transfer function of the blend space, with tables for the sRGB
/// curve of [`BlendSpace::Perceptual`] (the unsharp mask encodes and
/// decodes each pixel, and `powf` is slow).
///
/// Inside the table ranges, the relative error to the exact curve is less
/// than `1e-6` (test `transfer_tables_are_close_to_exact`). Outside the
/// ranges (also for negative values), the exact curve applies.
struct Transfer {
    tables: Option<(PowTable, PowTable)>,
}

impl Transfer {
    fn new(space: BlendSpace) -> Self {
        let tables = match space {
            BlendSpace::Linear => None,
            BlendSpace::Perceptual => Some((
                // Encode: the power part starts at 0.0031308 < 2^-8.
                PowTable::new(-8, 12, 9, |v| TransferFunction::Srgb.encode(v)),
                // Decode: the power part starts at 0.04045 < 2^-4.
                PowTable::new(-4, 7, 10, |v| TransferFunction::Srgb.decode(v)),
            )),
        };
        Self { tables }
    }

    fn encode(&self, c: [f32; 3]) -> [f32; 3] {
        match &self.tables {
            None => c,
            Some((enc, _)) => c.map(|v| {
                enc.get(v)
                    .unwrap_or_else(|| TransferFunction::Srgb.encode(v))
            }),
        }
    }

    fn decode_one(&self, v: f32) -> f32 {
        match &self.tables {
            None => v,
            Some((_, dec)) => dec
                .get(v)
                .unwrap_or_else(|| TransferFunction::Srgb.decode(v)),
        }
    }
}

/// A table of a smooth function on `2^emin..2^(emin + octaves)`, with
/// `2^bits` linear segments per octave. The segment of a value comes from
/// the bits of its float exponent and mantissa.
struct PowTable {
    lo: f32,
    hi: f32,
    emin: i32,
    bits: u32,
    values: Vec<f32>,
}

impl PowTable {
    // The entries are `f32` values of the function at exact powers of 2
    // times `1 + i / 2^bits`; all casts are of small integers.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_possible_wrap
    )]
    fn new(emin: i32, octaves: u32, bits: u32, f: impl Fn(f32) -> f32) -> Self {
        let per = 1_usize << bits;
        let n = octaves as usize * per;
        let values = (0..=n)
            .map(|i| {
                let e = emin + (i / per) as i32;
                let x = 2.0_f64.powi(e) * (1.0 + (i % per) as f64 / per as f64);
                f(x as f32)
            })
            .collect();
        Self {
            lo: 2.0_f32.powi(emin),
            hi: 2.0_f32.powi(emin + octaves as i32),
            emin,
            bits,
            values,
        }
    }

    /// The table value for `v`, or `None` outside the range (also for NaN).
    // The exponent bits are at most 255 and the mantissa parts are less
    // than 2^23, so the casts keep the values.
    #[allow(
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        clippy::cast_possible_wrap
    )]
    fn get(&self, v: f32) -> Option<f32> {
        if !(v >= self.lo && v < self.hi) {
            return None;
        }
        let b = v.to_bits();
        // `v` is a positive normal number in the range, so its exponent is
        // `emin..emin + octaves` and the octave index is not negative.
        let octave = (((b >> 23) & 0xff) as i32 - 127 - self.emin) as usize;
        let shift = 23 - self.bits;
        let seg = ((b >> shift) & ((1 << self.bits) - 1)) as usize;
        let i = (octave << self.bits) | seg;
        let frac = (b & ((1 << shift) - 1)) as f32 / (1_u32 << shift) as f32;
        let (a, c) = (self.values[i], self.values[i + 1]);
        Some(a + (c - a) * frac)
    }
}

/// How the blur is computed for one radius.
#[derive(Debug, Clone, PartialEq)]
enum Plan {
    /// The sampled Gaussian kernel (weights for `-r..=r`).
    Exact { weights: Vec<f32> },
    /// [`BOX_PASSES`] extended box blurs.
    Boxes(ExtendedBox),
}

impl Plan {
    fn new(sigma: f32) -> Self {
        if sigma < EXACT_MAX_SIGMA {
            Self::Exact {
                weights: gaussian_weights(sigma),
            }
        } else {
            Self::Boxes(ExtendedBox::new(f64::from(sigma)))
        }
    }

    /// How far (pixels) the blur reads from a pixel.
    fn margin(&self) -> usize {
        match self {
            Self::Exact { weights } => weights.len() / 2,
            Self::Boxes(b) => BOX_PASSES * b.reach(),
        }
    }
}

/// An extended box kernel: the weight 1 for the offsets `-r..=r`, and the
/// weight `alpha` (`0..1`) for the offsets `-(r + 1)` and `r + 1`,
/// normalized to the sum 1. Unlike a box of integer width, its variance
/// changes continuously with `alpha`, so that [`BOX_PASSES`] passes give
/// exactly the variance of the Gaussian, and the blur changes smoothly
/// with the radius.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ExtendedBox {
    r: usize,
    alpha: f64,
}

impl ExtendedBox {
    /// The kernel for which [`BOX_PASSES`] passes have the variance
    /// `sigma^2`.
    // `sigma <= MAX_FILTER_RADIUS`, so `r` is a small positive integer.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    fn new(sigma: f64) -> Self {
        let s2 = sigma * sigma / BOX_PASSES as f64;
        // The largest `r` with the box variance `r (r + 1) / 3 <= s2`.
        let mut r = (((12.0 * s2 + 1.0).sqrt() - 1.0) / 2.0).floor().max(0.0) as usize;
        let var = |r: usize| (r * (r + 1)) as f64 / 3.0;
        // Correct a float rounding of the square root.
        while var(r + 1) <= s2 {
            r += 1;
        }
        while r > 0 && var(r) > s2 {
            r -= 1;
        }
        let rf = r as f64;
        let alpha = (2.0 * rf + 1.0) * (s2 - var(r)) / (2.0 * ((rf + 1.0) * (rf + 1.0) - s2));
        Self {
            r,
            alpha: alpha.clamp(0.0, 1.0),
        }
    }

    /// How far (pixels) one pass reads from a pixel.
    const fn reach(self) -> usize {
        self.r + 1
    }

    /// `1 / sum of the weights`.
    #[allow(clippy::cast_precision_loss)] // `2 r + 1` is a small integer.
    fn inv_weight(self) -> f64 {
        1.0 / (2.0 * self.r as f64 + 1.0 + 2.0 * self.alpha)
    }

    /// The variance of one pass.
    #[cfg(test)]
    #[allow(clippy::cast_precision_loss)] // Small integers.
    fn variance(self) -> f64 {
        let r = self.r as f64;
        let sum = r * (r + 1.0) * (2.0 * r + 1.0) / 3.0 + 2.0 * self.alpha * (r + 1.0) * (r + 1.0);
        sum * self.inv_weight()
    }
}

/// The sampled Gaussian kernel for `-r..=r` with `r = ceil(3 sigma)`,
/// normalized to the sum 1.
// `sigma < EXACT_MAX_SIGMA`, so `r` is a small positive integer.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn gaussian_weights(sigma: f32) -> Vec<f32> {
    let sigma = f64::from(sigma);
    let r = (f64::from(KERNEL_EXTENT) * sigma).ceil().max(1.0) as usize;
    let w: Vec<f64> = (0..=2 * r)
        .map(|i| {
            let x = i as f64 - r as f64;
            (-x * x / (2.0 * sigma * sigma)).exp()
        })
        .collect();
    let sum: f64 = w.iter().sum();
    w.iter().map(|v| (v / sum) as f32).collect()
}

/// Blurs a flat image of `size` pixels with `C` values per pixel. Outside
/// the image, the blur uses the nearest edge value.
fn blur<const C: usize>(src: Vec<f32>, size: (usize, usize), plan: &Plan) -> Vec<f32> {
    let (w, h) = size;
    if w == 0 || h == 0 {
        return src;
    }
    let mut a = src;
    match plan {
        Plan::Exact { weights } => {
            let mut b = vec![0.0; a.len()];
            exact_rows::<C>(&a, &mut b, w, weights);
            exact_columns(&b, &mut a, w * C, h, weights);
        }
        Plan::Boxes(kernel) => boxes_in_place::<C>(&mut a, w, h, *kernel),
    }
    a
}

/// The width (pixels) of the column bands for the vertical box passes.
const BAND: usize = 1024;
/// The number of rows that a horizontal box pass works on at the same
/// time (independent running sums, so that the processor can overlap
/// them).
const ROWS_AT_ONCE: usize = 8;

/// [`BOX_PASSES`] extended box blurs on each axis, in place (see "Kernel"
/// in the [module documentation](self)).
///
/// The result is the box sequence applied to the clamped (infinite) image:
/// each line is extended by `m` (the reach of all passes) values at each
/// end with copies of its edge value. (A clamp in each pass gives a
/// different result near the edges.)
fn boxes_in_place<const C: usize>(img: &mut [f32], w: usize, h: usize, k: ExtendedBox) {
    box_rows::<C>(img, w, h, k);
    for x0 in (0..w).step_by(BAND) {
        box_columns_band::<C>(img, w, (h, x0, BAND.min(w - x0)), k);
    }
}

/// The horizontal box passes, for [`ROWS_AT_ONCE`] rows at a time.
fn box_rows<const C: usize>(img: &mut [f32], w: usize, h: usize, kernel: ExtendedBox) {
    let e = kernel.reach();
    let m = BOX_PASSES * e;
    let n = w + 2 * m;
    let row_len = w * C;
    let line = n * C;
    let mut p = vec![0.0_f32; ROWS_AT_ONCE * line];
    let mut q = vec![0.0_f32; ROWS_AT_ONCE * line];
    for y0 in (0..h).step_by(ROWS_AT_ONCE) {
        let g = ROWS_AT_ONCE.min(h - y0);
        for k in 0..g {
            let row = &img[(y0 + k) * row_len..(y0 + k + 1) * row_len];
            let pad = &mut p[k * line..(k + 1) * line];
            let (left, rest) = pad.split_at_mut(m * C);
            let (mid, right) = rest.split_at_mut(row_len);
            for v in left.chunks_exact_mut(C) {
                v.copy_from_slice(&row[..C]);
            }
            mid.copy_from_slice(row);
            for v in right.chunks_exact_mut(C) {
                v.copy_from_slice(&row[row_len - C..]);
            }
        }
        let (mut lo, mut hi) = (0, n);
        for _ in 0..BOX_PASSES {
            row_pass::<C>(&p, &mut q, (n, g), (lo, hi), kernel);
            lo += e;
            hi -= e;
            std::mem::swap(&mut p, &mut q);
        }
        for k in 0..g {
            let s = k * line + m * C;
            img[(y0 + k) * row_len..(y0 + k + 1) * row_len].copy_from_slice(&p[s..s + row_len]);
        }
    }
}

/// One horizontal extended box pass on `g` padded lines of `n` pixels
/// (`lines = (n, g)`, `src` to `dst`). The input is valid for the pixels
/// `valid.0..valid.1`; the pass writes the pixels `valid.0 + e..valid.1 -
/// e` (`e` = reach). The running sums are in `f64`, so that the rounding
/// error does not grow along long lines.
fn row_pass<const C: usize>(
    src: &[f32],
    dst: &mut [f32],
    lines: (usize, usize),
    valid: (usize, usize),
    kernel: ExtendedBox,
) {
    let (n, g) = lines;
    let (lo, hi) = valid;
    let (r, alpha, inv) = (kernel.r, kernel.alpha, kernel.inv_weight());
    let e = r + 1;
    let line = n * C;
    // The sums of the inner weights (offsets `-r..=r`).
    let mut acc = [[0.0_f64; C]; ROWS_AT_ONCE];
    for (k, a) in acc.iter_mut().enumerate().take(g) {
        let s = &src[k * line..(k + 1) * line];
        for x in lo + 1..=lo + 2 * r + 1 {
            for ch in 0..C {
                a[ch] += f64::from(s[x * C + ch]);
            }
        }
    }
    for x in lo + e..hi - e {
        let more = x + 1 < hi - e;
        for (k, a) in acc.iter_mut().enumerate().take(g) {
            let s = &src[k * line..(k + 1) * line];
            let d = &mut dst[k * line + x * C..k * line + x * C + C];
            let left = &s[(x - e) * C..(x - e + 1) * C];
            let right = &s[(x + e) * C..(x + e + 1) * C];
            for ch in 0..C {
                let v = (a[ch] + alpha * (f64::from(left[ch]) + f64::from(right[ch]))) * inv;
                #[allow(clippy::cast_possible_truncation)] // Mean of f32 values.
                {
                    d[ch] = v as f32;
                }
            }
            if more {
                let add = &s[(x + r + 1) * C..(x + r + 2) * C];
                let sub = &s[(x - r) * C..(x - r + 1) * C];
                for ch in 0..C {
                    a[ch] += f64::from(add[ch]) - f64::from(sub[ch]);
                }
            }
        }
    }
}

/// The vertical box passes on the columns `x0..x0 + bw` (`area = (h, x0,
/// bw)`), as one pipeline over the rows: each pass keeps the last
/// `2 r + 3` lines of its input in a ring, so that the memory stays small
/// and the image is read and written once, row by row.
///
/// Line indices are shifted by `m` (the reach of all passes): input line
/// `t` is the image row `t - m`, clamped. A pass gives its output line `j`
/// from its input lines `j..=j + 2 r + 2` (inner lines `j + 1..=j + 2 r +
/// 1`), when it gets the input line `t = j + 2 r + 2`. The output of the
/// last pass is the image row `j`.
fn box_columns_band<const C: usize>(
    img: &mut [f32],
    w: usize,
    area: (usize, usize, usize),
    kernel: ExtendedBox,
) {
    let (h, x0, bw) = area;
    let (r, alpha, inv) = (kernel.r, kernel.alpha, kernel.inv_weight());
    let m = BOX_PASSES * kernel.reach();
    let row_len = w * C;
    let lane = bw * C;
    let start = x0 * C;
    let cap = 2 * r + 3;
    // `rings[k]` is the input of pass `k`; `accs[k]` its inner sums.
    let mut rings: Vec<Vec<f32>> = vec![vec![0.0; cap * lane]; BOX_PASSES];
    let mut accs: Vec<Vec<f64>> = vec![vec![0.0; lane]; BOX_PASSES];
    let line = |i: usize| (i % cap) * lane..(i % cap) * lane + lane;
    for t in 0..h + 2 * m {
        let sy = t.saturating_sub(m).min(h - 1);
        let s = sy * row_len + start;
        rings[0][line(t)].copy_from_slice(&img[s..s + lane]);
        let (mut k, mut ti) = (0, t);
        loop {
            let ring = &rings[k];
            let acc = &mut accs[k][..lane];
            if ti <= 2 * r + 1 {
                if ti >= 1 {
                    for (a, &v) in acc.iter_mut().zip(&ring[line(ti)]) {
                        *a += f64::from(v);
                    }
                }
                break;
            }
            let j = ti - 2 * r - 2;
            if j > 0 {
                // Move the inner window from `j..=j + 2 r` to
                // `j + 1..=j + 2 r + 1`.
                for ((a, &p), &o) in acc.iter_mut().zip(&ring[line(ti - 1)]).zip(&ring[line(j)]) {
                    *a += f64::from(p) - f64::from(o);
                }
            }
            let (outer_a, outer_b) = (line(j), line(ti));
            // The ring of the next pass, or the image row for the last pass.
            let (out, ring, acc) = if k + 1 == BOX_PASSES {
                let s = j * row_len + start;
                (&mut img[s..s + lane], &rings[k], &accs[k])
            } else {
                let (head, tail) = rings.split_at_mut(k + 1);
                (&mut tail[0][line(j)], &head[k], &accs[k])
            };
            let (oa, ob) = (&ring[outer_a], &ring[outer_b]);
            for (((o, &a), &x), &y) in out.iter_mut().zip(acc.iter()).zip(oa).zip(ob) {
                let v = (a + alpha * (f64::from(x) + f64::from(y))) * inv;
                #[allow(clippy::cast_possible_truncation)] // Mean of f32 values.
                {
                    *o = v as f32;
                }
            }
            if k + 1 == BOX_PASSES {
                break;
            }
            k += 1;
            ti = j;
        }
    }
}

/// Horizontal pass with the sampled kernel (`src` to `dst`).
fn exact_rows<const C: usize>(src: &[f32], dst: &mut [f32], w: usize, weights: &[f32]) {
    let r = weights.len() / 2;
    let row_len = w * C;
    let mut pad = vec![0.0_f32; (w + 2 * r) * C];
    for (row, out) in src.chunks_exact(row_len).zip(dst.chunks_exact_mut(row_len)) {
        let (left, rest) = pad.split_at_mut(r * C);
        let (mid, right) = rest.split_at_mut(row_len);
        for p in left.chunks_exact_mut(C) {
            p.copy_from_slice(&row[..C]);
        }
        mid.copy_from_slice(row);
        for p in right.chunks_exact_mut(C) {
            p.copy_from_slice(&row[row_len - C..]);
        }
        out.fill(0.0);
        for (k, &wk) in weights.iter().enumerate() {
            let p = &pad[k * C..k * C + row_len];
            for (o, v) in out.iter_mut().zip(p) {
                *o += wk * v;
            }
        }
    }
}

/// Vertical pass with the sampled kernel (`src` to `dst`).
fn exact_columns(src: &[f32], dst: &mut [f32], row_len: usize, h: usize, weights: &[f32]) {
    let r = weights.len() / 2;
    for (y, out) in dst.chunks_exact_mut(row_len).enumerate() {
        out.fill(0.0);
        for (k, &wk) in weights.iter().enumerate() {
            let sy = (y + k).saturating_sub(r).min(h - 1);
            let row = &src[sy * row_len..(sy + 1) * row_len];
            for (o, v) in out.iter_mut().zip(row) {
                *o += wk * v;
            }
        }
    }
}
