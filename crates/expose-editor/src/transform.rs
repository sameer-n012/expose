//! Free transform of a pixel layer on the CPU (ADR 0029).
//!
//! [`Document::transform_layer`] applies an [`Affine`] matrix to the
//! visible content of a pixel layer, and to its mask. The result is one
//! reversible [`Edit`].
//!
//! # Coordinates
//!
//! The matrix maps canvas pixel coordinates to canvas pixel coordinates.
//! The y axis points down. Pixel `(i, j)` covers the area
//! `i..i + 1, j..j + 1`, and its center is at `(i + 0.5, j + 0.5)`.
//!
//! # Resampling
//!
//! The function maps the center of each destination pixel back into the
//! source (inverse mapping) and samples the source there. The source is
//! premultiplied RGBA, so that transparent pixels do not add their color.
//! The area outside the source is transparent (for a mask: the
//! [`MaskFill`](crate::MaskFill) value).
//!
//! - [`Interpolation::Nearest`]: the source pixel that contains the point.
//! - [`Interpolation::Bilinear`]: a tent filter (radius 1).
//! - [`Interpolation::Bicubic`]: a Catmull-Rom filter (radius 2).
//!
//! The filters are separable on the source axes. If the transform makes a
//! source axis shorter (a downscale), the filter becomes wider in the
//! source by `1 / scale` on that axis, so that fine detail does not alias.
//! The scale of a source axis is the length of that axis after the linear
//! part of the matrix (the column norm). This is exact for an axis scale,
//! a rotation, and an axis scale followed by a rotation. For a shear, or a
//! scale on rotated axes, it is an approximation. The filter radius in the
//! source is at most [`MAX_FILTER_RADIUS`] pixels, so that the cost of a
//! very large downscale stays limited. A downscale that needs a wider
//! filter can alias a little.
//!
//! The weights of each filter footprint are normalized to a sum of 1.
//! Catmull-Rom has negative lobes: the result alpha is clamped to
//! `0.0..=1.0`, a pixel with alpha 0 is fully transparent, and mask values
//! are clamped to `0.0..=1.0`. Color values are not clamped (linear values
//! above 1 are valid).

use expose_core::buffer::{Rect, Rgba, WorkingImage};

use crate::document::{Document, MAX_DIMENSION, check_size};
use crate::error::{EditorError, EditorResult};
use crate::layer::{LayerId, LockKind, Mask, MaskImage};
use crate::ops::Edit;

#[cfg(test)]
mod tests;

/// The largest filter radius in source pixels (see the module
/// documentation). A downscale by more than `filter radius / 32` (16x for
/// bicubic, 32x for bilinear) uses this radius.
pub const MAX_FILTER_RADIUS: f64 = 32.0;

/// The smallest permitted absolute determinant of a transform matrix.
/// Smaller values make the inverse map unstable.
pub const MIN_DETERMINANT: f64 = 1e-6;

/// Tolerance for the test of a whole-pixel translation.
const TRANSLATE_TOLERANCE: f64 = 1e-9;

/// Values nearer than this to a whole number count as that whole number
/// when the function rounds out the destination bounds. This removes an
/// empty row or column that rounding errors (for example, of `sin` and
/// `cos` in a rotation by 90 degrees) can add.
const SNAP_TOLERANCE: f64 = 1e-6;

/// The largest number of taps on one axis: `2 * MAX_FILTER_RADIUS + 1`.
const MAX_TAPS: usize = 66;

/// A 2D affine transform in canvas pixels.
///
/// `x' = a·x + c·y + e`, `y' = b·x + d·y + f`. This is the order of the
/// CSS `matrix(a, b, c, d, e, f)` function.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    /// x scale part.
    pub a: f64,
    /// y shear part (y' per x).
    pub b: f64,
    /// x shear part (x' per y).
    pub c: f64,
    /// y scale part.
    pub d: f64,
    /// x translation.
    pub e: f64,
    /// y translation.
    pub f: f64,
}

