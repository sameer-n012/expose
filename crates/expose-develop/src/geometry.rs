//! Geometry stage: flip, straighten (rotate), and crop.
//!
//! This module is the CPU reference. A GPU kernel must use the same
//! per-pixel mapping ([`Geometry::affine`]) and the same sampling rule.
//!
//! # Order of operations
//!
//! The source image has the size `W x H`. It is already display-oriented.
//!
//! 1. Flip: mirror the source left to right and/or top to bottom.
//! 2. Rotate: turn the flipped image by `angle` degrees clockwise on the
//!    screen (y points down) about the image center `(W/2, H/2)`.
//! 3. Crop: the crop frame is a `W x H` frame with the same center as the
//!    image. [`Crop::rect`] is normalized to this frame (origin top-left).
//!    The output is the part of the rotated image inside the rect.
//!
//! # Output size
//!
//! - `out_w = max(1, round(rect.width * W))`, `out_h = max(1, round(rect.height * H))`,
//!   both limited to `W` and `H`.
//! - Origin in frame pixels: `x0 = round(rect.left * W)`, `y0 = round(rect.top * H)`,
//!   clamped so that `0 <= x0`, `x0 + out_w <= W`, `0 <= y0`, `y0 + out_h <= H`.
//!
//! # Pixel-center convention
//!
//! Positions are continuous. Pixel `i` covers `[i, i + 1)` and its center is
//! at `i + 0.5`. For output pixel `(x, y)`:
//!
//! 1. Frame point `p = (x0 + x + 0.5, y0 + y + 0.5)`.
//! 2. `d = p - (W/2, H/2)`.
//! 3. Inverse rotation (counter-clockwise on the screen by `angle`):
//!    `d' = (cos * d.x + sin * d.y, -sin * d.x + cos * d.y)`.
//! 4. `s = d' + (W/2, H/2)`.
//! 5. Flip: `s.x = W - s.x` (horizontal flip), `s.y = H - s.y` (vertical flip).
//!
//! All five steps are one affine transform. [`Geometry::affine`] gives it as
//! `s = (a*ox + b*oy + c, d*ox + e*oy + f)` with `ox = x + 0.5`,
//! `oy = y + 0.5`. The matrix is computed in `f64` and stored as `f32`. The
//! per-pixel transform and the sampling use `f32`, as a GPU kernel does.
//!
//! # Sampling
//!
//! - `angle == 0`: the mapping is integer-aligned. The stage copies pixels
//!   exactly (no interpolation), so a pure crop or flip is bit-exact.
//! - `angle != 0`: bicubic Catmull-Rom (`a = -0.5`, 4x4 taps, separable
//!   weights) on all four channels. Taps outside the source use the nearest
//!   edge pixel (clamp to edge). The stage does not clip values: scene-referred
//!   values outside `0..=1` pass through, and small overshoot near edges is
//!   accepted.
//!
//! # Outside rule
//!
//! When `s` is outside the source rectangle `[0, W] x [0, H]`, the output
//! pixel is transparent black `[0, 0, 0, 0]`. The crop tool must keep the
//! rect inside the rotated image: see [`max_inscribed_rect`] and
//! [`rect_is_inside`].

use expose_core::CoreError;
use expose_core::buffer::{Rgba, WorkingImage};
use expose_core::develop::{Crop, NormalizedRect};

use crate::error::{DevelopError, DevelopResult};

/// Tolerance in source pixels for [`rect_is_inside`].
const INSIDE_TOLERANCE_PIXELS: f64 = 0.01;

/// Transparent black, for sample points outside the source.
const TRANSPARENT: Rgba = [0.0; 4];

/// Precomputed geometry for one source size and one [`Crop`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    source_width: u32,
    source_height: u32,
    output_width: u32,
    output_height: u32,
    origin_x: u32,
    origin_y: u32,
    flip_horizontal: bool,
    flip_vertical: bool,
    /// `true` when the angle is not zero, so the stage must interpolate.
    rotated: bool,
    /// Output pixel center to source position (see the module docs).
    affine: [[f32; 3]; 2],
}

