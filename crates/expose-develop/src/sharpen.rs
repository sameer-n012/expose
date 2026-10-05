//! Detail stage: sharpening (CPU reference).
//!
//! This module is the reference for the sharpening stage. A GPU version must
//! give the same result within float tolerance. The code is single-threaded
//! and deterministic, so it also builds for WebAssembly.
//!
//! # Why luminance in an encoded scale
//!
//! - The stage changes luminance only. It scales the red, green, and blue
//!   values of a pixel by the same factor, so the hue and the saturation stay
//!   the same. Thus the stage adds no color fringes.
//! - The stage finds the detail in the sRGB-encoded luminance `E`, not in the
//!   linear luminance `Y`. The encoded scale is approximately perceptual, so a
//!   halo of the same size is equally visible in dark and in bright tones.
//!   The sRGB encode continues above 1 with the same power formula, so
//!   scene-referred highlights (`Y > 1`) are sharpened too.
//!
//! # Passes
//!
//! The algorithm has four passes. Each pass maps to one GPU kernel.
//!
//! 1. **Encode (per pixel).** `Y = luminance(rgb)`,
//!    `E = srgb_encode(max(Y, 0))`.
//! 2. **Horizontal blur.** A separable Gaussian blur of `E` along x. The
//!    weights come from [`Sharpener::weights`]. Pixels outside the image take
//!    the value of the nearest edge pixel (clamp to edge).
//! 3. **Vertical blur.** The same blur along y. The result is `B`.
//! 4. **Combine (per pixel).**
//!    - Detail: `D = E - B`.
//!    - Halo suppression: with `t = detail / 100`,
//!      `D' = t * D + (1 - t) * D / (1 + |D| / K)` and `K` =
//!      [`HALO_SOFT_LIMIT`]. A low `detail` value limits large differences
//!      (strong edges) softly, so these edges get smaller halos.
//!    - Edge mask: with `m = masking / 100`, `G` is the gradient magnitude of
//!      `B` (central differences, clamp to edge),
//!      `lo = m * `[`MASK_THRESHOLD_SCALE`], `hi = lo + `[`MASK_RAMP_WIDTH`],
//!      and `M = smoothstep(lo, hi, G)`. When `masking == 0`, `M = 1` and the
//!      gradient is not calculated.
//!    - Gain: `g = amount / 100 * `[`GAIN_SCALE`].
//!    - `E' = E + g * M * D'`, `Y' = srgb_decode(max(E', 0))`.
//!    - When `Y > `[`LUMINANCE_EPSILON`], `rgb' = rgb * (Y' / Y)`. Otherwise
//!      the pixel does not change. Alpha does not change. The stage does not
//!      clip values above 1.
//!
//! # Radius and scale
//!
//! The blur sigma is `radius * pixel_scale`. `pixel_scale` is the number of
//! output pixels for each source pixel, so a downscaled preview uses a smaller
//! sigma and looks like the full-size result. When the sigma is less than
//! [`MIN_SIGMA`], the effect is too small to see and the stage is a no-op. The
//! sigma is limited to [`MAX_SIGMA`], so that the kernel fits in
//! [`MAX_TAPS`] weights.

use std::ops::RangeInclusive;

use expose_core::CoreError;
use expose_core::buffer::{Rgba, WorkingImage};
use expose_core::color::{TransferFunction, luminance};
use expose_core::develop::{Sharpening, ranges};

use crate::error::{DevelopError, DevelopResult};

/// Maximum number of blur weights on one side of the kernel, including the
/// center weight. The kernel half-width is at most `MAX_TAPS - 1` (15), so a
/// GPU kernel can keep the weights in a fixed-size uniform array.
pub const MAX_TAPS: usize = 16;

/// Smallest blur sigma (in output pixels) that has a visible effect. A
/// smaller sigma makes the stage a no-op.
pub const MIN_SIGMA: f32 = 0.25;

