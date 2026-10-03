//! Automatic tone: a luminance histogram and a suggestion for the basic
//! tone sliders.
//!
//! # Algorithm
//!
//! [`auto_tone`] reads three percentiles of `e = log2(Y / 0.18)` from the
//! histogram: low (0.5 %), median (50 %), and high (99.5 %).
//!
//! 1. **Exposure** moves the median to mid gray (`-median`), but it does
//!    not move the high percentile more than 0.25 EV above linear white
//!    (`2.72 - high`). The suggestion is the smaller of the two values,
//!    limited to ±4 EV. Thus the suggestion protects highlights: a low-key
//!    image is not made bright until its highlights clip.
//! 2. **Contrast** compares the range `high - low` with the target range
//!    from black (`e = -6`) to linear white (`e = 2.47`), about 8.5 EV.
//!    A narrow (flat) image gets more contrast; a wide one gets less:
//!    `contrast = 20 * (target - range)`, limited to ±50. This closes about
//!    half of the gap; whites and blacks close the rest.
//! 3. **Whites** and **blacks** move the high and low percentiles (after
//!    exposure and contrast, through the real tone functions) to white and
//!    black. The sliders are limited to ±50, so that the suggestion stays
//!    moderate.
//!
//! Slider values are rounded to whole numbers, and exposure to 0.01 EV.
//! The function is pure: the same histogram gives the same result.
//!
//! Use a histogram of the white-balanced image. A preview size image is
//! sufficient.

use expose_core::buffer::WorkingImage;
use expose_core::color::luminance;
use expose_core::develop::BasicTone;

use crate::error::{DevelopError, DevelopResult};
use crate::tone::{BLACKS_MAX_EV, MID_GRAY, ToneMap, WHITES_MAX_EV};

/// The lowest `e` (EV from mid gray) of the histogram. Lower values go in
/// the first bin.
pub const HISTOGRAM_MIN_EV: f32 = -16.0;
/// The highest `e` of the histogram. Higher values go in the last bin.
pub const HISTOGRAM_MAX_EV: f32 = 8.0;
/// Histogram bins per EV.
pub const HISTOGRAM_BINS_PER_EV: usize = 16;
/// The number of histogram bins.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub const HISTOGRAM_BINS: usize =
    (HISTOGRAM_MAX_EV - HISTOGRAM_MIN_EV) as usize * HISTOGRAM_BINS_PER_EV;

/// The target `e` of white (linear 1.0).
const WHITE_EV: f64 = 2.473_931_188_332_412; // log2(1 / 0.18)
/// The target `e` of black.
const BLACK_EV: f64 = -6.0;
/// Contrast slider units per EV of range difference.
const CONTRAST_PER_EV: f64 = 20.0;
/// The limit of the contrast suggestion.
const CONTRAST_LIMIT: f64 = 50.0;
/// The limit of the whites and blacks suggestions.
const ENDS_LIMIT: f64 = 50.0;
/// The limit of the exposure suggestion, in EV.
const EXPOSURE_LIMIT: f64 = 4.0;
/// Exposure does not move the high percentile more than this above white.
const HIGHLIGHT_MARGIN_EV: f64 = 0.25;
/// Low and high percentiles.
const LOW_PERCENTILE: f64 = 0.005;
const HIGH_PERCENTILE: f64 = 0.995;

/// A histogram of `e = log2(Y / 0.18)` (luminance in EV from mid gray).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LuminanceHistogram {
    counts: Vec<u64>,
    total: u64,
    skipped: u64,
}

impl Default for LuminanceHistogram {
    fn default() -> Self {
        Self::new()
    }
}

impl LuminanceHistogram {
    /// Makes an empty histogram.
    pub fn new() -> Self {
        Self {
            counts: vec![0; HISTOGRAM_BINS],
            total: 0,
            skipped: 0,
        }
    }

    /// Makes the histogram of the luminance of all pixels of `image`.
    pub fn from_image(image: &WorkingImage) -> Self {
        let mut h = Self::new();
        for p in image.pixels() {
            h.add(luminance([p[0], p[1], p[2]]));
        }
        h
    }