impl Geometry {
    /// Computes the geometry for `crop` and a source of the given size.
    ///
    /// Any finite angle is accepted (the settings validation limits it to
    /// `-45..=45`). The rect is rounded and clamped to the frame as the
    /// module docs tell.
    ///
    /// # Errors
    ///
    /// - [`DevelopError::Core`] with [`CoreError::InvalidDimensions`] when the
    ///   source width or height is zero.
    /// - [`DevelopError::Settings`] with [`CoreError::NonFinite`] when the
    ///   angle or a rect value is NaN or infinite.
    pub fn new(crop: &Crop, source_width: u32, source_height: u32) -> DevelopResult<Self> {
        check_size(source_width, source_height)?;
        check_finite("crop.angle", crop.angle)?;
        check_rect_finite(&crop.rect)?;

        let w = f64::from(source_width);
        let h = f64::from(source_height);
        let (output_width, origin_x) = span(crop.rect.left, crop.rect.width, source_width);
        let (output_height, origin_y) = span(crop.rect.top, crop.rect.height, source_height);

        let (sin, cos) = f64::from(crop.angle).to_radians().sin_cos();
        let (cx, cy) = (w / 2.0, h / 2.0);
        // Frame offset of the output origin, relative to the center.
        let ux = f64::from(origin_x) - cx;
        let uy = f64::from(origin_y) - cy;
        // Inverse rotation of (ux + ox, uy + oy), then move back by the center.
        let mut row_x = [cos, sin, cos * ux + sin * uy + cx];
        let mut row_y = [-sin, cos, -sin * ux + cos * uy + cy];
        if crop.flip_horizontal {
            row_x = [-row_x[0], -row_x[1], w - row_x[2]];
        }
        if crop.flip_vertical {
            row_y = [-row_y[0], -row_y[1], h - row_y[2]];
        }
        #[allow(
            clippy::cast_possible_truncation,
            reason = "the GPU uniform is f32; the precision loss is documented"
        )]
        let affine = [row_x.map(|v| v as f32), row_y.map(|v| v as f32)];

        Ok(Self {
            source_width,
            source_height,
            output_width,
            output_height,
            origin_x,
            origin_y,
            flip_horizontal: crop.flip_horizontal,
            flip_vertical: crop.flip_vertical,
            rotated: crop.angle != 0.0,
            affine,
        })
    }

    /// Returns `true` when the stage does not change the image: the rect
    /// rounds to the full frame, the angle is 0, and there is no flip.
    pub fn is_identity(&self) -> bool {
        !self.rotated
            && !self.flip_horizontal
            && !self.flip_vertical
            && self.origin_x == 0
            && self.origin_y == 0
            && self.output_width == self.source_width
            && self.output_height == self.source_height
    }

    /// Source size `(width, height)` in pixels.
    pub fn source_size(&self) -> (u32, u32) {
        (self.source_width, self.source_height)
    }

    /// Output size `(width, height)` in pixels.
    pub fn output_size(&self) -> (u32, u32) {
        (self.output_width, self.output_height)
    }

    /// Output origin `(x0, y0)` in frame pixels.
    pub fn origin(&self) -> (u32, u32) {
        (self.origin_x, self.origin_y)
    }

    /// The flips `(horizontal, vertical)`.
    pub fn flips(&self) -> (bool, bool) {
        (self.flip_horizontal, self.flip_vertical)
    }

    /// Returns `true` when the angle is not zero, so the stage interpolates.
    pub fn needs_resample(&self) -> bool {
        self.rotated
    }

    /// Maps the center of output pixel `(x, y)` to a continuous source
    /// position (pixel `i` has its center at `i + 0.5`). The GPU kernel uses
    /// the same affine and the same `f32` math order.
    pub fn source_position(&self, x: u32, y: u32) -> (f32, f32) {
        #[allow(
            clippy::cast_precision_loss,
            reason = "pixel indices are below 2^24 for supported image sizes"
        )]
        let (ox, oy) = (x as f32 + 0.5, y as f32 + 0.5);
        let [[a, b, c], [d, e, f]] = self.affine;
        (a * ox + b * oy + c, d * ox + e * oy + f)
    }

    /// Affine matrix `[[a, b, c], [d, e, f]]`:
    /// `s = (a*ox + b*oy + c, d*ox + e*oy + f)` with `ox = x + 0.5` and
    /// `oy = y + 0.5`. It includes the flips. Use it for the GPU uniform.
    pub fn affine(&self) -> [[f32; 3]; 2] {
        self.affine
    }

    /// Applies the geometry to `image` on the CPU.
    ///
    /// # Errors
    ///
    /// - [`DevelopError::SizeMismatch`] when the image size is not the
    ///   source size of this geometry.
    /// - [`DevelopError::Core`] when the output buffer cannot be made.
    pub fn apply_cpu(&self, image: &WorkingImage) -> DevelopResult<WorkingImage> {
        if image.width() != self.source_width || image.height() != self.source_height {
            return Err(DevelopError::SizeMismatch {
                expected: self.source_size(),
                actual: (image.width(), image.height()),
            });
        }
        let pixels = if self.rotated {
            self.resample(image)
        } else {
            self.copy_exact(image)
        };
        Ok(WorkingImage::from_pixels(
            self.output_width,
            self.output_height,
            pixels,
        )?)
    }

    /// Integer-aligned copy for `angle == 0` (crop and flip only).
    fn copy_exact(&self, image: &WorkingImage) -> Vec<Rgba> {
        let src = image.pixels();
        let sw = self.source_width as usize;
        let sh = self.source_height as usize;
        let (ow, oh) = (self.output_width as usize, self.output_height as usize);
        let (x0, y0) = (self.origin_x as usize, self.origin_y as usize);
        let mut out = Vec::with_capacity(ow * oh);
        for y in 0..oh {
            // `y0 + y < sh` because `y0 + oh <= sh` (see `span`).
            let fy = y0 + y;
            let sy = if self.flip_vertical { sh - 1 - fy } else { fy };
            let row = &src[sy * sw..(sy + 1) * sw];
            if self.flip_horizontal {
                out.extend((0..ow).map(|x| row[sw - 1 - (x0 + x)]));
            } else {
                out.extend_from_slice(&row[x0..x0 + ow]);
            }
        }
        out
    }

    /// Catmull-Rom resampling for `angle != 0`.
    fn resample(&self, image: &WorkingImage) -> Vec<Rgba> {
        #[allow(
            clippy::cast_precision_loss,
            reason = "image sizes are below 2^24 pixels per side"
        )]
        let (w, h) = (self.source_width as f32, self.source_height as f32);
        let mut out = Vec::with_capacity(self.output_width as usize * self.output_height as usize);
        for y in 0..self.output_height {
            for x in 0..self.output_width {
                let (sx, sy) = self.source_position(x, y);
                if !(0.0..=w).contains(&sx) || !(0.0..=h).contains(&sy) {
                    out.push(TRANSPARENT);
                } else {
                    out.push(sample_catmull_rom(image, sx, sy));
                }
            }
        }
        out
    }
}

