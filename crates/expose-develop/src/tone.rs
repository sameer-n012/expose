//! Highlights, shadows, whites, blacks, and contrast: a global tone map on
//! luminance.
//!
//! # Scale
//!
//! All tone functions use the log scale `e = log2(Y / 0.18)`: the distance
//! from mid gray in EV (stops). On this scale, one slider step has a similar
//! visual effect in all parts of the range. Linear white (`Y = 1`) is at
//! `e = 2.47`.
//!
//! # Luminance ratio
//!
//! Each function maps `e` to a new value `g(e)`. The pipeline multiplies
//! R, G, and B by the same ratio `2^(g(e) - e)`. Thus the ratio of the
//! channels (hue and saturation in the working space) does not change.
//! Because the ratio is applied to all channels, a sequence of these stages
//! is the same as one function: the composition of the maps. The pipeline
//! uses this, and keeps one look-up table (LUT) for all of them.
//!
//! # Zone functions
//!
//! Each function is `h(e) = e + a * mask(e)`, with `a` in EV:
//!
//! | Slider     | Mask                                     | Max `a` (at ±100) |
//! | ---------- | ---------------------------------------- | ----------------- |
//! | highlights | rising logistic step, center +1.75, scale 0.7 | 1.25 EV      |
//! | shadows    | Gaussian bump, center -2.5, sigma 1.25   | 1.25 EV           |
//! | whites     | rising logistic step, center +2.5, scale 0.6  | 1.5 EV       |
//! | blacks     | falling logistic step, center -5.0, scale 0.75 | 1.5 EV      |
//!
//! - Highlights and whites use a step (not a bump), so that they also move
//!   scene values above 1.0. This gives highlight recovery.
//! - Shadows uses a bump, so that deep blacks stay where they are.
//! - The amounts are limited so that `|a * mask'(e)| <= 0.63`. Thus the slope
//!   of each function is at least 0.37 and each function is strictly
//!   increasing. The pipeline applies the functions one after the other
//!   (highlights, shadows, whites, blacks, then contrast). A composition of
//!   increasing functions is increasing, so the full map is monotonic for
//!   all slider values.
//!
//! # Contrast
//!
//! `h(e) = e + k * W * tanh(e / W)` with `W = 2.5` EV and
//! `k = contrast / 200` (so `k` is in `-0.5..=0.5`). This is an S-curve in
//! log space with its pivot at mid gray (`e = 0`, `Y = 0.18`, a fixed
//! point). The slope at the pivot is `1 + k`. Far from the pivot, the slope
//! goes back to 1, so the shift is limited to `±k * W` EV.
//!
//! # Limits
//!
//! - This is a global (per pixel) tone map. A local, edge-aware version
//!   (which keeps local contrast) is a later item.
//! - The functions use luminance. A saturated dark color (for example pure
//!   blue, `Y = 0.06`) is treated as a shadow.
//! - Luminance below `0.18 * 2^-20` (including zero and negative values)
//!   uses the ratio at `e = -20`. Luminance above `0.18 * 2^12` uses the
//!   ratio at `e = 12`. All masks are flat there, so the map is continuous.

use expose_core::develop::BasicTone;

/// Mid gray in linear light: the pivot of the tone functions.
pub const MID_GRAY: f32 = 0.18;
/// The lowest `e` (EV from mid gray) in the tone LUT.
pub const TONE_LUT_MIN_EV: f32 = -20.0;
/// The highest `e` (EV from mid gray) in the tone LUT.
pub const TONE_LUT_MAX_EV: f32 = 12.0;
/// The number of samples in the tone LUT (about 1/64 EV apart). It is a
/// multiple of 4, so that the GPU can pack it in `vec4` values.
pub const TONE_LUT_SIZE: usize = 2048;

/// Max shift of the highlights slider, in EV.
pub const HIGHLIGHTS_MAX_EV: f64 = 1.25;
/// Max shift of the shadows slider, in EV.
pub const SHADOWS_MAX_EV: f64 = 1.25;
/// Max shift of the whites slider, in EV.
pub const WHITES_MAX_EV: f64 = 1.5;
/// Max shift of the blacks slider, in EV.
pub const BLACKS_MAX_EV: f64 = 1.5;
/// Width `W` of the contrast S-curve, in EV.
pub const CONTRAST_WIDTH_EV: f64 = 2.5;
/// The contrast slope factor `k` at a slider value of 100.
pub const CONTRAST_MAX_SLOPE: f64 = 0.5;

const HIGHLIGHTS_CENTER: f64 = 1.75;
const HIGHLIGHTS_SCALE: f64 = 0.7;
const SHADOWS_CENTER: f64 = -2.5;
const SHADOWS_SIGMA: f64 = 1.25;
const WHITES_CENTER: f64 = 2.5;
const WHITES_SCALE: f64 = 0.6;
const BLACKS_CENTER: f64 = -5.0;
const BLACKS_SCALE: f64 = 0.75;