impl Affine {
    /// The transform that changes nothing.
    pub const IDENTITY: Self = Self::from_array([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// Makes a transform from `[a, b, c, d, e, f]`.
    pub const fn from_array(m: [f64; 6]) -> Self {
        Self {
            a: m[0],
            b: m[1],
            c: m[2],
            d: m[3],
            e: m[4],
            f: m[5],
        }
    }

    /// Returns `[a, b, c, d, e, f]`.
    pub const fn to_array(self) -> [f64; 6] {
        [self.a, self.b, self.c, self.d, self.e, self.f]
    }

    /// Makes a translation by `(x, y)`.
    pub fn translate(x: f64, y: f64) -> Self {
        Self::from_array([1.0, 0.0, 0.0, 1.0, x, y])
    }

    /// Makes a scale by `(sx, sy)` about the origin.
    pub fn scale(sx: f64, sy: f64) -> Self {
        Self::from_array([sx, 0.0, 0.0, sy, 0.0, 0.0])
    }

    /// Makes a rotation by `radians` about the origin. The y axis points
    /// down, so a positive angle turns clockwise on the screen: a rotation
    /// by +90 degrees maps `(1, 0)` to `(0, 1)`.
    pub fn rotate(radians: f64) -> Self {
        let (s, c) = radians.sin_cos();
        Self::from_array([c, s, -s, c, 0.0, 0.0])
    }

    /// Returns the transform that applies `self` first, then `next`.
    #[must_use]
    pub fn then(self, next: Self) -> Self {
        let n = next;
        Self {
            a: n.a * self.a + n.c * self.b,
            b: n.b * self.a + n.d * self.b,
            c: n.a * self.c + n.c * self.d,
            d: n.b * self.c + n.d * self.d,
            e: n.a * self.e + n.c * self.f + n.e,
            f: n.b * self.e + n.d * self.f + n.f,
        }
    }

    /// Maps the point `(x, y)`.
    pub fn apply(self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// Returns the determinant of the linear part (`a·d − b·c`).
    pub fn determinant(self) -> f64 {
        self.a * self.d - self.b * self.c
    }

    /// Returns the inverse transform, or `None` if the matrix is singular
    /// or the inverse has a value that is not finite.
    pub fn inverse(self) -> Option<Self> {
        let det = self.determinant();
        if det == 0.0 || !det.is_finite() {
            return None;
        }
        let a = self.d / det;
        let b = -self.b / det;
        let c = -self.c / det;
        let d = self.a / det;
        let inv = Self {
            a,
            b,
            c,
            d,
            e: -(a * self.e + c * self.f),
            f: -(b * self.e + d * self.f),
        };
        inv.to_array().iter().all(|v| v.is_finite()).then_some(inv)
    }

    /// Returns [`EditorError::InvalidTransform`] if a value is not finite
    /// or the absolute determinant is less than [`MIN_DETERMINANT`].
    pub fn validate(self) -> EditorResult<()> {
        if !self.to_array().iter().all(|v| v.is_finite()) {
            return Err(EditorError::InvalidTransform(
                "matrix has a value that is not finite",
            ));
        }
        let det = self.determinant();
        if !det.is_finite() || det.abs() < MIN_DETERMINANT {
            return Err(EditorError::InvalidTransform("matrix is singular"));
        }
        if self.inverse().is_none() {
            return Err(EditorError::InvalidTransform(
                "matrix has no finite inverse",
            ));
        }
        Ok(())
    }

    /// Returns the translation in whole pixels if the transform is a pure
    /// translation by whole pixels (within [`TRANSLATE_TOLERANCE`]).
    fn whole_translation(self) -> Option<(f64, f64)> {
        let near = |v: f64, target: f64| (v - target).abs() <= TRANSLATE_TOLERANCE;
        let linear =
            near(self.a, 1.0) && near(self.b, 0.0) && near(self.c, 0.0) && near(self.d, 1.0);
        let (ex, fy) = (self.e.round(), self.f.round());
        (linear && near(self.e, ex) && near(self.f, fy)).then_some((ex, fy))
    }
}

/// The interpolation of a transform (see the module documentation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Interpolation {
    /// Catmull-Rom cubic filter (radius 2).
    #[default]
    Bicubic,
    /// Tent filter (radius 1).
    Bilinear,
    /// The nearest source pixel. No anti-alias filter.
    Nearest,
}

impl Interpolation {
    /// The filter radius in source pixels at scale 1.
    const fn radius(self) -> f64 {
        match self {
            Self::Bicubic => 2.0,
            Self::Bilinear => 1.0,
            Self::Nearest => 0.0,
        }
    }

    /// The filter weight at distance `x` (in filter units).
    fn weight(self, x: f32) -> f32 {
        let x = x.abs();
        match self {
            Self::Bicubic => {
                if x < 1.0 {
                    (1.5 * x - 2.5) * x * x + 1.0
                } else if x < 2.0 {
                    ((-0.5 * x + 2.5) * x - 4.0) * x + 2.0
                } else {
                    0.0
                }
            }
            Self::Bilinear => (1.0 - x).max(0.0),
            Self::Nearest => f32::from(u8::from(x < 0.5)),
        }
    }
}

/// A rectangle in canvas pixel coordinates that can be partly or fully
/// outside the canvas.
///
/// [`Rect`] has unsigned coordinates, so it cannot hold content to the
/// left of or above the canvas. `x` and `y` are `i64`, because a layer
/// offset (`i32`) plus a position in the layer (up to
/// [`MAX_DIMENSION`]) can be outside the `i32` range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CanvasRect {
    /// Left edge (first column).
    pub x: i64,
    /// Top edge (first row).
    pub y: i64,
    /// Width in pixels (at least 1).
    pub width: u32,
    /// Height in pixels (at least 1).
    pub height: u32,
}

/// The destination area of a transform: canvas position and size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Target {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

/// Returns the rectangle of the pixels with alpha > 0, in image
/// coordinates, or `None` if all pixels are transparent.
fn content_rect(image: &WorkingImage) -> Option<Rect> {
    let mut x0 = usize::MAX;
    let mut x1 = 0;
    let mut y0 = None;
    let mut y1 = 0;
    for (y, row) in image.rows().enumerate() {
        let Some(first) = row.iter().position(|p| p[3] > 0.0) else {
            continue;
        };
        // `position` found an index, so `rposition` finds one too.
        let last = row.iter().rposition(|p| p[3] > 0.0).unwrap_or(first);
        x0 = x0.min(first);
        x1 = x1.max(last);
        y0.get_or_insert(y);
        y1 = y;
    }
    let y0 = y0?;
    // All values are less than the image size, which is a `u32`.
    let to_u32 = |v: usize| u32::try_from(v).unwrap_or(u32::MAX);
    Some(Rect::new(
        to_u32(x0),
        to_u32(y0),
        to_u32(x1 - x0 + 1),
        to_u32(y1 - y0 + 1),
    ))
}

/// Rounds `v` to the nearest whole number if it is within
/// [`SNAP_TOLERANCE`] of it.
fn snap(v: f64) -> f64 {
    let r = v.round();
    if (v - r).abs() < SNAP_TOLERANCE { r } else { v }
}

/// Returns the destination area of the source rectangle
/// `(x, y, width, height)` (canvas coordinates) after `m`: the bounding box
/// of the four mapped corners, rounded out to whole pixels.
fn transformed_bounds(m: &Affine, x: f64, y: f64, width: f64, height: f64) -> EditorResult<Target> {
    let corners = [
        m.apply(x, y),
        m.apply(x + width, y),
        m.apply(x, y + height),
        m.apply(x + width, y + height),
    ];
    let mut min = (f64::INFINITY, f64::INFINITY);
    let mut max = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (cx, cy) in corners {
        min = (min.0.min(cx), min.1.min(cy));
        max = (max.0.max(cx), max.1.max(cy));
    }
    let (x0, y0) = (snap(min.0).floor(), snap(min.1).floor());
    let (x1, y1) = (snap(max.0).ceil(), snap(max.1).ceil());
    if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
        return Err(EditorError::InvalidTransform(
            "transformed bounds are not finite",
        ));
    }
    // A very thin result can round to an empty span. Keep one pixel.
    let w = (x1 - x0).max(1.0);
    let h = (y1 - y0).max(1.0);
    let limit = f64::from(MAX_DIMENSION);
    if w > limit || h > limit {
        // The `as` casts saturate, which is correct for the error report.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        return Err(EditorError::TooLarge {
            width: w as u32,
            height: h as u32,
            reason: "width or height is more than MAX_DIMENSION",
        });
    }
    let range = f64::from(i32::MIN)..=f64::from(i32::MAX);
    if !range.contains(&x0) || !range.contains(&y0) {
        return Err(EditorError::OffsetOverflow);
    }
    // All four values are whole numbers in range (checked above).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let target = Target {
        x: x0 as i32,
        y: y0 as i32,
        width: w as u32,
        height: h as u32,
    };
    check_size(target.width, target.height)?;
    Ok(target)
}