/// Catmull-Rom weights (`a = -0.5`) for the taps `i-1, i, i+1, i+2` at the
/// fraction `t` in `[0, 1)`.
fn catmull_rom_weights(t: f32) -> [f32; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [
        -0.5 * t3 + t2 - 0.5 * t,
        1.5 * t3 - 2.5 * t2 + 1.0,
        -1.5 * t3 + 2.0 * t2 + 0.5 * t,
        0.5 * t3 - 0.5 * t2,
    ]
}

/// First tap index and weights for one axis. `s` is a continuous position;
/// the pixel centers are at `i + 0.5`.
fn axis_taps(s: f32) -> (i64, [f32; 4]) {
    let u = s - 0.5;
    let base = u.floor();
    #[allow(
        clippy::cast_possible_truncation,
        reason = "`s` is inside [0, size] with size below 2^24, so `base` fits"
    )]
    let first = base as i64 - 1;
    (first, catmull_rom_weights(u - base))
}

/// Clamps a tap index to `0..size` (clamp to edge).
fn clamp_index(i: i64, size: u32) -> usize {
    let max = size.saturating_sub(1);
    // After `max(0)` the index is not negative, so `try_from` fails only for
    // an index above `u32::MAX`. That index clamps to `max`.
    let index = u32::try_from(i.max(0)).map_or(max, |v| v.min(max));
    index as usize
}

