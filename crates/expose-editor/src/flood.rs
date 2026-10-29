//! Flood fill: the region core of the magic wand and the paint bucket.
//!
//! # Use
//!
//! 1. Make a [`FloodSample`] from the pixels that the tool compares: the
//!    composite of all visible layers ([`FloodSample::composite`] or
//!    [`FloodSample::from_composite_image`]), or one pixel layer
//!    ([`FloodSample::layer`]).
//! 2. [`flood_region`] returns a canvas-size region of the pixels that are
//!    similar to the seed pixel.
//! 3. The magic wand gives the region to [`Document::select_region`]. The
//!    paint bucket gives it to [`Document::fill_region`].
//!
//! # Key values
//!
//! The sample keeps one key value with four channels for each canvas
//! pixel. For a premultiplied pixel `(r, g, b, a)` in linear Rec. 2020:
//!
//! ```text
//! a'  = clamp(a, 0, 1)
//! C   = (r, g, b) / a                  (straight color; a <= 0 gives key 0)
//! S   = clamp(M * C, 0, 1)             (M: linear Rec. 2020 -> linear sRGB)
//! key = round(255 * (enc(S.r) * a', enc(S.g) * a', enc(S.b) * a', a'))
//! ```
//!
//! `enc` is the sRGB transfer function. The sample uses a table with
//! 65536 entries (nearest entry) in place of a `powf` call for each value.
//! The table error is less than 0.03 of one key step.
//!
//! The keys are integers (`u8`). This keeps the sample small (4 bytes for
//! each pixel) and makes the compare exact: tolerance 0 matches only equal
//! keys. Two pixels with a very small color difference can have the same
//! key.
//!
//! A pixel matches the seed if `max_i |key_i - seed_i| <= tolerance`. All
//! fully transparent pixels have the key `(0, 0, 0, 0)`, so they match each
//! other.
//!
//! # Region
//!
//! - Contiguous: a 4-connected scanline flood fill from the seed (pixels
//!   that touch only at a corner are not connected). The fill uses an
//!   explicit stack, not recursion.
//! - Not contiguous: all matching pixels of the canvas.
//!
//! A pixel in the region has the value 1. Without anti-alias, all other
//! pixels have 0. With anti-alias, a pixel outside the region gets the
//! mean of the region values (0 or 1) in its 3 x 3 neighborhood, where
//! pixels outside the canvas count as 0. Thus only the 1 px ring around the
//! region gets values between 0 and 1.
//!
//! # Fill
//!
//! [`Document::fill_region`] paints a color on a pixel layer with the
//! coverage `c = region * selection * opacity` (no selection counts as 1).
//! It uses the same paint rule as a brush stroke with the coverage `c`
//! (see "Accumulation" and "Blend mode" in [`crate::brush`]), also the rule
//! for the transparency lock. Only the pixels inside the layer bounds
//! change: the layer does not grow.

use expose_core::buffer::{Rect, Rgba, WorkingImage};
use expose_core::color::{ColorSpaceId, TransferFunction, mul_vec_f32};

use crate::blend::BlendMode;
use crate::brush::{Mode, STROKE_TILE_SIZE, Texel};
use crate::composite::composite;
use crate::document::Document;
use crate::error::{EditorError, EditorResult, check_range};
use crate::layer::{LayerId, LayerKind, LockKind, MaskImage};
use crate::ops::Edit;
use crate::selection::SelectionMode;

#[cfg(test)]
mod tests;

/// Largest flood tolerance (in key steps, see the module documentation).
pub const MAX_FLOOD_TOLERANCE: f32 = 255.0;

/// Number of entries of the encode table.
const ENCODE_TABLE_SIZE: usize = 65536;
/// `ENCODE_TABLE_SIZE - 1` as `f32`: entry `i` holds the encoded value of
/// `i / ENCODE_TABLE_SCALE`.
const ENCODE_TABLE_SCALE: f32 = 65535.0;

/// The settings of a flood region (magic wand, paint bucket).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloodSettings {
    /// The largest key difference that matches,
    /// `0..=`[`MAX_FLOOD_TOLERANCE`]. The keys are integers, so only the
    /// integer part has an effect.
    pub tolerance: f32,
    /// If `true`, only pixels that connect to the seed (4-connected) go
    /// into the region. If `false`, all matching pixels go into it.
    pub contiguous: bool,
    /// If `true`, the 1 px ring around the region gets partial values.
    pub anti_alias: bool,
}