/// A source image for [`resample`].
struct Source<'a, const N: usize> {
    /// Row-major texels.
    data: &'a [[f32; N]],
    width: u32,
    height: u32,
    /// Canvas position of the top-left texel.
    offset: (i32, i32),
    /// The value outside the source.
    outside: [f32; N],
}

/// The filter settings of one transform.
#[derive(Debug, Clone, Copy)]
struct Filter {
    interp: Interpolation,
    /// Filter scale on the source x axis (`0 < sx <= 1`).
    sx: f64,
    /// Filter scale on the source y axis.
    sy: f64,
    /// Filter radius in source pixels on x (`radius / sx`).
    rx: f64,
    /// Filter radius in source pixels on y.
    ry: f64,
}

impl Filter {
    fn new(m: &Affine, interp: Interpolation) -> Self {
        let r = interp.radius();
        if r == 0.0 {
            return Self {
                interp,
                sx: 1.0,
                sy: 1.0,
                rx: 0.0,
                ry: 0.0,
            };
        }
        let min_scale = r / MAX_FILTER_RADIUS;
        let sx = m.a.hypot(m.b).clamp(min_scale, 1.0);
        let sy = m.c.hypot(m.d).clamp(min_scale, 1.0);
        Self {
            interp,
            sx,
            sy,
            rx: r / sx,
            ry: r / sy,
        }
    }
}