/// Tap offsets `-1, 0, +1, +2` relative to the first tap.
const TAP_OFFSETS: [i64; 4] = [0, 1, 2, 3];

/// Samples `image` at the continuous position `(sx, sy)` with Catmull-Rom.
fn sample_catmull_rom(image: &WorkingImage, sx: f32, sy: f32) -> Rgba {
    let src = image.pixels();
    let width = image.width() as usize;
    let (fx, wx) = axis_taps(sx);
    let (fy, wy) = axis_taps(sy);
    let cols = TAP_OFFSETS.map(|k| clamp_index(fx + k, image.width()));
    let mut acc = [0.0_f32; 4];
    for (k, wyj) in TAP_OFFSETS.iter().zip(wy.iter()) {
        let row_start = clamp_index(fy + k, image.height()) * width;
        let mut row_acc = [0.0_f32; 4];
        for (col, wxk) in cols.iter().zip(wx.iter()) {
            let p = src[row_start + col];
            for c in 0..4 {
                row_acc[c] += wxk * p[c];
            }
        }
        for c in 0..4 {
            acc[c] += wyj * row_acc[c];
        }
    }
    acc
}

/// Output length and origin on one axis (see the module docs).
fn span(start: f32, length: f32, size: u32) -> (u32, u32) {
    let size_f = f64::from(size);
    let len = (f64::from(length) * size_f).round().clamp(1.0, size_f);
    let max_origin = size_f - len;
    let origin = (f64::from(start) * size_f).round().clamp(0.0, max_origin);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "both values are integers in 0..=size after round and clamp"
    )]
    (len as u32, origin as u32)
}

fn check_size(width: u32, height: u32) -> DevelopResult<()> {
    if width == 0 || height == 0 {
        return Err(CoreError::InvalidDimensions { width, height }.into());
    }
    Ok(())
}

fn check_finite(name: &'static str, value: f32) -> DevelopResult<()> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(DevelopError::Settings(CoreError::NonFinite { name }))
    }
}

fn check_rect_finite(rect: &NormalizedRect) -> DevelopResult<()> {
    check_finite("crop.rect.left", rect.left)?;
    check_finite("crop.rect.top", rect.top)?;
    check_finite("crop.rect.width", rect.width)?;
    check_finite("crop.rect.height", rect.height)
}

/// The largest rectangle with the aspect ratio `aspect` (width / height in
/// pixels) that is centered on the image center and lies fully inside the
/// rotated image and inside the frame. The result is normalized to the
/// `W x H` frame. `aspect = None` means the source aspect.
///
/// For half sizes `hw = r * hh`, `c = |cos|`, `s = |sin|`, the rect is
/// inside the rotated image when `hw*c + hh*s <= W/2` and
/// `hw*s + hh*c <= H/2`. Both limits are linear in `hh`, so the largest
/// `hh` is the smallest of the limits (the frame limits included).
///
/// # Errors
///
/// - [`DevelopError::Core`] with [`CoreError::InvalidDimensions`] when the
///   source width or height is zero.
/// - [`DevelopError::Settings`] when the angle is not finite, or the aspect
///   is not finite or not greater than 0.
pub fn max_inscribed_rect(
    source_width: u32,
    source_height: u32,
    angle_degrees: f32,
    aspect: Option<f32>,
) -> DevelopResult<NormalizedRect> {
    check_size(source_width, source_height)?;
    check_finite("crop.angle", angle_degrees)?;
    let w = f64::from(source_width);
    let h = f64::from(source_height);
    let r = match aspect {
        None => w / h,
        Some(a) if a.is_finite() && a > 0.0 => f64::from(a),
        Some(_) => {
            return Err(DevelopError::Settings(CoreError::InvalidCrop {
                reason: "the aspect ratio must be a finite number greater than 0",
            }));
        }
    };
    let (sin, cos) = f64::from(angle_degrees).to_radians().sin_cos();
    let (s, c) = (sin.abs(), cos.abs());
    let hh = (w / (2.0 * (r * c + s)))
        .min(h / (2.0 * (r * s + c)))
        .min(h / 2.0)
        .min(w / (2.0 * r));
    let hw = r * hh;
    let width = (2.0 * hw / w).min(1.0);
    let height = (2.0 * hh / h).min(1.0);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "normalized values in 0..=1 fit f32"
    )]
    Ok(NormalizedRect {
        left: ((1.0 - width) / 2.0) as f32,
        top: ((1.0 - height) / 2.0) as f32,
        width: width as f32,
        height: height as f32,
    })
}