impl Default for FloodSettings {
    /// Tolerance 32, contiguous, anti-alias.
    fn default() -> Self {
        Self {
            tolerance: 32.0,
            contiguous: true,
            anti_alias: true,
        }
    }
}

impl FloodSettings {
    /// Returns an error if the tolerance is not finite or is outside
    /// `0..=`[`MAX_FLOOD_TOLERANCE`].
    pub fn validate(&self) -> EditorResult<()> {
        check_range("flood tolerance", self.tolerance, 0.0, MAX_FLOOD_TOLERANCE)
    }
}

/// The settings of a paint bucket fill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FillSettings {
    /// The fill color: straight (not premultiplied) linear Rec. 2020.
    /// Values must be finite.
    pub color: [f32; 3],
    /// `0.0..=1.0`. The maximum coverage of the fill.
    pub opacity: f32,
    /// How the fill color mixes with the pixels below it (the same rule as
    /// a brush, see [`crate::brush`]). [`BlendMode::Dissolve`] is not
    /// available.
    pub blend_mode: BlendMode,
}

impl FillSettings {
    /// Returns an error if the color is not finite
    /// ([`EditorError::NonFinite`]), the opacity is out of range, or the
    /// blend mode is [`BlendMode::Dissolve`] ([`EditorError::InvalidFill`]).
    pub fn validate(&self) -> EditorResult<()> {
        if self.blend_mode == BlendMode::Dissolve {
            // The dissolve pattern changes alpha per pixel position. The
            // coverage rule of the paint rule has no place for it.
            return Err(EditorError::InvalidFill(
                "the dissolve blend mode is not available for a fill",
            ));
        }
        if self.color.iter().any(|v| !v.is_finite()) {
            return Err(EditorError::NonFinite { name: "fill color" });
        }
        check_range("fill opacity", self.opacity, 0.0, 1.0)
    }
}

/// Converts premultiplied working-space pixels to key values.
struct KeyEncoder {
    /// Linear Rec. 2020 to linear sRGB.
    matrix: [[f32; 3]; 3],
    /// `enc(i / ENCODE_TABLE_SCALE)` for each index `i`.
    table: Vec<f32>,
}

impl KeyEncoder {
    fn new() -> EditorResult<Self> {
        let matrix = ColorSpaceId::Srgb.from_working()?.to_f32();
        let table = (0..=u16::MAX)
            .map(|i| TransferFunction::Srgb.encode(f32::from(i) / ENCODE_TABLE_SCALE))
            .collect();
        Ok(Self { matrix, table })
    }

    /// The encoded value of a linear value (clamped to `0..=1`).
    #[inline]
    // The index is clamped to `0..=65535.5` before the cast, so it is in
    // the table. A NaN input gives index 0 (saturating cast).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn encode(&self, v: f32) -> f32 {
        let i = (v.clamp(0.0, 1.0) * ENCODE_TABLE_SCALE + 0.5) as usize;
        self.table[i.min(ENCODE_TABLE_SIZE - 1)]
    }

    /// The key of a premultiplied pixel.
    #[inline]
    // All values are in `0..=255.5` before the casts.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn key(&self, p: Rgba) -> [u8; 4] {
        // Transparent pixels (and a NaN alpha) give the key 0.
        if p[3].is_nan() || p[3] <= 0.0 {
            return [0; 4];
        }
        let inv = 1.0 / p[3];
        let s = mul_vec_f32(&self.matrix, [p[0] * inv, p[1] * inv, p[2] * inv]);
        let k = 255.0 * p[3].min(1.0);
        [
            (self.encode(s[0]) * k + 0.5) as u8,
            (self.encode(s[1]) * k + 0.5) as u8,
            (self.encode(s[2]) * k + 0.5) as u8,
            (k + 0.5) as u8,
        ]
    }

    /// The keys of a row of premultiplied pixels.
    fn keys(&self, src: &[Rgba], dst: &mut [[u8; 4]]) {
        // Flat areas (common in layered documents) repeat the same pixel:
        // reuse the last key for a pixel with the same bits.
        let mut last: Option<(Rgba, [u8; 4])> = None;
        for (k, &p) in dst.iter_mut().zip(src) {
            *k = match last {
                Some((q, key)) if q.same_bits(&p) => key,
                _ => {
                    let key = self.key(p);
                    last = Some((p, key));
                    key
                }
            };
        }
    }
}