/// Largest blur sigma (in output pixels). The half-width is `ceil(3 * sigma)`,
/// so this value gives a half-width of `MAX_TAPS - 1`.
/// The test `max_sigma_fits_max_taps` checks this relation.
pub const MAX_SIGMA: f32 = 5.0;

/// Soft limit `K` for halo suppression, in encoded units. Differences much
/// larger than `K` are compressed when `detail` is low.
pub const HALO_SOFT_LIMIT: f32 = 0.05;

/// Scale from `masking / 100` to the low gradient threshold `lo` of the edge
/// mask, in encoded units per pixel.
pub const MASK_THRESHOLD_SCALE: f32 = 0.1;

/// Width of the smooth ramp of the edge mask (`hi - lo`), in encoded units
/// per pixel.
pub const MASK_RAMP_WIDTH: f32 = 0.02;

/// Scale from `amount / 100` to the gain `g`. An amount of 150 gives a gain
/// of 2.25.
pub const GAIN_SCALE: f32 = 1.5;

/// Pixels with a luminance at or below this value do not change, because the
/// ratio `Y' / Y` is not stable for them.
pub const LUMINANCE_EPSILON: f32 = 1e-6;

/// Transfer function that gives the perceptual scale of the stage.
const SCALE: TransferFunction = TransferFunction::Srgb;

/// Prepared parameters of the sharpening stage for one output scale.
#[derive(Debug, Clone, PartialEq)]
pub struct Sharpener {
    /// `amount / 100 * GAIN_SCALE`.
    gain: f32,
    /// `detail / 100`, in `0..=1`.
    detail: f32,
    /// `masking / 100`, in `0..=1`.
    masking: f32,
    /// Blur sigma in output pixels, in `MIN_SIGMA..=MAX_SIGMA`.
    sigma: f32,
    /// Normalized Gaussian weights for offsets `0..=half_width`.
    weights: Vec<f32>,
}

impl Sharpener {
    /// Prepares the stage for `settings` at `pixel_scale` (output pixels for
    /// each source pixel; 1.0 for full size).
    ///
    /// Returns `Ok(None)` when the stage is a no-op at this scale: the amount
    /// is 0, or the sigma is less than [`MIN_SIGMA`]. Values outside the
    /// ranges of `expose_core::develop::ranges` are clamped, as
    /// `DevelopSettings::validate` does.
    ///
    /// # Errors
    ///
    /// - [`DevelopError::Settings`] when a settings value is not finite.
    /// - [`DevelopError::Core`] when `pixel_scale` is not finite or is not
    ///   more than 0.
    pub fn new(settings: &Sharpening, pixel_scale: f32) -> DevelopResult<Option<Self>> {
        let amount = checked(
            "sharpening.amount",
            settings.amount,
            &ranges::SHARPEN_AMOUNT,
        )?;
        let radius = checked(
            "sharpening.radius",
            settings.radius,
            &ranges::SHARPEN_RADIUS,
        )?;
        let detail = checked(
            "sharpening.detail",
            settings.detail,
            &ranges::SHARPEN_DETAIL,
        )?;
        let masking = checked(
            "sharpening.masking",
            settings.masking,
            &ranges::SHARPEN_MASKING,
        )?;
        if !pixel_scale.is_finite() {
            return Err(DevelopError::Core(CoreError::NonFinite {
                name: "pixel_scale",
            }));
        }
        if pixel_scale <= 0.0 {
            return Err(DevelopError::Core(CoreError::OutOfRange {
                name: "pixel_scale",
                value: f64::from(pixel_scale),
                min: f64::from(f32::MIN_POSITIVE),
                max: f64::from(f32::MAX),
            }));
        }
        if amount == 0.0 {
            return Ok(None);
        }
        let sigma = radius * pixel_scale;
        if sigma < MIN_SIGMA {
            return Ok(None);
        }
        let sigma = sigma.min(MAX_SIGMA);
        Ok(Some(Self {
            gain: amount / 100.0 * GAIN_SCALE,
            detail: detail / 100.0,
            masking: masking / 100.0,
            sigma,
            weights: gaussian_weights(sigma),
        }))
    }