/// Weights of the taps of one axis. `lo..hi` are the indices in `w` of the
/// taps inside the source. `start` is the source index of `w[lo]`.
struct Taps {
    w: [f32; MAX_TAPS],
    lo: usize,
    hi: usize,
    start: usize,
    /// Sum of the normalized weights inside the source.
    inside: f32,
}

impl Taps {
    /// Computes the normalized weights for the continuous source position
    /// `pos` (pixel centers at `i + 0.5`) on an axis with `len` pixels.
    /// Returns `None` if no tap is inside the source.
    // Source positions are inside a small band around the source (see
    // `span`), so the `as` casts of floor and ceil values are exact, and
    // tap counts (at most `MAX_TAPS`) convert to `f64` exactly.
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    fn new(pos: f64, len: u32, radius: f64, scale: f64, interp: Interpolation) -> Option<Self> {
        let t = pos - 0.5;
        let first_f = (t - radius).ceil();
        let first = first_f as i64;
        let last = (t + radius).floor() as i64;
        let count = usize::try_from(last - first + 1).ok()?.min(MAX_TAPS);
        // Tap `k` has the source index `first + k`. The taps inside the
        // source are `lo..hi`.
        let lo = usize::try_from(-first).unwrap_or(0);
        let hi = usize::try_from(i64::from(len) - first)
            .unwrap_or(0)
            .min(count);
        if lo >= hi {
            return None;
        }
        let mut w = [0.0f32; MAX_TAPS];
        let mut sum = 0.0f32;
        let scale = scale as f32;
        for (k, slot) in w.iter_mut().take(count).enumerate() {
            let d = (first_f + k as f64 - t) as f32;
            *slot = interp.weight(d * scale);
            sum += *slot;
        }
        if sum.abs() < 1e-12 {
            return None;
        }
        let inv = 1.0 / sum;
        for v in w.iter_mut().take(count) {
            *v *= inv;
        }
        let inside = w[lo..hi].iter().sum();
        // `first + lo` is in `0..len`: `lo` is 0 if `first >= 0`, else
        // `-first`.
        let start = usize::try_from(first.max(0)).ok()?;
        Some(Self {
            w,
            lo,
            hi,
            start,
            inside,
        })
    }
}

/// Returns the range of `x` (as floats) where `start + step·x` is inside
/// the open interval `(lo, hi)`. `None` means an empty range.
fn span(start: f64, step: f64, lo: f64, hi: f64) -> Option<(f64, f64)> {
    if step.abs() < 1e-12 {
        return (start > lo && start < hi).then_some((f64::NEG_INFINITY, f64::INFINITY));
    }
    let a = (lo - start) / step;
    let b = (hi - start) / step;
    Some((a.min(b), a.max(b)))
}