    /// Adds one luminance value. Zero and negative values go in the first
    /// bin. NaN values are not counted (see [`Self::skipped`]).
    pub fn add(&mut self, y: f32) {
        if y.is_nan() {
            self.skipped += 1;
            return;
        }
        let e = if y > 0.0 {
            (y / MID_GRAY).log2()
        } else {
            HISTOGRAM_MIN_EV
        };
        #[allow(clippy::cast_precision_loss, reason = "16 is exact")]
        let pos = (e - HISTOGRAM_MIN_EV) * HISTOGRAM_BINS_PER_EV as f32;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "pos is clamped to 0..=HISTOGRAM_BINS - 1"
        )]
        #[allow(clippy::cast_precision_loss, reason = "the bin count is small")]
        let bin = pos.clamp(0.0, (HISTOGRAM_BINS - 1) as f32) as usize;
        self.counts[bin] += 1;
        self.total += 1;
    }

    /// The bin counts.
    pub fn counts(&self) -> &[u64] {
        &self.counts
    }

    /// The number of counted values.
    pub fn total(&self) -> u64 {
        self.total
    }

    /// The number of NaN values that were not counted.
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// Returns the `e` value below which the fraction `p` (in `0..=1`) of
    /// the values are. The value is interpolated linearly in its bin.
    ///
    /// Returns [`DevelopError::EmptyHistogram`] if no value is counted.
    pub fn percentile(&self, p: f64) -> DevelopResult<f64> {
        if self.total == 0 {
            return Err(DevelopError::EmptyHistogram);
        }
        #[allow(
            clippy::cast_precision_loss,
            reason = "a percentile needs no exact count"
        )]
        let target = p.clamp(0.0, 1.0) * self.total as f64;
        #[allow(clippy::cast_precision_loss, reason = "16 is exact in f64")]
        let bin_width = 1.0 / HISTOGRAM_BINS_PER_EV as f64;
        let mut below = 0.0;
        for (i, &count) in self.counts.iter().enumerate() {
            #[allow(
                clippy::cast_precision_loss,
                reason = "a percentile needs no exact count"
            )]
            let c = count as f64;
            if c > 0.0 && below + c >= target {
                let frac = ((target - below) / c).clamp(0.0, 1.0);
                #[allow(clippy::cast_precision_loss, reason = "i is a small bin index")]
                let start = f64::from(HISTOGRAM_MIN_EV) + i as f64 * bin_width;
                return Ok(start + frac * bin_width);
            }
            below += c;
        }
        // `target <= total`, so the loop always returns. This line is for
        // floating-point safety only: the top of the histogram.
        Ok(f64::from(HISTOGRAM_MAX_EV))
    }
}

/// Rounds to `step`.
fn round_to(v: f64, step: f64) -> f32 {
    #[allow(clippy::cast_possible_truncation, reason = "slider values are small")]
    let r = ((v / step).round() * step) as f32;
    r
}