/// Returns `true` when `rect` lies fully inside the rotated image and inside
/// the frame. The tolerance is 0.01 source pixels. A rect with a zero size,
/// a negative size, or a value that is not finite gives `false`, as does a
/// zero-size source or an angle that is not finite.
pub fn rect_is_inside(
    source_width: u32,
    source_height: u32,
    angle_degrees: f32,
    rect: &NormalizedRect,
) -> bool {
    if source_width == 0
        || source_height == 0
        || !angle_degrees.is_finite()
        || check_rect_finite(rect).is_err()
        || rect.width <= 0.0
        || rect.height <= 0.0
    {
        return false;
    }
    let w = f64::from(source_width);
    let h = f64::from(source_height);
    let tolerance = INSIDE_TOLERANCE_PIXELS;
    let left = f64::from(rect.left) * w;
    let top = f64::from(rect.top) * h;
    let right = left + f64::from(rect.width) * w;
    let bottom = top + f64::from(rect.height) * h;
    let in_range = |v: f64, max: f64| v >= -tolerance && v <= max + tolerance;
    if !(in_range(left, w) && in_range(right, w) && in_range(top, h) && in_range(bottom, h)) {
        return false;
    }
    // The flips do not change the rotated region, so only the rotation counts.
    let (sin, cos) = f64::from(angle_degrees).to_radians().sin_cos();
    let (cx, cy) = (w / 2.0, h / 2.0);
    [(left, top), (right, top), (left, bottom), (right, bottom)]
        .iter()
        .all(|&(px, py)| {
            let (dx, dy) = (px - cx, py - cy);
            let sx = cos * dx + sin * dy + cx;
            let sy = -sin * dx + cos * dy + cy;
            in_range(sx, w) && in_range(sy, h)
        })
}

#[cfg(test)]
#[allow(
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "test images are small; exact compares are intended"
)]
mod tests {
    use super::*;
    use expose_core::buffer::Rect;

    fn crop_with(rect: NormalizedRect, angle: f32, fh: bool, fv: bool) -> Crop {
        Crop {
            rect,
            angle,
            flip_horizontal: fh,
            flip_vertical: fv,
            ..Crop::default()
        }
    }