/// The canvas-size key values that a flood region compares (see the
/// [module documentation](self)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloodSample {
    width: u32,
    height: u32,
    keys: Vec<[u8; 4]>,
}

impl FloodSample {
    /// Makes a sample from the CPU composite of all visible layers over the
    /// full canvas ([`composite`]).
    pub fn composite(doc: &Document) -> EditorResult<Self> {
        let image = composite(doc, doc.bounds())?;
        Self::from_composite_image(doc, &image)
    }

    /// Makes a sample from a canvas-size premultiplied image (for example
    /// [`GpuCompositor::composite_gpu_image`](crate::GpuCompositor::composite_gpu_image)
    /// with the full canvas as the region).
    ///
    /// Returns [`EditorError::InvalidRect`] (with the image size) if the
    /// image size is not the canvas size.
    pub fn from_composite_image(doc: &Document, image: &WorkingImage) -> EditorResult<Self> {
        check_canvas_size(doc, image.width(), image.height())?;
        let enc = KeyEncoder::new()?;
        let pixels = image.pixels();
        let mut keys = vec![[0u8; 4]; pixels.len()];
        enc.keys(pixels, &mut keys);
        Ok(Self {
            width: image.width(),
            height: image.height(),
            keys,
        })
    }

    /// Makes a sample from the pixels of one pixel layer. The function
    /// uses the pixel values only (not the mask, opacity, visibility, or
    /// blend mode). Canvas pixels outside the layer bounds are
    /// transparent.
    ///
    /// Errors: [`EditorError::LayerNotFound`],
    /// [`EditorError::NotAPixelLayer`].
    pub fn layer(doc: &Document, id: LayerId) -> EditorResult<Self> {
        let l = doc.layer(id).ok_or(EditorError::LayerNotFound(id))?;
        let p = l.as_pixel().ok_or(EditorError::NotAPixelLayer(id))?;
        let (cw, ch) = (doc.width(), doc.height());
        // The canvas size is below `MAX_PIXELS`, so the count fits in usize.
        let mut keys = vec![[0u8; 4]; cw as usize * ch as usize];
        let img = p.image();
        if let Some(o) = Overlap::new(p.offset(), (img.width(), img.height()), (cw, ch)) {
            let enc = KeyEncoder::new()?;
            let pixels = img.pixels();
            let lw = img.width() as usize;
            for row in 0..o.height {
                let src = (o.ly + row) * lw + o.lx;
                let dst = (o.cy + row) * cw as usize + o.cx;
                enc.keys(&pixels[src..src + o.width], &mut keys[dst..dst + o.width]);
            }
        }
        Ok(Self {
            width: cw,
            height: ch,
            keys,
        })
    }

    /// Returns the width in pixels (the canvas width).
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the height in pixels (the canvas height).
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the key of a pixel (for tests).
    #[cfg(test)]
    fn key(&self, x: u32, y: u32) -> [u8; 4] {
        self.keys[y as usize * self.width as usize + x as usize]
    }
}

/// The part of a layer buffer that is inside the canvas. All values are
/// buffer indices (not negative).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Overlap {
    /// Left edge in layer coordinates.
    lx: usize,
    /// Top edge in layer coordinates.
    ly: usize,
    /// Left edge in canvas coordinates.
    cx: usize,
    /// Top edge in canvas coordinates.
    cy: usize,
    width: usize,
    height: usize,
}

impl Overlap {
    /// Returns `None` if the layer and the canvas do not overlap.
    fn new(offset: (i32, i32), layer: (u32, u32), canvas: (u32, u32)) -> Option<Self> {
        let axis = |o: i32, l: u32, c: u32| -> Option<(usize, usize, usize)> {
            let o = i64::from(o);
            let c0 = o.max(0);
            let c1 = (o + i64::from(l)).min(i64::from(c));
            if c1 <= c0 {
                return None;
            }
            // `0 <= c0 < c1 <= canvas size`, and `0 <= c0 - o < l`, so all
            // values fit in usize.
            Some((
                usize::try_from(c0 - o).ok()?,
                usize::try_from(c0).ok()?,
                usize::try_from(c1 - c0).ok()?,
            ))
        };
        let (lx, cx, width) = axis(offset.0, layer.0, canvas.0)?;
        let (ly, cy, height) = axis(offset.1, layer.1, canvas.1)?;
        Some(Self {
            lx,
            ly,
            cx,
            cy,
            width,
            height,
        })
    }
}