/// Suggests exposure, contrast, whites, and blacks from a histogram. The
/// highlights and shadows suggestions are 0. See the module documentation.
pub fn auto_tone(histogram: &LuminanceHistogram) -> DevelopResult<BasicTone> {
    let low = histogram.percentile(LOW_PERCENTILE)?;
    let median = histogram.percentile(0.5)?;
    let high = histogram.percentile(HIGH_PERCENTILE)?;

    // Move the median to mid gray, but do not move the high percentile more
    // than HIGHLIGHT_MARGIN_EV above white.
    let to_mid_gray = -median;
    let highlight_limit = WHITE_EV + HIGHLIGHT_MARGIN_EV - high;
    let exposure = round_to(
        to_mid_gray
            .min(highlight_limit)
            .clamp(-EXPOSURE_LIMIT, EXPOSURE_LIMIT),
        0.01,
    );
    let shift = f64::from(exposure);

    let range = high - low;
    let contrast = round_to(
        (CONTRAST_PER_EV * ((WHITE_EV - BLACK_EV) - range)).clamp(-CONTRAST_LIMIT, CONTRAST_LIMIT),
        1.0,
    );

    // Where the percentiles are after exposure and contrast.
    let after = ToneMap::from_sliders(&BasicTone {
        contrast,
        ..BasicTone::default()
    });
    let high_after = after.map(high + shift);
    let low_after = after.map(low + shift);

    let whites = round_to(
        (100.0 * (WHITE_EV - high_after) / WHITES_MAX_EV).clamp(-ENDS_LIMIT, ENDS_LIMIT),
        1.0,
    );
    let blacks = round_to(
        (100.0 * (BLACK_EV - low_after) / BLACKS_MAX_EV).clamp(-ENDS_LIMIT, ENDS_LIMIT),
        1.0,
    );

    Ok(BasicTone {
        exposure,
        contrast,
        highlights: 0.0,
        shadows: 0.0,
        whites,
        blacks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A histogram with luminance values spread evenly in EV from
    /// `low` to `high` (EV from mid gray).
    fn spread(low: f32, high: f32) -> LuminanceHistogram {
        let mut h = LuminanceHistogram::new();
        let n = 10_000_u16;
        for i in 0..n {
            let e = low + (high - low) * f32::from(i) / f32::from(n - 1);
            h.add(MID_GRAY * e.exp2());
        }
        h
    }

    #[test]
    fn empty_histogram_is_an_error() {
        let h = LuminanceHistogram::new();
        assert!(matches!(auto_tone(&h), Err(DevelopError::EmptyHistogram)));
        let mut nan = LuminanceHistogram::new();
        nan.add(f32::NAN);
        assert_eq!(nan.skipped(), 1);
        assert!(auto_tone(&nan).is_err());
    }

    #[test]
    fn percentiles_of_even_spread() {
        let h = spread(-4.0, 2.0);
        assert!((h.percentile(0.5).unwrap() - -1.0).abs() < 0.05);
        assert!((h.percentile(0.0).unwrap() - -4.0).abs() < 0.07);
        assert!((h.percentile(1.0).unwrap() - 2.0).abs() < 0.07);
    }

    #[test]
    fn dark_image_gets_positive_exposure() {
        // Median -4.5, but the highlight limit (2.724 + 1.035) is lower.
        let t = auto_tone(&spread(-8.0, -1.0)).unwrap();
        assert!((t.exposure - 3.76).abs() < 0.05, "{t:?}");
        let t = auto_tone(&spread(-12.0, -6.0)).unwrap();
        assert!((t.exposure - 4.0).abs() < 0.01, "limited to 4 EV: {t:?}");
        let t = auto_tone(&spread(-5.0, -1.0)).unwrap();
        assert!((t.exposure - 3.0).abs() < 0.05, "{t:?}");
    }

    #[test]
    fn bright_image_gets_negative_exposure() {
        let t = auto_tone(&spread(-1.0, 4.0)).unwrap();
        assert!((t.exposure + 1.5).abs() < 0.05, "{t:?}");
    }

    #[test]
    fn flat_image_gets_more_contrast_and_wide_image_less() {
        let flat = auto_tone(&spread(-1.5, 1.5)).unwrap();
        assert!(flat.contrast > 40.0, "{flat:?}");
        assert!(flat.whites > 0.0 && flat.blacks < 0.0, "{flat:?}");
        let wide = auto_tone(&spread(-12.0, 6.0)).unwrap();
        assert!(wide.contrast < -40.0, "{wide:?}");
        // Exposure protects the highlights; blacks lift the deep shadows.
        assert!(wide.exposure < -3.0 && wide.blacks > 0.0, "{wide:?}");
    }

    #[test]
    fn exposure_protects_highlights() {
        // Median far below mid gray, but highlights already near white.
        let t = auto_tone(&spread(-9.0, 2.0)).unwrap();
        // high = 1.945, so the limit is 2.474 + 0.25 - 1.945 = 0.78.
        assert!((t.exposure - 0.78).abs() < 0.05, "{t:?}");
    }

    #[test]
    fn well_exposed_image_needs_little_change() {
        let t = auto_tone(&spread(-6.0, 2.47)).unwrap();
        assert!(t.exposure.abs() <= 0.35, "{t:?}");
        assert!(t.contrast.abs() <= 10.0, "{t:?}");
        assert!(t.whites.abs() <= 25.0 && t.blacks.abs() <= 25.0, "{t:?}");
    }

    #[test]
    fn result_is_deterministic_and_valid() {
        let h = spread(-3.3, 0.7);
        let a = auto_tone(&h).unwrap();
        assert_eq!(a, auto_tone(&h).unwrap());
        let mut settings = expose_core::develop::DevelopSettings {
            tone: a.clone(),
            ..Default::default()
        };
        settings.validate().unwrap();
        assert_eq!(settings.tone, a, "suggestion is inside the slider ranges");
    }
}