    /// Blur sigma in output pixels (after the clamp to [`MAX_SIGMA`]).
    #[must_use]
    pub fn sigma(&self) -> f32 {
        self.sigma
    }

    /// Gaussian weights for the offsets `0..=half_width`, center first. Use
    /// each weight after the first for the offsets `+i` and `-i`. The sum
    /// `w[0] + 2 * (w[1] + ... + w[n])` is 1. The length is at most
    /// [`MAX_TAPS`].
    #[must_use]
    pub fn weights(&self) -> &[f32] {
        &self.weights
    }

    /// Kernel half-width (`weights().len() - 1`), at least 1.
    #[must_use]
    pub fn half_width(&self) -> usize {
        self.weights.len().saturating_sub(1)
    }

    /// Gain `g = amount / 100 * GAIN_SCALE`.
    #[must_use]
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Detail factor `t = detail / 100`, in `0..=1`.
    #[must_use]
    pub fn detail(&self) -> f32 {
        self.detail
    }

    /// Masking factor `m = masking / 100`, in `0..=1`.
    #[must_use]
    pub fn masking(&self) -> f32 {
        self.masking
    }

    /// Applies the stage to `image` in place, with the four passes of the
    /// module documentation.
    ///
    /// # Errors
    ///
    /// The current implementation does not fail. The `Result` lets later
    /// versions report buffer errors without an API change.
    pub fn apply_cpu(&self, image: &mut WorkingImage) -> DevelopResult<()> {
        let width = image.width() as usize;
        let height = image.height() as usize;
        let encoded = encode_pass(image.pixels());
        let horizontal = blur_horizontal(&encoded, width, height, &self.weights);
        let blurred = blur_vertical(&horizontal, width, height, &self.weights);
        drop(horizontal);
        self.combine_pass(image.pixels_mut(), &encoded, &blurred, width, height);
        Ok(())
    }

    /// Pass 4: detail, halo suppression, edge mask, and combine.
    fn combine_pass(
        &self,
        pixels: &mut [Rgba],
        encoded: &[f32],
        blurred: &[f32],
        width: usize,
        height: usize,
    ) {
        let use_mask = self.masking > 0.0;
        let lo = self.masking * MASK_THRESHOLD_SCALE;
        let hi = lo + MASK_RAMP_WIDTH;
        for y in 0..height {
            for x in 0..width {
                let index = y * width + x;
                let pixel = &mut pixels[index];
                let lum = luminance([pixel[0], pixel[1], pixel[2]]);
                if lum.is_nan() || lum <= LUMINANCE_EPSILON {
                    continue;
                }
                let e = encoded[index];
                let diff = e - blurred[index];
                let soft = diff / (1.0 + diff.abs() / HALO_SOFT_LIMIT);
                let limited = self.detail * diff + (1.0 - self.detail) * soft;
                let mask = if use_mask {
                    smoothstep(lo, hi, gradient(blurred, width, height, x, y))
                } else {
                    1.0
                };
                let sharpened = e + self.gain * mask * limited;
                let new_lum = SCALE.decode(sharpened.max(0.0));
                let ratio = new_lum / lum;
                pixel[0] *= ratio;
                pixel[1] *= ratio;
                pixel[2] *= ratio;
            }
        }
    }
}

/// Clamps a settings value to `range`, or returns an error when it is not
/// finite.
fn checked(name: &'static str, value: f32, range: &RangeInclusive<f32>) -> DevelopResult<f32> {
    if value.is_finite() {
        Ok(value.clamp(*range.start(), *range.end()))
    } else {
        Err(DevelopError::Settings(CoreError::NonFinite { name }))
    }
}

