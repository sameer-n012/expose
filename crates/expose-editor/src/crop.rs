//! Canvas crop with straighten (ADR 0032).
//!
//! [`Document::crop_canvas`] gives the document a new canvas size and
//! moves (or turns) all layers and masks, so that the content in the crop
//! box fills the new canvas. The result is one reversible [`Edit`].
//!
//! # Mapping
//!
//! The crop box has its center at [`CropSpec::center`] (old canvas pixels)
//! and the new canvas size. The box turns by [`CropSpec::angle`] degrees
//! clockwise on the screen (the y axis points down). The center of the new
//! pixel `(u, v)` samples the old canvas point
//!
//! ```text
//! center + R(angle) · (u + 0.5 − width / 2, v + 0.5 − height / 2)
//! ```
//!
//! where `R` is [`Affine::rotate`]. Thus the content turns by `−angle`, and
//! the box becomes level. [`CropSpec::new_to_old`] gives this map as an
//! [`Affine`]. The crop box can go outside the old canvas: the new area is
//! transparent (for a mask: the [`MaskFill`](crate::MaskFill) value).
//!
//! # Two paths
//!
//! - **Move only.** If the angle is 0 and `center − size / 2` is a whole
//!   number on each axis, the function changes only the layer offsets and
//!   the mask offsets. The values stay the same, bit for bit.
//! - **Resample.** Otherwise, the function resamples each pixel layer and
//!   each mask (also the masks of adjustment layers and groups) with the
//!   resampler of [`crate::transform`] and [`CropSpec::interpolation`]. A
//!   pixel layer with no pixel with alpha > 0 is not resampled: it only
//!   gets a new offset (the mapped position of its center, rounded).
//!
//! # Hidden or deleted pixels
//!
//! With [`CropSpec::delete_cropped`] `false`, the layers keep the pixels
//! outside the new canvas (hidden). A later move can show them again. With
//! `true`, the function trims each pixel buffer and each mask buffer to the
//! new canvas. A buffer with no part on the new canvas becomes the smallest
//! buffer (1 × 1 at `(0, 0)`): transparent for a pixel layer, the fill value
//! for a mask. A mask outside its buffer has its fill value, so the mask
//! keeps its meaning on the canvas.
//!
//! # Locks, selection, and versions
//!
//! A canvas operation ignores the layer locks (it changes the document, not
//! one layer). The function removes the selection and the selection that
//! [`Document::reselect`] can restore. The document gets a new size, a new
//! revision, and a new selection version. A layer gets a new pixel version
//! (or mask version) only if its values change (resample or trim). An
//! offset change keeps the versions, as a move does: caches must compare
//! the offsets too (the GPU compositor does).
//!
//! The undo edit restores the old size, the old selection, and each changed
//! layer. For a layer that only moves, it holds only the offsets.

use expose_core::buffer::{Rect, WorkingImage};

use crate::document::{Document, check_size};
use crate::error::{EditorError, EditorResult};
use crate::layer::{LayerId, LayerKind, Mask, MaskImage, PixelLayer};
use crate::ops::Edit;
use crate::selection::StoredSelection;
use crate::transform::{
    Affine, Filter, Interpolation, Target, content_rect, resample_image, resample_mask,
    rounded_bounds, target_from_bounds,
};
use crate::version::next_stamp;

#[cfg(test)]
mod tests;

/// The largest absolute crop angle in degrees.
pub const MAX_CROP_ANGLE: f64 = 45.0;

/// The largest absolute value of a crop center coordinate, in old canvas
/// pixels.
pub const MAX_CROP_COORD: f64 = 1_000_000.0;

/// The settings of a canvas crop (see the module documentation).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropSpec {
    /// Center of the crop box in old canvas pixels.
    pub center: (f64, f64),
    /// New canvas width (document size limits).
    pub width: u32,
    /// New canvas height (document size limits).
    pub height: u32,
    /// Degrees, `-MAX_CROP_ANGLE..=MAX_CROP_ANGLE`. A positive angle turns
    /// the box clockwise on the screen. The content turns by `-angle`, so
    /// that the box becomes level.
    pub angle: f64,
    /// `true`: remove the pixels outside the new canvas. `false`: keep them
    /// in the layer buffers outside the canvas (hidden).
    pub delete_cropped: bool,
    /// The interpolation of the resample path.
    pub interpolation: Interpolation,
}