/// Returns [`EditorError::InvalidRect`] if `width x height` is not the
/// canvas size.
fn check_canvas_size(doc: &Document, width: u32, height: u32) -> EditorResult<()> {
    if (width, height) == (doc.width(), doc.height()) {
        Ok(())
    } else {
        Err(EditorError::InvalidRect {
            x: 0,
            y: 0,
            width,
            height,
        })
    }
}

/// Checks that a region has the canvas size and finite values.
fn check_region(doc: &Document, region: &MaskImage) -> EditorResult<()> {
    check_canvas_size(doc, region.width(), region.height())?;
    if region.data().iter().any(|v| !v.is_finite()) {
        return Err(EditorError::NonFinite {
            name: "region value",
        });
    }
    Ok(())
}

/// A bounding box in canvas pixels (`x0..x1`, `y0..y1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Bounds {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

impl Bounds {
    /// Adds the span `x0..x1` in row `y`.
    fn add_span(this: &mut Option<Self>, x0: usize, x1: usize, y: usize) {
        match this {
            None => {
                *this = Some(Self {
                    x0,
                    y0: y,
                    x1,
                    y1: y + 1,
                });
            }
            Some(b) => {
                b.x0 = b.x0.min(x0);
                b.x1 = b.x1.max(x1);
                b.y0 = b.y0.min(y);
                b.y1 = b.y1.max(y + 1);
            }
        }
    }
}

/// Returns the region of the pixels that are similar to the seed pixel
/// `(x, y)` (canvas pixels). See the [module documentation](self) for the
/// rules. The result has the canvas size.
///
/// Errors: bad settings, or [`EditorError::OutOfRange`] if the seed is
/// outside the canvas.
pub fn flood_region(
    sample: &FloodSample,
    x: u32,
    y: u32,
    settings: &FloodSettings,
) -> EditorResult<MaskImage> {
    settings.validate()?;
    let (w, h) = (sample.width, sample.height);
    for (name, v, size) in [("seed x", x, w), ("seed y", y, h)] {
        if v >= size {
            return Err(EditorError::OutOfRange {
                name,
                value: f64::from(v),
                min: 0.0,
                max: f64::from(size) - 1.0,
            });
        }
    }
    let (wu, hu) = (w as usize, h as usize);
    let keys = &sample.keys;
    let seed = keys[y as usize * wu + x as usize];
    // The keys are integers, so `diff <= tolerance` is the same as
    // `diff <= floor(tolerance)`. The tolerance is in `0..=255`.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let tol = settings.tolerance.floor() as u8;
    let matches = |k: [u8; 4]| {
        k[0].abs_diff(seed[0]) <= tol
            && k[1].abs_diff(seed[1]) <= tol
            && k[2].abs_diff(seed[2]) <= tol
            && k[3].abs_diff(seed[3]) <= tol
    };
    // The region: 1 for a pixel in the region, else 0.
    let mut region = vec![0u8; wu * hu];
    let bounds = if settings.contiguous {
        fill_contiguous(&mut region, keys, wu, hu, (x as usize, y as usize), matches)
    } else {
        fill_all(&mut region, keys, wu, matches)
    };
    // Pixels outside the bounds (plus the 1 px ring) stay 0.
    let mut data = vec![0.0f32; wu * hu];
    if let Some(b) = bounds {
        if settings.anti_alias {
            write_anti_aliased(&mut data, &region, wu, hu, b);
        } else {
            write_binary(&mut data, &region, wu, b);
        }
    }
    MaskImage::from_data(w, h, data)
}

/// Marks all matching pixels. Returns the bounds of the region.
fn fill_all(
    region: &mut [u8],
    keys: &[[u8; 4]],
    w: usize,
    matches: impl Fn([u8; 4]) -> bool,
) -> Option<Bounds> {
    let mut bounds = None;
    for (y, (row, krow)) in region
        .chunks_exact_mut(w)
        .zip(keys.chunks_exact(w))
        .enumerate()
    {
        for (r, &k) in row.iter_mut().zip(krow) {
            *r = u8::from(matches(k));
        }
        if let (Some(first), Some(last)) = (
            row.iter().position(|&r| r == 1),
            row.iter().rposition(|&r| r == 1),
        ) {
            Bounds::add_span(&mut bounds, first, last + 1, y);
        }
    }
    bounds
}