/// Normalized Gaussian weights for offsets `0..=ceil(3 * sigma)` (at least 1,
/// at most `MAX_TAPS - 1`).
fn gaussian_weights(sigma: f32) -> Vec<f32> {
    // `sigma` is finite and positive here, so the cast is exact and small.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "sigma is in MIN_SIGMA..=MAX_SIGMA, so 3 * sigma is in 0..=15"
    )]
    let half_width = ((3.0 * sigma).ceil() as usize).clamp(1, MAX_TAPS - 1);
    let denom = 2.0 * sigma * sigma;
    let mut weights: Vec<f32> = (0..=half_width)
        .map(|i| {
            #[allow(
                clippy::cast_precision_loss,
                reason = "i is at most MAX_TAPS - 1, so the cast is exact"
            )]
            let offset = i as f32;
            (-(offset * offset) / denom).exp()
        })
        .collect();
    let sum = weights[0] + 2.0 * weights[1..].iter().sum::<f32>();
    for w in &mut weights {
        *w /= sum;
    }
    weights
}

/// Pass 1: sRGB-encoded luminance `E = encode(max(Y, 0))` for each pixel.
fn encode_pass(pixels: &[Rgba]) -> Vec<f32> {
    pixels
        .iter()
        .map(|p| SCALE.encode(luminance([p[0], p[1], p[2]]).max(0.0)))
        .collect()
}

/// Pass 2: Gaussian blur along x, clamp to edge.
fn blur_horizontal(src: &[f32], width: usize, height: usize, weights: &[f32]) -> Vec<f32> {
    let mut out = vec![0.0; src.len()];
    let last = width.saturating_sub(1);
    for y in 0..height {
        let row = &src[y * width..(y + 1) * width];
        let out_row = &mut out[y * width..(y + 1) * width];
        for (x, value) in out_row.iter_mut().enumerate() {
            let mut acc = weights[0] * row[x];
            for (i, &w) in weights.iter().enumerate().skip(1) {
                let left = x.saturating_sub(i);
                let right = (x + i).min(last);
                acc += w * (row[left] + row[right]);
            }
            *value = acc;
        }
    }
    out
}

/// Pass 3: Gaussian blur along y, clamp to edge.
fn blur_vertical(src: &[f32], width: usize, height: usize, weights: &[f32]) -> Vec<f32> {
    let mut out = vec![0.0; src.len()];
    let last = height.saturating_sub(1);
    for y in 0..height {
        for x in 0..width {
            let mut acc = weights[0] * src[y * width + x];
            for (i, &w) in weights.iter().enumerate().skip(1) {
                let up = y.saturating_sub(i);
                let down = (y + i).min(last);
                acc += w * (src[up * width + x] + src[down * width + x]);
            }
            out[y * width + x] = acc;
        }
    }
    out
}

/// Gradient magnitude of `values` at `(x, y)`, with central differences and
/// clamp to edge. The step is half the difference of the two neighbours.
fn gradient(values: &[f32], width: usize, height: usize, x: usize, y: usize) -> f32 {
    let left = x.saturating_sub(1);
    let right = (x + 1).min(width - 1);
    let up = y.saturating_sub(1);
    let down = (y + 1).min(height - 1);
    let gx = 0.5 * (values[y * width + right] - values[y * width + left]);
    let gy = 0.5 * (values[down * width + x] - values[up * width + x]);
    gx.hypot(gy)
}