/// Resamples `src` into the destination area `dst` with the inverse map
/// `inv` (destination canvas → source canvas).
// The casts convert pixel indices (less than `MAX_DIMENSION`) between
// integer and float types; they are exact.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn resample<const N: usize>(
    src: &Source<'_, N>,
    inv: &Affine,
    dst: Target,
    filter: Filter,
) -> Vec<[f32; N]> {
    let dw = dst.width as usize;
    let dh = dst.height as usize;
    let mut out = vec![src.outside; dw * dh];
    let sw = src.width as usize;
    let (swf, shf) = (f64::from(src.width), f64::from(src.height));
    // Valid source positions: inside the source, or within the filter
    // radius of a source pixel center.
    let nearest = filter.interp == Interpolation::Nearest;
    let (ulo, uhi, vlo, vhi) = if nearest {
        (0.0, swf, 0.0, shf)
    } else {
        (
            0.5 - filter.rx,
            swf - 0.5 + filter.rx,
            0.5 - filter.ry,
            shf - 0.5 + filter.ry,
        )
    };
    let (sox, soy) = (f64::from(src.offset.0), f64::from(src.offset.1));
    for (y, row) in out.chunks_exact_mut(dw).enumerate() {
        let cy = f64::from(dst.y) + y as f64 + 0.5;
        let (qx, qy) = inv.apply(f64::from(dst.x) + 0.5, cy);
        let (u0, v0) = (qx - sox, qy - soy);
        let (Some(xu), Some(xv)) = (span(u0, inv.a, ulo, uhi), span(v0, inv.b, vlo, vhi)) else {
            continue;
        };
        let x_from = xu.0.max(xv.0).max(0.0);
        let x_to = xu.1.min(xv.1).min(dw as f64);
        if x_from > x_to {
            continue;
        }
        // One pixel of margin on each side: the loop body handles points
        // outside the source correctly.
        let x_start = (x_from.floor() as usize).saturating_sub(1);
        let x_end = ((x_to.ceil() as usize) + 1).min(dw);
        for (x, px) in row.iter_mut().enumerate().take(x_end).skip(x_start) {
            let u = u0 + inv.a * x as f64;
            let v = v0 + inv.b * x as f64;
            if nearest {
                let (iu, iv) = (u.floor(), v.floor());
                if iu >= 0.0 && iu < swf && iv >= 0.0 && iv < shf {
                    *px = src.data[iv as usize * sw + iu as usize];
                }
                continue;
            }
            let Some(tx) = Taps::new(u, src.width, filter.rx, filter.sx, filter.interp) else {
                continue;
            };
            let Some(ty) = Taps::new(v, src.height, filter.ry, filter.sy, filter.interp) else {
                continue;
            };
            let mut acc = [0.0f32; N];
            let wxs = &tx.w[tx.lo..tx.hi];
            // `Taps` keeps `start + (hi - lo)` within the source length, so
            // the slices below are inside `src.data`.
            for (j, wy) in ty.w[ty.lo..ty.hi].iter().enumerate() {
                let base = (ty.start + j) * sw + tx.start;
                let texels = &src.data[base..base + wxs.len()];
                let mut racc = [0.0f32; N];
                for (t, wx) in texels.iter().zip(wxs) {
                    for (r, v) in racc.iter_mut().zip(t) {
                        *r += wx * v;
                    }
                }
                for (a, r) in acc.iter_mut().zip(racc) {
                    *a += wy * r;
                }
            }
            let rest = 1.0 - tx.inside * ty.inside;
            for (a, o) in acc.iter_mut().zip(src.outside) {
                *a += rest * o;
            }
            *px = acc;
        }
    }
    out
}

/// Cleans a resampled pixel: alpha in `0.0..=1.0`, a pixel with alpha 0 is
/// fully transparent, and values that are not finite become 0.
fn clean_pixel(p: &mut Rgba) {
    let a = p[3];
    if a.is_nan() || a <= 0.0 || !p.iter().all(|v| v.is_finite()) {
        *p = [0.0; 4];
        return;
    }
    p[3] = a.min(1.0);
}

/// Cleans a resampled mask value: `0.0..=1.0`, and 0 if not finite.
fn clean_mask(v: &mut f32) {
    *v = if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    };
}

impl Document {
    /// Returns the canvas rectangle of the pixels with alpha > 0 of a pixel
    /// layer, or `None` if all pixels are transparent.
    ///
    /// The rectangle is not clipped to the canvas: content outside the
    /// canvas is included (see [`CanvasRect`]).
    pub fn layer_content_bounds(&self, layer: LayerId) -> EditorResult<Option<CanvasRect>> {
        let p = self
            .layer(layer)
            .ok_or(EditorError::LayerNotFound(layer))?
            .as_pixel()
            .ok_or(EditorError::NotAPixelLayer(layer))?;
        let (ox, oy) = p.offset();
        Ok(content_rect(p.image()).map(|r| CanvasRect {
            x: i64::from(ox) + i64::from(r.x),
            y: i64::from(oy) + i64::from(r.y),
            width: r.width,
            height: r.height,
        }))
    }