/// Marks the 4-connected matching pixels from `seed` with a scanline flood
/// fill. Returns the bounds of the region.
fn fill_contiguous(
    region: &mut [u8],
    keys: &[[u8; 4]],
    w: usize,
    h: usize,
    seed: (usize, usize),
    matches: impl Fn([u8; 4]) -> bool,
) -> Option<Bounds> {
    let open = |region: &[u8], i: usize| region[i] == 0 && matches(keys[i]);
    let mut bounds = None;
    let mut stack = vec![seed];
    while let Some((sx, sy)) = stack.pop() {
        let row = sy * w;
        if !open(region, row + sx) {
            continue;
        }
        // Extend the span to the left and to the right.
        let mut l = sx;
        while l > 0 && open(region, row + l - 1) {
            l -= 1;
        }
        let mut r = sx + 1;
        while r < w && open(region, row + r) {
            r += 1;
        }
        region[row + l..row + r].fill(1);
        Bounds::add_span(&mut bounds, l, r, sy);
        // Push one seed for each run of open pixels above and below the
        // span. Only pixels directly above or below the span are
        // neighbors, so the fill is 4-connected.
        let above = sy.checked_sub(1);
        let below = (sy + 1 < h).then_some(sy + 1);
        for ny in [above, below].into_iter().flatten() {
            let nrow = ny * w;
            let mut in_run = false;
            for nx in l..r {
                let ok = open(region, nrow + nx);
                if ok && !in_run {
                    stack.push((nx, ny));
                }
                in_run = ok;
            }
        }
    }
    bounds
}

/// Writes the binary region (`0`/`1`) into `data` in the bounds.
fn write_binary(data: &mut [f32], region: &[u8], w: usize, b: Bounds) {
    for y in b.y0..b.y1 {
        let (s, e) = (y * w + b.x0, y * w + b.x1);
        for (d, &r) in data[s..e].iter_mut().zip(&region[s..e]) {
            *d = f32::from(r);
        }
    }
}

/// Writes the anti-aliased region into `data` in the bounds plus a 1 px
/// ring: 1 inside the region, else the mean of the region over the 3 x 3
/// neighborhood (pixels outside the canvas count as 0).
fn write_anti_aliased(data: &mut [f32], region: &[u8], w: usize, h: usize, b: Bounds) {
    let x0 = b.x0.saturating_sub(1);
    let x1 = (b.x1 + 1).min(w);
    let y0 = b.y0.saturating_sub(1);
    let y1 = (b.y1 + 1).min(h);
    let span = x1 - x0;
    // `padded[i]` is the region value at `x0 - 1 + i` (0 outside the
    // canvas). Then the horizontal sum of 3 pixels at `x0 + i` is
    // `padded[i] + padded[i + 1] + padded[i + 2]`.
    let mut padded = vec![0u8; span + 2];
    let mut row_sums = |y: Option<usize>, out: &mut [u8]| match y {
        Some(y) if y < h => {
            let row = &region[y * w..(y + 1) * w];
            let lo = x0.saturating_sub(1);
            let hi = (x1 + 1).min(w);
            padded.fill(0);
            // `x0 - 1 + i = lo` gives `i = lo + 1 - x0` (0 or 1).
            let start = lo + 1 - x0;
            padded[start..start + hi - lo].copy_from_slice(&row[lo..hi]);
            for (i, s) in out.iter_mut().enumerate() {
                *s = padded[i] + padded[i + 1] + padded[i + 2];
            }
        }
        _ => out.fill(0),
    };
    let mut prev = vec![0u8; span];
    let mut cur = vec![0u8; span];
    let mut next = vec![0u8; span];
    row_sums(y0.checked_sub(1), &mut prev);
    row_sums(Some(y0), &mut cur);
    for y in y0..y1 {
        row_sums(Some(y + 1), &mut next);
        let (s, e) = (y * w + x0, y * w + x1);
        let rows = prev.iter().zip(&cur).zip(&next);
        for ((d, &r), ((&a, &c), &n)) in data[s..e].iter_mut().zip(&region[s..e]).zip(rows) {
            // The count is at most 9, so `n / 9` is in `0..=1`.
            *d = if r == 1 {
                1.0
            } else {
                f32::from(a + c + n) / 9.0
            };
        }
        std::mem::swap(&mut prev, &mut cur);
        std::mem::swap(&mut cur, &mut next);
    }
}