    /// Every pixel is unique: value depends on x, y and the channel.
    fn pattern(w: u32, h: u32) -> WorkingImage {
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let (xf, yf) = (x as f32, y as f32);
                px.push([xf + 0.25, yf * 3.0 - 1.5, xf * yf + 2.0, 0.5 + xf * 0.01]);
            }
        }
        WorkingImage::from_pixels(w, h, px).unwrap()
    }

    fn constant(w: u32, h: u32, v: Rgba) -> WorkingImage {
        WorkingImage::from_pixels(w, h, vec![v; (w * h) as usize]).unwrap()
    }

    fn apply(crop: &Crop, img: &WorkingImage) -> WorkingImage {
        Geometry::new(crop, img.width(), img.height())
            .unwrap()
            .apply_cpu(img)
            .unwrap()
    }

    fn bits(img: &WorkingImage) -> Vec<u32> {
        img.pixels().iter().flatten().map(|v| v.to_bits()).collect()
    }

    #[test]
    fn identity_is_bit_exact() {
        for (w, h) in [(8, 6), (7, 5), (1, 1)] {
            let img = pattern(w, h);
            let g = Geometry::new(&Crop::default(), w, h).unwrap();
            assert!(g.is_identity());
            assert!(!g.needs_resample());
            assert_eq!(g.output_size(), (w, h));
            let out = g.apply_cpu(&img).unwrap();
            assert_eq!(bits(&out), bits(&img));
        }
    }

    #[test]
    fn integer_crop_matches_buffer_crop() {
        let img = pattern(10, 8);
        let rect = NormalizedRect {
            left: 0.2,
            top: 0.25,
            width: 0.5,
            height: 0.5,
        };
        let g = Geometry::new(&crop_with(rect, 0.0, false, false), 10, 8).unwrap();
        assert!(!g.is_identity());
        assert_eq!(g.output_size(), (5, 4));
        assert_eq!(g.origin(), (2, 2));
        let out = g.apply_cpu(&img).unwrap();
        let expected = img.crop(Rect::new(2, 2, 5, 4)).unwrap();
        assert_eq!(bits(&out), bits(&expected));
    }

    #[test]
    fn rect_is_rounded_and_clamped_to_frame() {
        let rect = NormalizedRect {
            left: 0.9,
            top: -0.2,
            width: 0.5,
            height: 0.0,
        };
        let g = Geometry::new(&crop_with(rect, 0.0, false, false), 10, 8).unwrap();
        assert_eq!(g.output_size(), (5, 1));
        assert_eq!(g.origin(), (5, 0));
    }

    #[test]
    fn flips_mirror_exactly() {
        let (w, h) = (7, 5);
        let img = pattern(w, h);
        let full = NormalizedRect::FULL;
        let fh = apply(&crop_with(full, 0.0, true, false), &img);
        let fv = apply(&crop_with(full, 0.0, false, true), &img);
        let both = apply(&crop_with(full, 0.0, true, true), &img);
        for y in 0..h {
            for x in 0..w {
                let p = img.pixel(x, y).unwrap();
                assert_eq!(fh.pixel(w - 1 - x, y).unwrap(), p);
                assert_eq!(fv.pixel(x, h - 1 - y).unwrap(), p);
                assert_eq!(both.pixel(w - 1 - x, h - 1 - y).unwrap(), p);
            }
        }
        let twice = apply(&crop_with(full, 0.0, true, false), &fh);
        assert_eq!(bits(&twice), bits(&img));
        let twice_v = apply(&crop_with(full, 0.0, false, true), &fv);
        assert_eq!(bits(&twice_v), bits(&img));
    }

    #[test]
    fn flip_then_crop_takes_frame_region() {
        // Flip first, then crop: the left half of the frame shows the right
        // half of the source, mirrored.
        let img = pattern(8, 4);
        let rect = NormalizedRect {
            left: 0.0,
            top: 0.0,
            width: 0.5,
            height: 1.0,
        };
        let out = apply(&crop_with(rect, 0.0, true, false), &img);
        assert_eq!(out.width(), 4);
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(out.pixel(x, y), img.pixel(7 - x, y));
            }
        }
    }

    #[test]
    fn source_position_matches_hand_value() {
        // W = 4, H = 2, angle 30 degrees, output pixel (0, 0):
        // p = (0.5, 0.5), d = (-1.5, -0.5),
        // s.x = cos*dx + sin*dy + 2 = -1.2990381 - 0.25 + 2 = 0.4509619
        // s.y = -sin*dx + cos*dy + 1 = 0.75 - 0.4330127 + 1 = 1.3169873
        let g = Geometry::new(&crop_with(NormalizedRect::FULL, 30.0, false, false), 4, 2).unwrap();
        assert!(g.needs_resample());
        let (sx, sy) = g.source_position(0, 0);
        assert!((sx - 0.450_961_9).abs() < 1e-5, "{sx}");
        assert!((sy - 1.316_987_3).abs() < 1e-5, "{sy}");
        // With a horizontal flip, s.x becomes W - s.x.
        let gf = Geometry::new(&crop_with(NormalizedRect::FULL, 30.0, true, false), 4, 2).unwrap();
        let (fx, fy) = gf.source_position(0, 0);
        assert!((fx - (4.0 - 0.450_961_9)).abs() < 1e-5, "{fx}");
        assert!((fy - 1.316_987_3).abs() < 1e-5, "{fy}");
    }

    #[test]
    fn center_maps_to_center() {
        for angle in [-45.0, -7.5, 3.0, 45.0] {
            let g =
                Geometry::new(&crop_with(NormalizedRect::FULL, angle, false, false), 7, 5).unwrap();
            let (sx, sy) = g.source_position(3, 2);
            assert!((sx - 3.5).abs() < 1e-5 && (sy - 2.5).abs() < 1e-5);
        }
    }

    #[test]
    fn constant_stays_constant_inside_inscribed_rect() {
        let v = [0.25, 1.5, -0.125, 1.0];
        let img = constant(40, 30, v);
        for angle in [-30.0, -5.0, 2.5, 17.0, 45.0] {
            let rect = max_inscribed_rect(40, 30, angle, None).unwrap();
            let out = apply(&crop_with(rect, angle, false, false), &img);
            for p in out.pixels() {
                for c in 0..4 {
                    assert!((p[c] - v[c]).abs() < 1e-5, "angle {angle}: {p:?}");
                }
            }
        }
    }

    #[test]
    fn linear_gradient_matches_analytic_value() {
        // f(s) = 0.3 * s.x - 0.7 * s.y + 2 on pixel centers.
        let (w, h) = (41, 31);
        let f = |sx: f32, sy: f32| 0.3 * sx - 0.7 * sy + 2.0;
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let v = f(x as f32 + 0.5, y as f32 + 0.5);
                px.push([v, -v, 2.0 * v, 1.0]);
            }
        }
        let img = WorkingImage::from_pixels(w, h, px).unwrap();
        for angle in [-12.0_f32, 7.0, 30.0] {
            let crop = crop_with(NormalizedRect::FULL, angle, false, true);
            let g = Geometry::new(&crop, w, h).unwrap();
            let out = g.apply_cpu(&img).unwrap();
            for (x, y) in [(20, 15), (18, 14), (23, 17)] {
                let (sx, sy) = g.source_position(x, y);
                let expected = f(sx, sy);
                let p = out.pixel(x, y).unwrap();
                assert!(
                    (p[0] - expected).abs() < 1e-4,
                    "{angle}: {p:?} vs {expected}"
                );
                assert!((p[1] + expected).abs() < 1e-4);
                assert!((p[2] - 2.0 * expected).abs() < 2e-4);
                assert!((p[3] - 1.0).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn outside_points_are_transparent() {
        let img = constant(20, 10, [1.0, 1.0, 1.0, 1.0]);
        let g =
            Geometry::new(&crop_with(NormalizedRect::FULL, 30.0, false, false), 20, 10).unwrap();
        let out = g.apply_cpu(&img).unwrap();
        // The frame corners are outside the rotated image.
        let (sx, sy) = g.source_position(0, 0);
        assert!(!(0.0..=20.0).contains(&sx) || !(0.0..=10.0).contains(&sy));
        assert_eq!(out.pixel(0, 0).unwrap(), [0.0; 4]);
        assert_eq!(out.pixel(19, 9).unwrap(), [0.0; 4]);
        // Inside: the weights sum to 1 within f32 rounding.
        let center = out.pixel(10, 5).unwrap();
        assert!(center.iter().all(|v| (v - 1.0).abs() < 1e-6), "{center:?}");
    }

    #[test]
    fn rotation_is_deterministic() {
        let img = pattern(13, 9);
        let crop = crop_with(NormalizedRect::FULL, 11.0, true, false);
        let a = apply(&crop, &img);
        let b = apply(&crop, &img);
        assert_eq!(bits(&a), bits(&b));
    }

    #[test]
    fn one_by_one_source_works() {
        let img = constant(1, 1, [0.5, 2.0, -1.0, 1.0]);
        for (angle, fh) in [(0.0, false), (0.0, true), (20.0, false)] {
            let out = apply(&crop_with(NormalizedRect::FULL, angle, fh, fh), &img);
            assert_eq!(out.width(), 1);
            let p = out.pixel(0, 0).unwrap();
            let expected = [0.5, 2.0, -1.0, 1.0];
            // Exact for angle 0; within float tolerance after rotation.
            for c in 0..4 {
                assert!((p[c] - expected[c]).abs() < 1e-6, "{angle}: {p:?}");
            }
        }
    }

    #[test]
    fn errors() {
        assert!(matches!(
            Geometry::new(&Crop::default(), 0, 5),
            Err(DevelopError::Core(CoreError::InvalidDimensions { .. }))
        ));
        assert!(matches!(
            Geometry::new(
                &crop_with(NormalizedRect::FULL, f32::NAN, false, false),
                4,
                4
            ),
            Err(DevelopError::Settings(CoreError::NonFinite { .. }))
        ));
        let g = Geometry::new(&Crop::default(), 4, 4).unwrap();
        assert!(matches!(
            g.apply_cpu(&pattern(4, 5)),
            Err(DevelopError::SizeMismatch {
                expected: (4, 4),
                actual: (4, 5)
            })
        ));
        assert!(max_inscribed_rect(4, 4, 0.0, Some(0.0)).is_err());
        assert!(max_inscribed_rect(4, 0, 0.0, None).is_err());
    }

    #[test]
    fn inscribed_rect_at_zero_angle() {
        let r = max_inscribed_rect(30, 20, 0.0, None).unwrap();
        assert_eq!(r, NormalizedRect::FULL);
        // A square in a 30 x 20 frame: 20 x 20 pixels, centered.
        let sq = max_inscribed_rect(30, 20, 0.0, Some(1.0)).unwrap();
        assert!((sq.width - 20.0 / 30.0).abs() < 1e-6);
        assert!((sq.height - 1.0).abs() < 1e-6);
        assert!((sq.left - 5.0 / 30.0).abs() < 1e-6);
        assert!(sq.top.abs() < 1e-6);
    }

    #[test]
    fn inscribed_rect_is_inside_and_maximal() {
        let (w, h) = (300, 200);
        for angle in [-45.0_f32, -20.0, -3.0, 1.0, 12.5, 33.0, 45.0] {
            for aspect in [None, Some(1.0), Some(16.0 / 9.0), Some(0.5), Some(4.0)] {
                let r = max_inscribed_rect(w, h, angle, aspect).unwrap();
                assert!(rect_is_inside(w, h, angle, &r), "{angle} {aspect:?} {r:?}");
                let ratio = aspect.unwrap_or(1.5);
                let px_ratio = r.width * w as f32 / (r.height * h as f32);
                assert!((px_ratio - ratio).abs() < 1e-3 * ratio);
                // Grow the rect by 0.5 % about the center: it must not fit.
                let scale = 1.005;
                let (width, height) = (r.width * scale, r.height * scale);
                let (left, top) = (0.5 - width / 2.0, 0.5 - height / 2.0);
                let bigger = NormalizedRect {
                    left,
                    top,
                    width,
                    height,
                };
                assert!(!rect_is_inside(w, h, angle, &bigger), "{angle} {aspect:?}");
            }
        }
    }

    #[test]
    fn inscribed_rect_area_beats_search() {
        // Small search over centered rects of the source aspect.
        let (w, h, angle) = (300_u32, 200_u32, 15.0_f32);
        let r = max_inscribed_rect(w, h, angle, None).unwrap();
        let mut best = 0.0_f32;
        for i in 1..=1000 {
            let k = i as f32 / 1000.0;
            let cand = NormalizedRect {
                left: 0.5 - k / 2.0,
                top: 0.5 - k / 2.0,
                width: k,
                height: k,
            };
            if rect_is_inside(w, h, angle, &cand) {
                best = best.max(k * k);
            }
        }
        let area = r.width * r.height;
        assert!(area >= best - 1e-6);
        assert!(area - best < 0.005);
    }

    #[test]
    fn rect_is_inside_rejects_bad_input() {
        let full = NormalizedRect::FULL;
        assert!(rect_is_inside(10, 10, 0.0, &full));
        assert!(!rect_is_inside(10, 10, 5.0, &full));
        assert!(!rect_is_inside(0, 10, 0.0, &full));
        let empty = NormalizedRect { width: 0.0, ..full };
        assert!(!rect_is_inside(10, 10, 0.0, &empty));
        let outside = NormalizedRect { left: 0.6, ..full };
        assert!(!rect_is_inside(10, 10, 0.0, &outside));
    }
}