impl CropSpec {
    /// Checks the values.
    ///
    /// Errors: [`EditorError::NonFinite`] or [`EditorError::OutOfRange`]
    /// for the center or the angle, and the errors of the document size
    /// check for the new size (a size of 0 or a size larger than the
    /// limits).
    pub fn validate(&self) -> EditorResult<()> {
        for v in [self.center.0, self.center.1] {
            check_f64("crop center", v, -MAX_CROP_COORD, MAX_CROP_COORD)?;
        }
        check_f64("crop angle", self.angle, -MAX_CROP_ANGLE, MAX_CROP_ANGLE)?;
        check_size(self.width, self.height)?;
        Ok(())
    }

    /// Returns the map from new canvas points to old canvas points (see the
    /// module documentation).
    pub fn new_to_old(&self) -> Affine {
        let (hw, hh) = (f64::from(self.width) / 2.0, f64::from(self.height) / 2.0);
        Affine::translate(-hw, -hh)
            .then(Affine::rotate(self.angle.to_radians()))
            .then(Affine::translate(self.center.0, self.center.1))
    }
}

/// Checks that `value` is finite and in `min..=max`.
fn check_f64(name: &'static str, value: f64, min: f64, max: f64) -> EditorResult<()> {
    if !value.is_finite() {
        return Err(EditorError::NonFinite { name });
    }
    if value < min || value > max {
        return Err(EditorError::OutOfRange {
            name,
            value,
            min,
            max,
        });
    }
    Ok(())
}

// -------------------------------------------------------------------
// The edit data.
// -------------------------------------------------------------------

/// The new offset of a buffer, and the new buffer if it changes (`None`
/// keeps the buffer).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BufferChange<T> {
    offset: (i32, i32),
    image: Option<T>,
}

/// The change of one layer: its pixels (pixel layers only) and its mask.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LayerChange {
    id: LayerId,
    pixels: Option<BufferChange<WorkingImage>>,
    mask: Option<BufferChange<MaskImage>>,
}

/// A canvas change: the canvas size to set, the selection to set (compact
/// form, `None` = no selection), and the layer changes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CanvasChange {
    width: u32,
    height: u32,
    selection: Option<StoredSelection>,
    layers: Vec<LayerChange>,
}

impl CanvasChange {
    /// Returns the approximate heap memory in bytes.
    pub(crate) fn memory_bytes(&self) -> usize {
        let selection = self
            .selection
            .as_ref()
            .map_or(0, StoredSelection::memory_bytes);
        let layers: usize = self
            .layers
            .iter()
            .map(|c| {
                let pixels = c
                    .pixels
                    .as_ref()
                    .and_then(|b| b.image.as_ref())
                    .map_or(0, |i| i.as_bytes().len());
                let mask = c
                    .mask
                    .as_ref()
                    .and_then(|b| b.image.as_ref())
                    .map_or(0, |m| size_of_val(m.data()));
                size_of::<LayerChange>() + pixels + mask
            })
            .sum();
        selection + layers
    }
}

// -------------------------------------------------------------------
// Buffer helpers.
// -------------------------------------------------------------------

/// Returns the part of a buffer of size `size` at canvas position `offset`
/// that is on the canvas `(0, 0, w, h)`, in buffer coordinates. `None` means
/// no part is on the canvas.
fn canvas_part(offset: (i32, i32), size: (u32, u32), w: u32, h: u32) -> Option<Rect> {
    let (ox, oy) = (i64::from(offset.0), i64::from(offset.1));
    let x0 = ox.max(0);
    let y0 = oy.max(0);
    let x1 = (ox + i64::from(size.0)).min(i64::from(w));
    let y1 = (oy + i64::from(size.1)).min(i64::from(h));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    // All values are in `0..=u32::MAX`: `x0 - ox` is less than the buffer
    // width, and `x1 - x0` is at most the canvas width.
    let to_u32 = |v: i64| u32::try_from(v).unwrap_or(u32::MAX);
    Some(Rect::new(
        to_u32(x0 - ox),
        to_u32(y0 - oy),
        to_u32(x1 - x0),
        to_u32(y1 - y0),
    ))
}