impl Document {
    /// Combines a canvas-size region (for example from [`flood_region`])
    /// with the selection by `mode`. The rules are the same as for
    /// [`Document::select_shape`] without feather: no selection counts as
    /// `a = 0`. Region values are clamped to `0..=1`.
    ///
    /// Errors: [`EditorError::InvalidRect`] if the region does not have the
    /// canvas size, [`EditorError::NonFinite`] for a region value that is
    /// not finite.
    pub fn select_region(&mut self, region: &MaskImage, mode: SelectionMode) -> EditorResult<Edit> {
        check_region(self, region)?;
        let mut values: Vec<f32> = region.data().iter().map(|v| v.clamp(0.0, 1.0)).collect();
        if mode != SelectionMode::Replace {
            match self.selection() {
                Some(old) => {
                    for (b, &a) in values.iter_mut().zip(old.data()) {
                        *b = mode.combine(a, *b);
                    }
                }
                None => {
                    for b in &mut values {
                        *b = mode.combine(0.0, *b);
                    }
                }
            }
        }
        let mask = MaskImage::from_data(self.width(), self.height(), values)?;
        Ok(self.replace_selection(Some(mask)))
    }

    /// Paints `fill` on a pixel layer with the coverage
    /// `region * selection * opacity` (see the
    /// [module documentation](crate::flood)).
    ///
    /// Returns the undo edit and the changed rectangle in canvas
    /// coordinates, or `None` if no value changed. If a value changed, the
    /// layer gets a new pixel version and the document a new revision.
    /// The edit holds the original values of the changed tiles
    /// ([`STROKE_TILE_SIZE`] pixels) only.
    ///
    /// Errors:
    /// - Bad settings ([`FillSettings::validate`]).
    /// - [`EditorError::InvalidRect`] / [`EditorError::NonFinite`] for a
    ///   bad region (see [`Document::select_region`]).
    /// - [`EditorError::LayerNotFound`], [`EditorError::NotAPixelLayer`].
    /// - [`EditorError::Locked`] if the pixels lock (or `all`) is set on
    ///   the layer or on a parent group.
    pub fn fill_region(
        &mut self,
        layer: LayerId,
        region: &MaskImage,
        fill: &FillSettings,
    ) -> EditorResult<Option<(Edit, Rect)>> {
        fill.validate()?;
        check_region(self, region)?;
        let l = self.layer(layer).ok_or(EditorError::LayerNotFound(layer))?;
        if l.as_pixel().is_none() {
            return Err(EditorError::NotAPixelLayer(layer));
        }
        let locks = self.effective_locks(layer)?;
        locks.check(layer, LockKind::Pixels)?;
        let mode = Mode::paint(
            fill.color,
            fill.blend_mode,
            self.blend_space,
            locks.transparency,
        );
        let canvas = (self.width(), self.height());
        let (l, selection) = self.layer_mut_and_selection(layer)?;
        let LayerKind::Pixel(p) = l.kind_mut() else {
            return Err(EditorError::NotAPixelLayer(layer));
        };
        let offset = p.offset();
        let img = p.image_mut();
        let (lw, lh) = (img.width(), img.height());
        let Some(o) = Overlap::new(offset, (lw, lh), canvas) else {
            return Ok(None);
        };
        let ctx = FillContext {
            mode,
            opacity: fill.opacity,
            region: region.data(),
            selection: selection.map(MaskImage::data),
            canvas_w: canvas.0 as usize,
            layer_w: lw as usize,
            overlap: o,
        };
        let (tiles, changed) = ctx.apply(img.pixels_mut());
        let Some(b) = changed else {
            return Ok(None);
        };
        let to_rect = |x0: usize, y0: usize, x1: usize, y1: usize| -> Option<Rect> {
            Some(Rect::new(
                u32::try_from(x0).ok()?,
                u32::try_from(y0).ok()?,
                u32::try_from(x1 - x0).ok()?,
                u32::try_from(y1 - y0).ok()?,
            ))
        };
        // The bounds are layer buffer indices, so they fit in u32. If they
        // do not, the change log is cleared (the area is then not known).
        match to_rect(b.x0, b.y0, b.x1, b.y1) {
            Some(r) => l.touch_pixels_in(r),
            None => l.touch_pixels(),
        }
        self.touch();
        // The changed area is inside the overlap (`b.x0 >= o.lx`,
        // `b.y0 >= o.ly`), so the canvas rectangle is inside the canvas.
        // If the conversion fails, report the full canvas.
        let rect = to_rect(
            b.x0 - o.lx + o.cx,
            b.y0 - o.ly + o.cy,
            b.x1 - o.lx + o.cx,
            b.y1 - o.ly + o.cy,
        )
        .unwrap_or_else(|| Rect::new(0, 0, canvas.0, canvas.1));
        Ok(Some((Edit::pixel_tiles(layer, tiles), rect)))
    }
}