/// The index of the last LUT sample, as `f32`.
#[allow(clippy::cast_precision_loss, reason = "2047 is exact in f32")]
const LAST_INDEX_F32: f32 = (TONE_LUT_SIZE - 1) as f32;

/// LUT samples per EV. The CPU and the GPU both multiply by this value, so
/// that both find the same LUT position.
pub(crate) const TONE_LUT_SAMPLES_PER_EV: f32 =
    LAST_INDEX_F32 / (TONE_LUT_MAX_EV - TONE_LUT_MIN_EV);

/// The lowest luminance that the tone LUT resolves: `0.18 * 2^-20`. Lower
/// values (also zero and negative values) use this value.
pub(crate) const LUMINANCE_FLOOR: f32 = MID_GRAY / 1_048_576.0;

/// The LUT sample spacing, in EV.
fn tone_lut_step() -> f64 {
    f64::from(TONE_LUT_MAX_EV - TONE_LUT_MIN_EV) / f64::from(LAST_INDEX_F32)
}

fn logistic(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

/// The tone sliders as amounts in EV (zones) and slope factor (contrast).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToneMap {
    /// Highlights shift, in EV.
    pub highlights: f64,
    /// Shadows shift, in EV.
    pub shadows: f64,
    /// Whites shift, in EV.
    pub whites: f64,
    /// Blacks shift, in EV.
    pub blacks: f64,
    /// Contrast slope factor `k`.
    pub contrast: f64,
}

impl ToneMap {
    /// Converts slider values (`-100..=100`) to amounts. The sliders must be
    /// validated (finite and in range).
    pub fn from_sliders(tone: &BasicTone) -> Self {
        let s = |v: f32| f64::from(v) / 100.0;
        Self {
            highlights: s(tone.highlights) * HIGHLIGHTS_MAX_EV,
            shadows: s(tone.shadows) * SHADOWS_MAX_EV,
            whites: s(tone.whites) * WHITES_MAX_EV,
            blacks: s(tone.blacks) * BLACKS_MAX_EV,
            contrast: s(tone.contrast) * CONTRAST_MAX_SLOPE,
        }
    }

    /// Returns `true` when the zone sliders (not contrast) are all zero.
    pub fn zones_are_identity(&self) -> bool {
        self.highlights == 0.0 && self.shadows == 0.0 && self.whites == 0.0 && self.blacks == 0.0
    }

    /// Returns `true` when all sliders are zero.
    pub fn is_identity(&self) -> bool {
        self.zones_are_identity() && self.contrast == 0.0
    }

    /// The highlights function.
    pub fn highlights_fn(&self, e: f64) -> f64 {
        e + self.highlights * logistic((e - HIGHLIGHTS_CENTER) / HIGHLIGHTS_SCALE)
    }

    /// The shadows function.
    pub fn shadows_fn(&self, e: f64) -> f64 {
        let d = (e - SHADOWS_CENTER) / SHADOWS_SIGMA;
        e + self.shadows * (-0.5 * d * d).exp()
    }

    /// The whites function.
    pub fn whites_fn(&self, e: f64) -> f64 {
        e + self.whites * logistic((e - WHITES_CENTER) / WHITES_SCALE)
    }

    /// The blacks function.
    pub fn blacks_fn(&self, e: f64) -> f64 {
        e + self.blacks * logistic(-(e - BLACKS_CENTER) / BLACKS_SCALE)
    }

    /// The contrast function.
    pub fn contrast_fn(&self, e: f64) -> f64 {
        e + self.contrast * CONTRAST_WIDTH_EV * (e / CONTRAST_WIDTH_EV).tanh()
    }

    /// The full map: zones in order (highlights, shadows, whites, blacks),
    /// then contrast.
    pub fn map(&self, e: f64) -> f64 {
        let e = self.highlights_fn(e);
        let e = self.shadows_fn(e);
        let e = self.whites_fn(e);
        let e = self.blacks_fn(e);
        self.contrast_fn(e)
    }

    /// Builds the LUT of `map(e) - e` (the log2 of the luminance ratio) at
    /// [`TONE_LUT_SIZE`] samples from [`TONE_LUT_MIN_EV`] to
    /// [`TONE_LUT_MAX_EV`].
    pub fn build_lut(&self) -> Box<ToneLut> {
        let step = tone_lut_step();
        let mut lut = Box::new([0.0_f32; TONE_LUT_SIZE]);
        for (i, out) in lut.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "i < 2048 is exact")]
            let e = f64::from(TONE_LUT_MIN_EV) + step * i as f64;
            #[allow(
                clippy::cast_possible_truncation,
                reason = "an EV shift needs no more than f32 precision"
            )]
            let delta = (self.map(e) - e) as f32;
            *out = delta;
        }
        lut
    }
}

/// The tone LUT: `log2` of the luminance ratio at each sample.
pub type ToneLut = [f32; TONE_LUT_SIZE];