/// Copies the values of `rect` (inside the mask) into a new mask image.
fn crop_mask(image: &MaskImage, rect: Rect) -> EditorResult<MaskImage> {
    let w = image.width() as usize;
    let (x0, rw) = (rect.x as usize, rect.width as usize);
    let mut data = Vec::with_capacity(rw * rect.height as usize);
    for row in image
        .data()
        .chunks_exact(w)
        .skip(rect.y as usize)
        .take(rect.height as usize)
    {
        data.extend_from_slice(&row[x0..x0 + rw]);
    }
    MaskImage::from_data(rect.width, rect.height, data)
}

/// Adds `(dx, dy)` to `offset`, or returns [`EditorError::OffsetOverflow`].
fn shifted(offset: (i32, i32), d: (i64, i64)) -> EditorResult<(i32, i32)> {
    let x = i32::try_from(i64::from(offset.0) + d.0).map_err(|_| EditorError::OffsetOverflow)?;
    let y = i32::try_from(i64::from(offset.1) + d.1).map_err(|_| EditorError::OffsetOverflow)?;
    Ok((x, y))
}

/// Converts a whole number to `i64`, or returns
/// [`EditorError::OffsetOverflow`].
fn whole_i64(v: f64) -> EditorResult<i64> {
    // The limit is far inside the `i64` range, so the cast is exact for a
    // whole number in the range.
    let limit = f64::from(u32::MAX);
    if !v.is_finite() || v.abs() > limit {
        return Err(EditorError::OffsetOverflow);
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(v.round() as i64)
}

/// The kind of a buffer: how to trim it and how to make an empty one.
trait Buffer: Sized {
    fn size(&self) -> (u32, u32);
    fn crop(&self, rect: Rect) -> EditorResult<Self>;
}

impl Buffer for WorkingImage {
    fn size(&self) -> (u32, u32) {
        (self.width(), self.height())
    }
    fn crop(&self, rect: Rect) -> EditorResult<Self> {
        Ok(Self::crop(self, rect)?)
    }
}

impl Buffer for MaskImage {
    fn size(&self) -> (u32, u32) {
        (self.width(), self.height())
    }
    fn crop(&self, rect: Rect) -> EditorResult<Self> {
        crop_mask(self, rect)
    }
}

/// The settings of one crop, for all layers.
struct Ctx {
    /// Old canvas → new canvas.
    m: Affine,
    /// New canvas → old canvas.
    inv: Affine,
    filter: Filter,
    /// The whole-pixel shift of the move-only path, or `None` for the
    /// resample path.
    shift: Option<(i64, i64)>,
    delete: bool,
    width: u32,
    height: u32,
}

impl Ctx {
    /// Moves a buffer by `d` and trims it if the crop deletes pixels.
    /// `empty` makes the 1 × 1 buffer for a buffer with no part on the
    /// canvas. Returns `None` if nothing changes.
    fn shift<T: Buffer>(
        &self,
        image: &T,
        offset: (i32, i32),
        d: (i64, i64),
        empty: impl FnOnce() -> EditorResult<T>,
    ) -> EditorResult<Option<BufferChange<T>>> {
        let new_offset = shifted(offset, d)?;
        let size = image.size();
        if self.delete {
            match canvas_part(new_offset, size, self.width, self.height) {
                None => {
                    return Ok(Some(BufferChange {
                        offset: (0, 0),
                        image: Some(empty()?),
                    }));
                }
                Some(r) if (r.width, r.height) != size => {
                    let at = shifted(new_offset, (i64::from(r.x), i64::from(r.y)))?;
                    return Ok(Some(BufferChange {
                        offset: at,
                        image: Some(image.crop(r)?),
                    }));
                }
                Some(_) => {}
            }
        }
        Ok((new_offset != offset).then_some(BufferChange {
            offset: new_offset,
            image: None,
        }))
    }

    /// Returns the destination area for the mapped bounds of a buffer, or
    /// `None` if the crop deletes pixels and no part is on the canvas.
    fn place(&self, bounds: [f64; 4]) -> EditorResult<Option<Target>> {
        // `target_from_bounds` refuses values that are not finite.
        if !self.delete || !bounds.iter().all(|v| v.is_finite()) {
            return target_from_bounds(bounds).map(Some);
        }
        let [x0, y0, x1, y1] = bounds;
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        let (x0, y0, x1, y1) = (x0.max(0.0), y0.max(0.0), x1.min(w), y1.min(h));
        if x1 > x0 && y1 > y0 {
            target_from_bounds([x0, y0, x1, y1]).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Returns the whole-pixel shift for a buffer that is not resampled: the
    /// move-only shift, or the rounded shift of the buffer center.
    fn buffer_shift(&self, offset: (i32, i32), size: (u32, u32)) -> EditorResult<(i64, i64)> {
        if let Some(d) = self.shift {
            return Ok(d);
        }
        let (hw, hh) = (f64::from(size.0) / 2.0, f64::from(size.1) / 2.0);
        let (ox, oy) = (f64::from(offset.0), f64::from(offset.1));
        let (cx, cy) = self.m.apply(ox + hw, oy + hh);
        Ok((
            whole_i64((cx - hw - ox).round())?,
            whole_i64((cy - hh - oy).round())?,
        ))
    }

    /// Returns the change of a pixel layer, or `None` if it does not
    /// change.
    fn pixels(&self, p: &PixelLayer) -> EditorResult<Option<BufferChange<WorkingImage>>> {
        let image = p.image();
        let offset = p.offset();
        let empty = || Ok(WorkingImage::new_filled(1, 1, [0.0; 4])?);
        let content = match self.shift {
            Some(d) => return self.shift(image, offset, d, empty),
            None => content_rect(image),
        };
        let Some(c) = content else {
            // No visible pixel: nothing to resample.
            let d = self.buffer_shift(offset, (image.width(), image.height()))?;
            return self.shift(image, offset, d, empty);
        };
        let bounds = rounded_bounds(
            &self.m,
            f64::from(offset.0) + f64::from(c.x),
            f64::from(offset.1) + f64::from(c.y),
            f64::from(c.width),
            f64::from(c.height),
        );
        let Some(t) = self.place(bounds)? else {
            return Ok(Some(BufferChange {
                offset: (0, 0),
                image: Some(empty()?),
            }));
        };
        let new = resample_image(image, offset, &self.inv, t, self.filter)?;
        Ok(Some(BufferChange {
            offset: (t.x, t.y),
            image: Some(new),
        }))
    }

    /// Returns the change of a mask, or `None` if it does not change.
    fn mask(&self, mask: &Mask) -> EditorResult<Option<BufferChange<MaskImage>>> {
        let fill = mask.fill.value();
        let empty = || MaskImage::new_filled(1, 1, fill);
        if let Some(d) = self.shift {
            return self.shift(&mask.image, mask.offset, d, empty);
        }
        let bounds = rounded_bounds(
            &self.m,
            f64::from(mask.offset.0),
            f64::from(mask.offset.1),
            f64::from(mask.image.width()),
            f64::from(mask.image.height()),
        );
        let Some(t) = self.place(bounds)? else {
            return Ok(Some(BufferChange {
                offset: (0, 0),
                image: Some(empty()?),
            }));
        };
        let new = resample_mask(mask, &self.inv, t, self.filter)?;
        Ok(Some(BufferChange {
            offset: (t.x, t.y),
            image: Some(new),
        }))
    }
}

impl Document {
    /// Crops the canvas (see the module documentation).
    ///
    /// Errors:
    /// - [`EditorError::NonFinite`] / [`EditorError::OutOfRange`]: a center
    ///   or angle value that is not finite or is out of range.
    /// - [`EditorError::Core`] / [`EditorError::TooLarge`]: a new size of 0
    ///   or larger than the document size limits.
    /// - [`EditorError::TooLarge`] / [`EditorError::OffsetOverflow`]: with
    ///   `delete_cropped` `false`, a turned layer or mask that is larger
    ///   than the size limits or outside the offset range.
    ///
    /// If the function returns an error, the document did not change.
    pub fn crop_canvas(&mut self, spec: &CropSpec) -> EditorResult<Edit> {
        spec.validate()?;
        let inv = spec.new_to_old();
        // A rotation and a translation always have an inverse.
        let m = inv
            .inverse()
            .ok_or(EditorError::InvalidTransform("crop map has no inverse"))?;
        let shift = match m.whole_translation() {
            Some((dx, dy)) => Some((whole_i64(dx)?, whole_i64(dy)?)),
            None => None,
        };
        let ctx = Ctx {
            m,
            inv,
            filter: Filter::new(&m, spec.interpolation),
            shift,
            delete: spec.delete_cropped,
            width: spec.width,
            height: spec.height,
        };

        let mut ids = Vec::new();
        for l in &self.layers {
            l.visit(&mut |x| ids.push(x.id()));
        }
        let mut layers = Vec::new();
        for id in ids {
            let layer = self.layer(id).ok_or(EditorError::LayerNotFound(id))?;
            let pixels = match layer.as_pixel() {
                Some(p) => ctx.pixels(p)?,
                None => None,
            };
            let mask = match layer.mask() {
                Some(mk) => ctx.mask(mk)?,
                None => None,
            };
            if pixels.is_some() || mask.is_some() {
                layers.push(LayerChange { id, pixels, mask });
            }
        }
        let undo = self.apply_edit(Edit::canvas(CanvasChange {
            width: spec.width,
            height: spec.height,
            selection: None,
            layers,
        }))?;
        // The stored selection has the old canvas size.
        self.last_selection = None;
        Ok(undo)
    }

    /// Applies a canvas change and returns its inverse. Checks first, so
    /// that an error changes nothing.
    ///
    /// Each layer whose buffer changes gets a new pixel (or mask) version
    /// and a full change. Offset changes keep the versions. The selection
    /// gets a new version.
    pub(crate) fn raw_canvas(&mut self, change: CanvasChange) -> EditorResult<CanvasChange> {
        let CanvasChange {
            width,
            height,
            selection,
            mut layers,
        } = change;
        check_size(width, height)?;
        let new_selection = match &selection {
            Some(s) => Some(s.expand(width, height)?),
            None => None,
        };
        for c in &layers {
            let l = self.layer(c.id).ok_or(EditorError::LayerNotFound(c.id))?;
            if c.pixels.is_some() && l.as_pixel().is_none() {
                return Err(EditorError::NotAPixelLayer(c.id));
            }
            if c.mask.is_some() && l.mask().is_none() {
                return Err(EditorError::NoMask(c.id));
            }
        }

        // The loop above checked that each layer exists and has the
        // buffers that the change sets, so nothing below fails.
        for c in &mut layers {
            let layer = self.layer_mut(c.id)?;
            if let Some(b) = &mut c.pixels {
                let mut values = false;
                if let LayerKind::Pixel(p) = layer.kind_mut() {
                    let old = p.offset();
                    p.set_offset(b.offset);
                    b.offset = old;
                    if let Some(image) = &mut b.image {
                        std::mem::swap(p.image_mut(), image);
                        values = true;
                    }
                }
                if values {
                    layer.touch_pixels();
                }
            }
            if let Some(b) = &mut c.mask {
                let mut values = false;
                if let Some(m) = layer.mask_mut() {
                    std::mem::swap(&mut m.offset, &mut b.offset);
                    if let Some(image) = &mut b.image {
                        std::mem::swap(&mut m.image, image);
                        values = true;
                    }
                }
                if values {
                    layer.touch_mask();
                }
            }
        }
        let (old_width, old_height) = (self.width(), self.height());
        self.set_size(width, height);
        let old_selection = std::mem::replace(&mut self.selection, new_selection);
        self.selection_version = next_stamp();
        Ok(CanvasChange {
            width: old_width,
            height: old_height,
            selection: old_selection.as_ref().map(StoredSelection::compact),
            layers,
        })
    }
}