/// Hermite smoothstep from `lo` to `hi` (`hi > lo`).
fn smoothstep(lo: f32, hi: f32, value: f32) -> f32 {
    let t = ((value - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(amount: f32, radius: f32, detail: f32, masking: f32) -> Sharpening {
        Sharpening {
            amount,
            radius,
            detail,
            masking,
        }
    }

    fn sharpener(amount: f32, detail: f32, masking: f32) -> Sharpener {
        match Sharpener::new(&settings(amount, 1.0, detail, masking), 1.0) {
            Ok(Some(s)) => s,
            other => panic!("expected a sharpener, got {other:?}"),
        }
    }

    fn gray(v: f32) -> Rgba {
        [v, v, v, 1.0]
    }

    /// A 1-row image with gray values from `values`.
    fn row_image(values: &[f32]) -> WorkingImage {
        let pixels = values.iter().map(|&v| gray(v)).collect();
        let width = u32::try_from(values.len()).unwrap();
        WorkingImage::from_pixels(width, 1, pixels).unwrap()
    }

    /// A step from `dark` to `bright` at the middle of a 1-row image.
    fn step(dark: f32, bright: f32) -> WorkingImage {
        let values: Vec<f32> = (0..32)
            .map(|i| if i < 16 { dark } else { bright })
            .collect();
        row_image(&values)
    }

    fn sharpened(image: &WorkingImage, s: &Sharpener) -> WorkingImage {
        let mut out = image.clone();
        s.apply_cpu(&mut out).unwrap();
        out
    }

    fn green(image: &WorkingImage) -> Vec<f32> {
        image.pixels().iter().map(|p| p[1]).collect()
    }

    fn max_of(values: &[f32]) -> f32 {
        values.iter().copied().fold(f32::MIN, f32::max)
    }

    fn min_of(values: &[f32]) -> f32 {
        values.iter().copied().fold(f32::MAX, f32::min)
    }

    #[test]
    fn amount_zero_is_none() {
        let r = Sharpener::new(&settings(0.0, 1.0, 25.0, 0.0), 1.0).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn tiny_sigma_is_none() {
        let r = Sharpener::new(&settings(50.0, 0.5, 25.0, 0.0), 0.4).unwrap();
        assert!(r.is_none());
        let r = Sharpener::new(&settings(50.0, 0.5, 25.0, 0.0), 0.5).unwrap();
        assert!(r.is_some());
    }

    #[test]
    fn bad_input_is_an_error() {
        let s = settings(50.0, 1.0, 25.0, 0.0);
        assert!(Sharpener::new(&s, 0.0).is_err());
        assert!(Sharpener::new(&s, -1.0).is_err());
        assert!(Sharpener::new(&s, f32::NAN).is_err());
        assert!(Sharpener::new(&s, f32::INFINITY).is_err());
        let s = settings(f32::NAN, 1.0, 25.0, 0.0);
        assert!(matches!(
            Sharpener::new(&s, 1.0),
            Err(DevelopError::Settings(_))
        ));
        let s = settings(50.0, 1.0, f32::INFINITY, 0.0);
        assert!(Sharpener::new(&s, 1.0).is_err());
    }

    #[test]
    fn out_of_range_settings_are_clamped() {
        let s = Sharpener::new(&settings(500.0, 10.0, 200.0, -5.0), 1.0)
            .unwrap()
            .unwrap();
        assert!((s.gain() - 2.25).abs() < 1e-6);
        assert!((s.sigma() - 3.0).abs() < 1e-6);
        assert!((s.detail() - 1.0).abs() < 1e-6);
        assert!(s.masking().abs() < 1e-6);
    }

    #[test]
    fn weights_sum_to_one_and_are_bounded() {
        for (radius, scale) in [(0.5, 1.0), (1.0, 1.0), (3.0, 1.0), (1.3, 0.37)] {
            let Some(s) = Sharpener::new(&settings(50.0, radius, 25.0, 0.0), scale).unwrap() else {
                continue;
            };
            let w = s.weights();
            let sum = w[0] + 2.0 * w[1..].iter().sum::<f32>();
            assert!((sum - 1.0).abs() < 1e-6, "sum {sum}");
            assert!(w.len() <= MAX_TAPS);
            assert!(s.half_width() >= 1);
            assert!(w.windows(2).all(|p| p[0] >= p[1]), "weights must decrease");
        }
        let s = Sharpener::new(&settings(50.0, 3.0, 25.0, 0.0), 1.0)
            .unwrap()
            .unwrap();
        assert_eq!(s.half_width(), 9);
        assert!(s.half_width() < MAX_TAPS);
    }

    #[test]
    fn max_sigma_fits_max_taps() {
        assert_eq!(gaussian_weights(MAX_SIGMA).len(), MAX_TAPS);
        let w = gaussian_weights(MAX_SIGMA + 1.0);
        assert_eq!(w.len(), MAX_TAPS, "the half-width is capped");
    }

    #[test]
    fn large_scale_clamps_sigma() {
        let s = Sharpener::new(&settings(50.0, 3.0, 25.0, 0.0), 8.0)
            .unwrap()
            .unwrap();
        assert!((s.sigma() - MAX_SIGMA).abs() < 1e-6);
        assert_eq!(s.weights().len(), MAX_TAPS);
    }

    #[test]
    fn symmetric_input_gives_symmetric_output() {
        let values: Vec<f32> = (0..21)
            .map(|i: i32| if (i - 10).abs() < 3 { 0.8 } else { 0.1 })
            .collect();
        let out = green(&sharpened(
            &row_image(&values),
            &sharpener(100.0, 25.0, 0.0),
        ));
        for i in 0..out.len() {
            assert!((out[i] - out[out.len() - 1 - i]).abs() < 1e-6);
        }
    }

    #[test]
    fn constant_image_is_unchanged() {
        let image = WorkingImage::new_filled(7, 5, [0.3, 0.2, 0.1, 1.0]).unwrap();
        for masking in [0.0, 50.0] {
            let out = sharpened(&image, &sharpener(150.0, 25.0, masking));
            for (a, b) in image.pixels().iter().zip(out.pixels()) {
                for c in 0..4 {
                    assert!((a[c] - b[c]).abs() < 1e-6);
                }
            }
        }
    }

    #[test]
    fn step_edge_overshoots_and_grows_with_amount() {
        let image = step(0.1, 0.5);
        let low = green(&sharpened(&image, &sharpener(50.0, 25.0, 0.0)));
        let high = green(&sharpened(&image, &sharpener(150.0, 25.0, 0.0)));
        assert!(max_of(&low[16..]) > 0.5);
        assert!(min_of(&low[..16]) < 0.1);
        assert!(max_of(&high[16..]) > max_of(&low[16..]));
        assert!(min_of(&high[..16]) < min_of(&low[..16]));
        // Far from the edge, nothing changes.
        assert!((low[0] - 0.1).abs() < 1e-6);
        assert!((low[31] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn low_detail_reduces_overshoot_on_strong_edge() {
        let image = step(0.02, 0.8);
        let low = green(&sharpened(&image, &sharpener(100.0, 0.0, 0.0)));
        let high = green(&sharpened(&image, &sharpener(100.0, 100.0, 0.0)));
        assert!(max_of(&low[16..]) > 0.8);
        assert!(max_of(&low[16..]) < max_of(&high[16..]));
    }

    #[test]
    fn masking_protects_smooth_areas_but_not_edges() {
        // Smooth curved ramp: low gradient, but the blur still changes it.
        let ramp: Vec<f32> = (0..32_u8)
            .map(|i| {
                let t = f32::from(i) / 31.0;
                0.1 + 0.3 * t * t
            })
            .collect();
        let ramp_image = row_image(&ramp);
        let unmasked = green(&sharpened(&ramp_image, &sharpener(150.0, 100.0, 0.0)));
        let masked = green(&sharpened(&ramp_image, &sharpener(150.0, 100.0, 100.0)));
        let changed = ramp
            .iter()
            .zip(&unmasked)
            .any(|(a, b)| (a - b).abs() > 1e-4);
        assert!(changed, "without masking the ramp must change");
        for (a, b) in ramp.iter().zip(&masked) {
            assert!((a - b).abs() < 1e-5, "{a} -> {b}");
        }
        let edge = green(&sharpened(
            &step(0.05, 0.6),
            &sharpener(150.0, 100.0, 100.0),
        ));
        assert!(max_of(&edge[16..]) > 0.6);
        assert!(min_of(&edge[..16]) < 0.05);
    }

    #[test]
    fn hue_and_alpha_are_preserved() {
        let mut pixels = vec![[0.05, 0.02, 0.01, 0.5]; 16];
        pixels.extend(vec![[0.8, 0.4, 0.2, 0.25]; 16]);
        let image = WorkingImage::from_pixels(32, 1, pixels).unwrap();
        let out = sharpened(&image, &sharpener(150.0, 25.0, 0.0));
        for (a, b) in image.pixels().iter().zip(out.pixels()) {
            assert_eq!(a[3].to_bits(), b[3].to_bits());
            let ra = [a[0] / a[1], a[2] / a[1]];
            let rb = [b[0] / b[1], b[2] / b[1]];
            assert!((ra[0] - rb[0]).abs() < 1e-5 && (ra[1] - rb[1]).abs() < 1e-5);
        }
        assert!(out.pixels()[16][0] > 0.8, "the edge must sharpen");
    }

    #[test]
    fn dark_and_negative_pixels_are_unchanged_and_no_nan() {
        let pixels = vec![
            [0.0, 0.0, 0.0, 1.0],
            [-0.2, -0.1, -0.3, 1.0],
            [0.5, -0.1, 0.0, 1.0],
            [4.0, 3.0, 5.0, 1.0],
            [20.0, 20.0, 20.0, 1.0],
            [-1.0, 2.0, -0.5, 1.0],
            [0.0, 0.0, 0.0, 1.0],
            [8.0, 0.1, 0.1, 1.0],
        ];
        let image = WorkingImage::from_pixels(8, 1, pixels).unwrap();
        let out = sharpened(&image, &sharpener(150.0, 100.0, 0.0));
        for (a, b) in image.pixels().iter().zip(out.pixels()) {
            assert!(b.iter().all(|v| v.is_finite()), "{b:?}");
            if luminance([a[0], a[1], a[2]]) <= LUMINANCE_EPSILON {
                assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits));
            }
        }
        let bits = |p: Rgba| p.map(f32::to_bits);
        assert_eq!(bits(out.pixels()[0]), bits([0.0, 0.0, 0.0, 1.0]));
        assert_eq!(bits(out.pixels()[1]), bits([-0.2, -0.1, -0.3, 1.0]));
    }

    #[test]
    fn output_is_deterministic() {
        let pixels: Vec<Rgba> = (0..64_u16)
            .map(|i| {
                let v = f32::from((i * 37) % 17) / 8.0;
                [v, v * 0.5, 0.3, 1.0]
            })
            .collect();
        let image = WorkingImage::from_pixels(8, 8, pixels).unwrap();
        let s = sharpener(120.0, 40.0, 30.0);
        let a = sharpened(&image, &s);
        let b = sharpened(&image, &s);
        let bits = |img: &WorkingImage| -> Vec<u32> {
            img.as_slice().iter().map(|v| v.to_bits()).collect()
        };
        assert_eq!(bits(&a), bits(&b));
    }

    #[test]
    fn tiny_images_work() {
        let s = sharpener(150.0, 25.0, 50.0);
        let one = WorkingImage::new_filled(1, 1, [0.4, 0.3, 0.2, 1.0]).unwrap();
        let out = sharpened(&one, &s);
        for c in 0..4 {
            assert!((one.pixels()[0][c] - out.pixels()[0][c]).abs() < 1e-6);
        }
        let column =
            WorkingImage::from_pixels(1, 4, vec![gray(0.1), gray(0.1), gray(0.7), gray(0.7)])
                .unwrap();
        let out = sharpened(&column, &s);
        assert!(out.pixels().iter().all(|p| p.iter().all(|v| v.is_finite())));
        assert!(out.pixels()[2][1] > 0.7);
        let row = step(0.1, 0.7);
        assert!(green(&sharpened(&row, &s)).iter().all(|v| v.is_finite()));
    }
}