/// Reads the tone LUT at luminance `y`: returns `log2` of the luminance
/// ratio. The GPU kernel uses the same arithmetic.
///
/// A NaN `y` uses the floor (`f32::max` returns the other operand). The
/// caller multiplies the NaN pixel by the ratio, so the pixel stays NaN.
#[inline]
pub(crate) fn lut_delta(lut: &ToneLut, y: f32) -> f32 {
    let e = (y.max(LUMINANCE_FLOOR) / MID_GRAY).log2();
    let t = ((e - TONE_LUT_MIN_EV) * TONE_LUT_SAMPLES_PER_EV).clamp(0.0, LAST_INDEX_F32);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "t is clamped to 0..=2047 above"
    )]
    let i = (t.floor() as usize).min(TONE_LUT_SIZE - 2);
    #[allow(clippy::cast_precision_loss, reason = "i < 2048 is exact")]
    let f = t - i as f32;
    // `i + 1 < TONE_LUT_SIZE` from the `min` above, so both reads are in
    // bounds of the fixed-size array.
    let a = lut[i];
    let b = lut[i + 1];
    a + f * (b - a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sliders(c: f32, h: f32, s: f32, w: f32, b: f32) -> ToneMap {
        ToneMap::from_sliders(&BasicTone {
            exposure: 0.0,
            contrast: c,
            highlights: h,
            shadows: s,
            whites: w,
            blacks: b,
        })
    }

    #[test]
    fn identity_map_is_zero_lut() {
        let t = sliders(0.0, 0.0, 0.0, 0.0, 0.0);
        assert!(t.is_identity());
        assert!(t.build_lut().iter().all(|v| *v == 0.0));
    }

    #[test]
    fn map_is_strictly_increasing_for_extreme_sliders() {
        let values = [-100.0, -50.0, 0.0, 50.0, 100.0];
        let step = 1.0 / 64.0;
        for c in values {
            for h in values {
                for s in values {
                    for w in values {
                        for b in values {
                            let t = sliders(c, h, s, w, b);
                            let mut prev = t.map(-24.0);
                            let mut e = -24.0 + step;
                            while e <= 16.0 {
                                let v = t.map(e);
                                assert!(
                                    v - prev > 0.005 * step,
                                    "not increasing at e={e} for {t:?}"
                                );
                                prev = v;
                                e += step;
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn contrast_keeps_mid_gray_and_spreads_tones() {
        let t = sliders(60.0, 0.0, 0.0, 0.0, 0.0);
        assert!(t.map(0.0).abs() < 1e-12);
        assert!(t.map(2.0) > 2.0);
        assert!(t.map(-2.0) < -2.0);
        let soft = sliders(-60.0, 0.0, 0.0, 0.0, 0.0);
        assert!(soft.map(2.0) < 2.0);
        assert!(soft.map(-2.0) > -2.0);
    }

    #[test]
    fn zones_act_in_their_range() {
        let shadows = sliders(0.0, 0.0, 100.0, 0.0, 0.0);
        assert!(shadows.map(-2.5) - (-2.5) > 1.2);
        assert!((shadows.map(-9.0) + 9.0).abs() < 0.01);
        assert!((shadows.map(4.0) - 4.0).abs() < 0.01);

        let highlights = sliders(0.0, -100.0, 0.0, 0.0, 0.0);
        // Values above 1.0 (e = 2.47) move down: highlight recovery.
        assert!(highlights.map(4.0) - 4.0 < -1.2);
        assert!((highlights.map(-3.0) + 3.0).abs() < 0.02);

        let whites = sliders(0.0, 0.0, 0.0, 100.0, 0.0);
        assert!(whites.map(4.0) - 4.0 > 1.35);
        assert!((whites.map(0.0)).abs() < 0.03);

        let blacks = sliders(0.0, 0.0, 0.0, 0.0, -100.0);
        assert!(blacks.map(-9.0) + 9.0 < -1.4);
        assert!(blacks.map(0.0).abs() < 0.01);
    }

    #[test]
    fn lut_matches_analytic_map() {
        let t = sliders(35.0, -70.0, 55.0, 20.0, -30.0);
        let lut = t.build_lut();
        let mut y = 1e-6_f32;
        while y < 500.0 {
            let e = (f64::from(y) / f64::from(MID_GRAY)).log2();
            let expected = t.map(e) - e;
            let got = f64::from(lut_delta(&lut, y));
            assert!(
                (got - expected).abs() < 2e-5,
                "y={y}: lut {got}, analytic {expected}"
            );
            y *= 1.07;
        }
    }

    #[test]
    #[allow(clippy::float_cmp, reason = "the clamped LUT read must be exact")]
    fn lut_is_flat_outside_range() {
        let t = sliders(35.0, -70.0, 55.0, 20.0, -30.0);
        let lut = t.build_lut();
        let low = lut_delta(&lut, 0.0);
        assert_eq!(low, lut_delta(&lut, -5.0));
        assert_eq!(low, lut[0]);
        assert_eq!(lut_delta(&lut, 1e9), lut[TONE_LUT_SIZE - 1]);
    }
}