    /// Transforms the content of a pixel layer, and its mask, with the
    /// matrix `m` (canvas pixels, see the module documentation).
    ///
    /// - Only the content bounds (see [`Self::layer_content_bounds`]) are
    ///   transformed. The new layer image is the bounding box of the
    ///   transformed content rectangle, rounded out to whole pixels. Its
    ///   top-left corner is the new layer offset.
    /// - A translation by whole pixels changes only the offsets of the
    ///   layer and the mask. The pixels stay the same, bit for bit.
    /// - A mask is resampled with the same matrix and interpolation, over
    ///   the transformed area of the full mask image. Outside that area,
    ///   the mask keeps its fill value.
    ///
    /// Errors:
    /// - [`EditorError::InvalidTransform`]: a value that is not finite, a
    ///   singular matrix, or a layer with no pixel with alpha > 0.
    /// - [`EditorError::NotAPixelLayer`]: the layer is not a pixel layer.
    /// - [`EditorError::Locked`]: the pixels, position, or `all` lock of
    ///   the layer or of a parent group. The transparency lock does not
    ///   prevent a transform (a transform moves pixels, as a move does).
    /// - [`EditorError::TooLarge`] / [`EditorError::OffsetOverflow`]: the
    ///   result is larger than the size limits or outside the offset range.
    ///
    /// The returned edit restores the old image, offset, and mask.
    pub fn transform_layer(
        &mut self,
        layer: LayerId,
        m: &Affine,
        interp: Interpolation,
    ) -> EditorResult<Edit> {
        m.validate()?;
        let l = self.layer(layer).ok_or(EditorError::LayerNotFound(layer))?;
        let p = l.as_pixel().ok_or(EditorError::NotAPixelLayer(layer))?;
        let locks = self.effective_locks(layer)?;
        locks.check(layer, LockKind::Pixels)?;
        locks.check(layer, LockKind::Position)?;
        let content = content_rect(p.image())
            .ok_or(EditorError::InvalidTransform("layer has no visible pixels"))?;

        if let Some((dx, dy)) = m.whole_translation() {
            let range = f64::from(i32::MIN)..=f64::from(i32::MAX);
            if !range.contains(&dx) || !range.contains(&dy) {
                return Err(EditorError::OffsetOverflow);
            }
            // Whole numbers in the i32 range (checked above).
            #[allow(clippy::cast_possible_truncation)]
            return self.translate_layer(layer, dx as i32, dy as i32);
        }

        // `validate` checked that the inverse exists.
        let inv = m.inverse().ok_or(EditorError::InvalidTransform(
            "matrix has no finite inverse",
        ))?;
        let filter = Filter::new(m, interp);

        let (ox, oy) = p.offset();
        let target = transformed_bounds(
            m,
            f64::from(ox) + f64::from(content.x),
            f64::from(oy) + f64::from(content.y),
            f64::from(content.width),
            f64::from(content.height),
        )?;
        let mask_target = match l.mask() {
            Some(mask) => Some(transformed_bounds(
                m,
                f64::from(mask.offset.0),
                f64::from(mask.offset.1),
                f64::from(mask.image.width()),
                f64::from(mask.image.height()),
            )?),
            None => None,
        };

        let image = p.image();
        let src = Source {
            data: image.pixels(),
            width: image.width(),
            height: image.height(),
            offset: (ox, oy),
            outside: [0.0; 4],
        };
        let mut pixels = resample(&src, &inv, target, filter);
        pixels.iter_mut().for_each(clean_pixel);
        let new_image = WorkingImage::from_pixels(target.width, target.height, pixels)?;

        let new_mask = match (l.mask(), mask_target) {
            (Some(mask), Some(t)) => {
                // A mask image has one value per pixel, so the chunk
                // remainder is empty.
                let (texels, _) = mask.image.data().as_chunks::<1>();
                let src = Source {
                    data: texels,
                    width: mask.image.width(),
                    height: mask.image.height(),
                    offset: mask.offset,
                    outside: [mask.fill.value()],
                };
                let mut values = resample(&src, &inv, t, filter).into_flattened();
                values.iter_mut().for_each(clean_mask);
                Some(Mask {
                    image: MaskImage::from_data(t.width, t.height, values)?,
                    offset: (t.x, t.y),
                    enabled: mask.enabled,
                    fill: mask.fill,
                })
            }
            _ => None,
        };

        self.apply_edit(Edit::pixel_layer(
            layer,
            new_image,
            (target.x, target.y),
            new_mask,
        ))
    }
}