/// The fixed inputs of a fill.
struct FillContext<'a> {
    mode: Mode,
    opacity: f32,
    /// The region values (canvas size).
    region: &'a [f32],
    /// The selection values (canvas size), or `None` (all selected).
    selection: Option<&'a [f32]>,
    canvas_w: usize,
    layer_w: usize,
    overlap: Overlap,
}

impl FillContext<'_> {
    /// Paints the overlap area of `buf` tile by tile. Returns the original
    /// values of the changed tiles (layer coordinates) and the bounds of
    /// the changed pixels (layer coordinates).
    fn apply(&self, buf: &mut [Rgba]) -> (Vec<(Rect, Vec<Rgba>)>, Option<Bounds>) {
        let o = self.overlap;
        let tile = STROKE_TILE_SIZE as usize;
        let (x0, x1) = (o.lx, o.lx + o.width);
        let (y0, y1) = (o.ly, o.ly + o.height);
        let mut tiles = Vec::new();
        let mut changed = None;
        for ty in y0 / tile..=(y1 - 1) / tile {
            let (ty0, ty1) = ((ty * tile).max(y0), ((ty + 1) * tile).min(y1));
            for tx in x0 / tile..=(x1 - 1) / tile {
                let (tx0, tx1) = ((tx * tile).max(x0), ((tx + 1) * tile).min(x1));
                if let Some(orig) = self.apply_tile(buf, (tx0, tx1), (ty0, ty1), &mut changed) {
                    // The tile is inside the layer buffer, so the values
                    // fit in u32 (the buffer size is below `MAX_PIXELS`).
                    #[allow(clippy::cast_possible_truncation)]
                    let rect = Rect::new(
                        tx0 as u32,
                        ty0 as u32,
                        (tx1 - tx0) as u32,
                        (ty1 - ty0) as u32,
                    );
                    tiles.push((rect, orig));
                }
            }
        }
        (tiles, changed)
    }

    /// Paints one tile (`xs`, `ys` in layer coordinates). Returns the
    /// original values of the tile if a value changed.
    fn apply_tile(
        &self,
        buf: &mut [Rgba],
        xs: (usize, usize),
        ys: (usize, usize),
        changed: &mut Option<Bounds>,
    ) -> Option<Vec<Rgba>> {
        let o = self.overlap;
        let mut orig: Option<Vec<Rgba>> = None;
        for y in ys.0..ys.1 {
            let crow = (y - o.ly + o.cy) * self.canvas_w;
            let brow = y * self.layer_w;
            let mut first = None;
            let mut last = 0;
            for x in xs.0..xs.1 {
                let ci = crow + x - o.lx + o.cx;
                let mut c = self.region[ci].clamp(0.0, 1.0) * self.opacity;
                if let Some(sel) = self.selection {
                    c *= sel[ci].clamp(0.0, 1.0);
                }
                if c <= 0.0 {
                    continue;
                }
                let old = buf[brow + x];
                let new = self.mode.pixel(old, c);
                if new.same_bits(&old) {
                    continue;
                }
                // Copy the tile before its first change: the copy then
                // holds the original values.
                if orig.is_none() {
                    orig = Some(copy_rect(buf, self.layer_w, xs, ys));
                }
                buf[brow + x] = new;
                first.get_or_insert(x);
                last = x;
            }
            if let Some(first) = first {
                Bounds::add_span(changed, first, last + 1, y);
            }
        }
        orig
    }
}

/// Copies the area `xs` x `ys` of a buffer with the row length `w`.
fn copy_rect(buf: &[Rgba], w: usize, xs: (usize, usize), ys: (usize, usize)) -> Vec<Rgba> {
    let mut out = Vec::with_capacity((xs.1 - xs.0) * (ys.1 - ys.0));
    for y in ys.0..ys.1 {
        out.extend_from_slice(&buf[y * w + xs.0..y * w + xs.1]);
    }
    out
}
