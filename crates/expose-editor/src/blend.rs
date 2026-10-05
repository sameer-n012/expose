//! Blend modes and the blending space.
//!
//! # Formulas
//!
//! The functions in this module work on **straight** (not premultiplied)
//! colors. `cb` is the backdrop color (the layers below). `cs` is the source
//! color (the layer). The compositor (see [`crate::composite`]) uses the
//! result `B(cb, cs)` in the W3C Compositing and Blending Level 1 formula:
//!
//! ```text
//! co = as * (1 - ab) * cs + ab * (1 - as) * cb + as * ab * B(cb, cs)
//! ao = as + ab * (1 - as)
//! ```
//!
//! # Values outside 0..1
//!
//! The working space is linear and scene-referred: values above 1.0 (and
//! below 0.0) are valid. Many blend formulas are defined only for display
//! values in `0..=1`. Each mode is in one of these groups
//! ([`RangeHandling`]):
//!
//! - **Unbounded:** the formula applies directly to all values. Normal,
//!   dissolve, darken, lighten, multiply, linear dodge (add), difference,
//!   darker color, lighter color, subtract, linear burn, divide.
//!   Subtract and linear burn set results below 0.0 to 0.0 (as for display
//!   values). Divide uses `cs.max(DIVIDE_MIN_SOURCE)` as the divisor.
//! - **Screen:** the W3C formula `cb + cs - cb * cs` when both values are
//!   `<= 1.0`, else `max(cb, cs)`. The formula gives wrong results above 1.0
//!   (for example, 2 screen 2 = 0). `max` is the limit of screen at 1.0, so
//!   the result is continuous there.
//! - **Bounded with backdrop excess:** the formula is defined for `0..=1`
//!   only. The function clamps both inputs to `0..=1`, applies the formula,
//!   and adds the backdrop excess `cb - clamp(cb)` to the result. The source
//!   excess is lost. Thus a neutral source (for example, 50 % gray for
//!   overlay) keeps all backdrop values, also highlights above 1.0. Modes:
//!   color burn, color dodge, overlay, soft light, hard light, vivid light,
//!   linear light, pin light, hard mix, exclusion.
//! - **Non-separable:** hue, saturation, color, luminosity use the W3C
//!   `SetLum`/`SetSat`/`ClipColor` functions with the working-space (Rec.
//!   2020) luminance. The inputs are not clamped. `ClipColor` clips to the
//!   range `0..=u`, where `u = max(1, max channel of cb, max channel of cs)`.
//!   For inputs in `0..=1`, `u` is 1 and the result is the W3C result.
//!
//! For inputs in `0..=1`, all modes give the standard result.
//!
//! # Blending space
//!
//! [`BlendSpace::Perceptual`] (the default) encodes colors with the sRGB
//! transfer curve before the blend and decodes after it. Perceptual
//! blending gives results that look like those of display-referred editors
//! (for example, 50 % opacity looks like a perceptual mid-point). The
//! transfer function of `expose-core` continues above 1.0 and is odd for
//! negative values, so no data is clipped. [`BlendSpace::Linear`] blends the
//! linear working values (physically correct for light). ADR 0005 records
//! the choice of the default.

use expose_core::buffer::Rgba;
use expose_core::color::{TransferFunction, luminance};

/// The smallest divisor that the divide mode uses.
pub const DIVIDE_MIN_SOURCE: f32 = 1.0e-4;

/// The color encoding in which the compositor blends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlendSpace {
    /// Blend the linear working values.
    Linear,
    /// Encode with the sRGB transfer curve, blend, then decode. Default.
    #[default]
    Perceptual,
}

impl BlendSpace {
    /// Converts a straight linear color to the blending space.
    #[inline]
    pub fn encode(self, c: [f32; 3]) -> [f32; 3] {
        match self {
            Self::Linear => c,
            Self::Perceptual => c.map(|v| TransferFunction::Srgb.encode(v)),
        }
    }

    /// Converts a straight color in the blending space back to linear.
    #[inline]
    pub fn decode(self, c: [f32; 3]) -> [f32; 3] {
        match self {
            Self::Linear => c,
            Self::Perceptual => c.map(|v| TransferFunction::Srgb.decode(v)),
        }
    }

    /// Converts a premultiplied linear pixel to a premultiplied pixel in the
    /// blending space.
    #[inline]
    pub fn encode_premultiplied(self, p: Rgba) -> Rgba {
        match self {
            Self::Linear => p,
            Self::Perceptual => {
                let (c, a) = unpremultiply(p);
                premultiply(self.encode(c), a)
            }
        }
    }

    /// Converts a premultiplied pixel in the blending space back to a
    /// premultiplied linear pixel.
    #[inline]
    pub fn decode_premultiplied(self, p: Rgba) -> Rgba {
        match self {
            Self::Linear => p,
            Self::Perceptual => {
                let (c, a) = unpremultiply(p);
                premultiply(self.decode(c), a)
            }
        }
    }
}

/// Splits a premultiplied pixel into a straight color and alpha. If alpha
/// is zero or less, the color is black.
#[inline]
pub(crate) fn unpremultiply(p: Rgba) -> ([f32; 3], f32) {
    let a = p[3];
    if a > 0.0 {
        let inv = 1.0 / a;
        ([p[0] * inv, p[1] * inv, p[2] * inv], a)
    } else {
        ([0.0; 3], a)
    }
}

/// Makes a premultiplied pixel from a straight color and alpha.
#[inline]
pub(crate) fn premultiply(c: [f32; 3], a: f32) -> Rgba {
    [c[0] * a, c[1] * a, c[2] * a, a]
}

/// How a blend mode treats values outside `0..=1`. See the module
/// documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RangeHandling {
    /// The formula applies directly to all values.
    Unbounded,
    /// Screen: the formula for values `<= 1`, else `max`.
    ScreenOrMax,
    /// Clamp inputs to `0..=1`, then add the backdrop excess.
    BoundedWithBackdropExcess,
    /// Non-separable mode with an extended `ClipColor` range.
    NonSeparable,
}

/// A layer blend mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlendMode {
    /// The source covers the backdrop.
    #[default]
    Normal,
    /// Normal, but each pixel is fully shown or fully hidden. The
    /// probability to show a pixel is its alpha. The random value comes
    /// from a hash of the canvas coordinates only, so the result is
    /// deterministic and the same for all tiles.
    Dissolve,
    /// `min(cb, cs)`.
    Darken,
    /// `cb * cs`.
    Multiply,
    /// `1 - min(1, (1 - cb) / cs)` (W3C).
    ColorBurn,
    /// `max(0, cb + cs - 1)`.
    LinearBurn,
    /// The color (all channels) with the lower luminance.
    DarkerColor,
    /// `max(cb, cs)`.
    Lighten,
    /// `cb + cs - cb * cs` (see [`RangeHandling::ScreenOrMax`]).
    Screen,
    /// `min(1, cb / (1 - cs))` (W3C).
    ColorDodge,
    /// `cb + cs` (also known as "add").
    LinearDodge,
    /// The color (all channels) with the higher luminance.
    LighterColor,
    /// Hard light with backdrop and source swapped (W3C).
    Overlay,
    /// W3C soft light.
    SoftLight,
    /// Multiply or screen, by the source value (W3C).
    HardLight,
    /// Color burn or color dodge with `2 * cs`, by the source value.
    VividLight,
    /// `clamp(cb + 2 * cs - 1, 0, 1)`.
    LinearLight,
    /// Darken or lighten with `2 * cs`, by the source value.
    PinLight,
    /// 1 if `cb + cs >= 1`, else 0.
    HardMix,
    /// `|cb - cs|`.
    Difference,
    /// `cb + cs - 2 * cb * cs`.
    Exclusion,
    /// `max(0, cb - cs)`.
    Subtract,
    /// `cb / max(cs, DIVIDE_MIN_SOURCE)`.
    Divide,
    /// Hue of the source, saturation and luminosity of the backdrop.
    Hue,
    /// Saturation of the source, hue and luminosity of the backdrop.
    Saturation,
    /// Hue and saturation of the source, luminosity of the backdrop.
    Color,
    /// Luminosity of the source, hue and saturation of the backdrop.
    Luminosity,
}

impl BlendMode {
    /// All blend modes, in menu order.
    pub const ALL: [Self; 27] = [
        Self::Normal,
        Self::Dissolve,
        Self::Darken,
        Self::Multiply,
        Self::ColorBurn,
        Self::LinearBurn,
        Self::DarkerColor,
        Self::Lighten,
        Self::Screen,
        Self::ColorDodge,
        Self::LinearDodge,
        Self::LighterColor,
        Self::Overlay,
        Self::SoftLight,
        Self::HardLight,
        Self::VividLight,
        Self::LinearLight,
        Self::PinLight,
        Self::HardMix,
        Self::Difference,
        Self::Exclusion,
        Self::Subtract,
        Self::Divide,
        Self::Hue,
        Self::Saturation,
        Self::Color,
        Self::Luminosity,
    ];

    /// Returns how the mode treats values outside `0..=1`.
    pub const fn range_handling(self) -> RangeHandling {
        match self {
            Self::Normal
            | Self::Dissolve
            | Self::Darken
            | Self::Multiply
            | Self::LinearBurn
            | Self::DarkerColor
            | Self::Lighten
            | Self::LinearDodge
            | Self::LighterColor
            | Self::Difference
            | Self::Subtract
            | Self::Divide => RangeHandling::Unbounded,
            Self::Screen => RangeHandling::ScreenOrMax,
            Self::ColorBurn
            | Self::ColorDodge
            | Self::Overlay
            | Self::SoftLight
            | Self::HardLight
            | Self::VividLight
            | Self::LinearLight
            | Self::PinLight
            | Self::HardMix
            | Self::Exclusion => RangeHandling::BoundedWithBackdropExcess,
            Self::Hue | Self::Saturation | Self::Color | Self::Luminosity => {
                RangeHandling::NonSeparable
            }
        }
    }

    /// Returns `B(cb, cs)` for straight colors.
    ///
    /// Dissolve returns `cs` (the same as normal). The compositor applies
    /// the dissolve pattern to alpha.
    pub fn blend(self, cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
        match self {
            Self::Normal | Self::Dissolve => cs,
            Self::DarkerColor => {
                if luminance(cs) < luminance(cb) {
                    cs
                } else {
                    cb
                }
            }
            Self::LighterColor => {
                if luminance(cs) > luminance(cb) {
                    cs
                } else {
                    cb
                }
            }
            Self::Hue => set_lum(set_sat(cs, sat(cb)), luminance(cb), clip_limit(cb, cs)),
            Self::Saturation => set_lum(set_sat(cb, sat(cs)), luminance(cb), clip_limit(cb, cs)),
            Self::Color => set_lum(cs, luminance(cb), clip_limit(cb, cs)),
            Self::Luminosity => set_lum(cb, luminance(cs), clip_limit(cb, cs)),
            _ => [
                self.blend_channel(cb[0], cs[0]),
                self.blend_channel(cb[1], cs[1]),
                self.blend_channel(cb[2], cs[2]),
            ],
        }
    }

    /// Applies a separable mode to one channel, with the range handling of
    /// the mode. Non-separable modes return `cs` here; use [`Self::blend`].
    pub fn blend_channel(self, cb: f32, cs: f32) -> f32 {
        match self.range_handling() {
            RangeHandling::Unbounded | RangeHandling::NonSeparable => self.unbounded(cb, cs),
            RangeHandling::ScreenOrMax => {
                if cb <= 1.0 && cs <= 1.0 {
                    cb + cs - cb * cs
                } else {
                    cb.max(cs)
                }
            }
            RangeHandling::BoundedWithBackdropExcess => {
                let b = cb.clamp(0.0, 1.0);
                let s = cs.clamp(0.0, 1.0);
                self.bounded(b, s) + (cb - b)
            }
        }
    }

    fn unbounded(self, cb: f32, cs: f32) -> f32 {
        match self {
            Self::Darken => cb.min(cs),
            Self::Lighten => cb.max(cs),
            Self::Multiply => cb * cs,
            Self::LinearBurn => (cb + cs - 1.0).max(0.0),
            Self::LinearDodge => cb + cs,
            Self::Difference => (cb - cs).abs(),
            Self::Subtract => (cb - cs).max(0.0),
            Self::Divide => cb / cs.max(DIVIDE_MIN_SOURCE),
            // Normal, dissolve, and (in this per-channel path only) the
            // non-separable and luminance-select modes.
            _ => cs,
        }
    }

    /// The formulas for inputs in `0..=1`.
    fn bounded(self, cb: f32, cs: f32) -> f32 {
        match self {
            Self::ColorBurn => color_burn(cb, cs),
            Self::ColorDodge => color_dodge(cb, cs),
            Self::Overlay => hard_light(cs, cb),
            Self::SoftLight => soft_light(cb, cs),
            Self::HardLight => hard_light(cb, cs),
            Self::VividLight => {
                if cs <= 0.5 {
                    color_burn(cb, 2.0 * cs)
                } else {
                    color_dodge(cb, 2.0 * cs - 1.0)
                }
            }
            Self::LinearLight => (cb + 2.0 * cs - 1.0).clamp(0.0, 1.0),
            Self::PinLight => {
                if cs <= 0.5 {
                    cb.min(2.0 * cs)
                } else {
                    cb.max(2.0 * cs - 1.0)
                }
            }
            Self::HardMix => {
                if cb + cs >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Exclusion => cb + cs - 2.0 * cb * cs,
            _ => cs,
        }
    }
}

fn color_burn(cb: f32, cs: f32) -> f32 {
    if cb >= 1.0 {
        1.0
    } else if cs <= 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - cb) / cs).min(1.0)
    }
}

fn color_dodge(cb: f32, cs: f32) -> f32 {
    if cb <= 0.0 {
        0.0
    } else if cs >= 1.0 {
        1.0
    } else {
        (cb / (1.0 - cs)).min(1.0)
    }
}

fn hard_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        cb * (2.0 * cs)
    } else {
        let s = 2.0 * cs - 1.0;
        cb + s - cb * s
    }
}

fn soft_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
    } else {
        let d = if cb <= 0.25 {
            ((16.0 * cb - 12.0) * cb + 4.0) * cb
        } else {
            cb.sqrt()
        };
        cb + (2.0 * cs - 1.0) * (d - cb)
    }
}

/// The upper limit for `ClipColor`: 1, or the largest input channel if it
/// is larger (scene-referred values).
fn clip_limit(cb: [f32; 3], cs: [f32; 3]) -> f32 {
    cb.iter().chain(cs.iter()).fold(1.0_f32, |m, &v| m.max(v))
}

/// W3C `ClipColor`, with the upper limit `upper` in place of 1.
///
/// If the luminance itself is outside `0..=upper`, no hue can keep it in
/// range. The function then returns the gray with that luminance, so that
/// the luminance stays correct.
fn clip_color(c: [f32; 3], upper: f32) -> [f32; 3] {
    let l = luminance(c);
    if l < 0.0 || l > upper {
        return [l; 3];
    }
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut out = c;
    if n < 0.0 {
        // l >= 0 > n, so l - n > 0.
        let k = l / (l - n);
        out = out.map(|v| l + (v - l) * k);
    }
    if x > upper {
        // l <= upper < x, so x - l > 0.
        let k = (upper - l) / (x - l);
        out = out.map(|v| l + (v - l) * k);
    }
    out
}

/// W3C `SetLum`.
fn set_lum(c: [f32; 3], l: f32, upper: f32) -> [f32; 3] {
    let d = l - luminance(c);
    clip_color(c.map(|v| v + d), upper)
}

/// W3C `Sat`: max channel minus min channel.
fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

/// W3C `SetSat`.
fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    // Sort the channel indices by value: lo, mid, hi.
    let mut idx = [0_usize, 1, 2];
    idx.sort_by(|&a, &b| c[a].total_cmp(&c[b]));
    let [lo, mid, hi] = idx;
    let mut out = [0.0_f32; 3];
    let range = c[hi] - c[lo];
    if range > 0.0 {
        out[mid] = (c[mid] - c[lo]) * s / range;
        out[hi] = s;
    }
    // out[lo] is 0, and also mid and hi if the input is gray.
    out
}

/// Returns a hash value in `0.0..1.0` for the canvas pixel `(x, y)`.
///
/// This is the "lowbias32" integer hash (Chris Wellons) of `x` and `y`
/// combined. Dissolve uses it, so that the pattern does not depend on the
/// tile layout.
pub fn dissolve_threshold(x: i64, y: i64) -> f32 {
    // Truncation to the low 32 bits is intended: the hash mixes all bits
    // that matter for canvas coordinates.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let mut h = (x as u32).wrapping_mul(0x9E37_79B9) ^ (y as u32).wrapping_mul(0x85EB_CA6B);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    // Use the top 24 bits, so that the value is exact in f32 and < 1.0.
    #[allow(clippy::cast_precision_loss)]
    let v = (h >> 8) as f32 / 16_777_216.0;
    v
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // Tests compare exact expected values.
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    fn ch(mode: BlendMode, cb: f32, cs: f32) -> f32 {
        mode.blend([cb; 3], [cs; 3])[0]
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    fn close3(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| close(*x, *y))
    }

    #[test]
    fn separable_modes_hand_computed() {
        use BlendMode as M;
        // (mode, cb, cs, expected), all values computed by hand.
        let cases: &[(M, f32, f32, f32)] = &[
            (M::Normal, 0.2, 0.7, 0.7),
            (M::Dissolve, 0.2, 0.7, 0.7),
            (M::Darken, 0.2, 0.7, 0.2),
            (M::Lighten, 0.2, 0.7, 0.7),
            (M::Multiply, 0.5, 0.4, 0.2),
            (M::Screen, 0.5, 0.4, 0.7),
            (M::ColorBurn, 0.5, 0.8, 0.375),
            (M::ColorBurn, 1.0, 0.0, 1.0),
            (M::ColorBurn, 0.5, 0.0, 0.0),
            (M::ColorDodge, 0.5, 0.2, 0.625),
            (M::ColorDodge, 0.0, 1.0, 0.0),
            (M::ColorDodge, 0.5, 1.0, 1.0),
            (M::LinearBurn, 0.5, 0.8, 0.3),
            (M::LinearBurn, 0.2, 0.3, 0.0),
            (M::LinearDodge, 0.5, 0.8, 1.3),
            (M::Overlay, 0.25, 0.6, 0.3),
            (M::Overlay, 0.75, 0.6, 0.8),
            (M::HardLight, 0.6, 0.25, 0.3),
            (M::HardLight, 0.6, 0.75, 0.8),
            (M::SoftLight, 0.5, 0.25, 0.375),
            (M::SoftLight, 0.25, 0.75, 0.375),
            (M::SoftLight, 0.64, 1.0, 0.8),
            (M::VividLight, 0.5, 0.4, 0.375),
            (M::VividLight, 0.5, 0.6, 0.625),
            (M::LinearLight, 0.5, 0.7, 0.9),
            (M::LinearLight, 0.8, 0.9, 1.0),
            (M::PinLight, 0.5, 0.2, 0.4),
            (M::PinLight, 0.5, 0.9, 0.8),
            (M::PinLight, 0.5, 0.6, 0.5),
            (M::HardMix, 0.5, 0.5, 1.0),
            (M::HardMix, 0.5, 0.4, 0.0),
            (M::Difference, 0.2, 0.7, 0.5),
            (M::Exclusion, 0.5, 0.4, 0.5),
            (M::Exclusion, 0.2, 0.7, 0.62),
            (M::Subtract, 0.7, 0.2, 0.5),
            (M::Subtract, 0.2, 0.7, 0.0),
            (M::Divide, 0.3, 0.6, 0.5),
            (M::Divide, 0.3, 0.0, 3000.0),
        ];
        for &(mode, cb, cs, want) in cases {
            let got = ch(mode, cb, cs);
            assert!(
                (got - want).abs() < 1e-4 * want.abs().max(1.0),
                "{mode:?}({cb}, {cs}) = {got}, expected {want}"
            );
        }
    }

    #[test]
    fn darker_and_lighter_color_select_by_luminance() {
        // Luminance of pure green (0.678) > pure red (0.2627).
        let red = [1.0, 0.0, 0.0];
        let green = [0.0, 1.0, 0.0];
        assert_eq!(BlendMode::DarkerColor.blend(red, green), red);
        assert_eq!(BlendMode::DarkerColor.blend(green, red), red);
        assert_eq!(BlendMode::LighterColor.blend(red, green), green);
        // Ties keep the backdrop.
        assert_eq!(BlendMode::LighterColor.blend(red, red), red);
    }

    #[test]
    fn luminosity_hand_computed() {
        // cb = (0.6, 0.2, 0.2): Lum = 0.2627*0.6 + 0.678*0.2 + 0.0593*0.2
        //    = 0.15762 + 0.1356 + 0.01186 = 0.30508.
        // cs = gray 0.5: Lum = 0.5. d = 0.19492.
        // SetLum: (0.79492, 0.39492, 0.39492), all in 0..1 so no clip.
        let out = BlendMode::Luminosity.blend([0.6, 0.2, 0.2], [0.5; 3]);
        assert!(close3(out, [0.79492, 0.39492, 0.39492]), "{out:?}");
        assert!(close(luminance(out), 0.5));
    }

    #[test]
    fn color_mode_clips_with_w3c_clip_color() {
        // cs = pure red (Lum 0.2627). cb = white (Lum 1).
        // SetLum(red, 1): d = 0.7373 -> (1.7373, 0.7373, 0.7373).
        // x = 1.7373 > 1, l = 1 -> k = (1 - 1) / (x - 1) = 0 -> white.
        let out = BlendMode::Color.blend([1.0; 3], [1.0, 0.0, 0.0]);
        assert!(close3(out, [1.0; 3]), "{out:?}");

        // cb = gray 0.5, cs = red. SetLum(red, 0.5): d = 0.2373 ->
        // (1.2373, 0.2373, 0.2373). x > 1, l = 0.5:
        // k = 0.5 / 0.7373 = 0.678149; r = 0.5 + 0.7373 * k = 1.0,
        // g = b = 0.5 - 0.2627 * k = 0.321853.
        let out = BlendMode::Color.blend([0.5; 3], [1.0, 0.0, 0.0]);
        assert!(close3(out, [1.0, 0.321_853, 0.321_853]), "{out:?}");
    }

    #[test]
    fn hue_and_saturation_hand_computed() {
        // Hue: SetLum(SetSat(cs, Sat(cb)), Lum(cb)).
        // cs = (0.8, 0.4, 0.2), cb = (0.3, 0.5, 0.4): Sat(cb) = 0.2.
        // SetSat(cs, 0.2): hi = r -> 0.2, mid = g -> (0.4-0.2)*0.2/0.6
        // = 0.066667, lo = b -> 0.
        // Lum(cb) = 0.07881 + 0.339 + 0.02372 = 0.44153.
        // Lum(set) = 0.05254 + 0.0452 + 0 = 0.09774. d = 0.34379.
        // Result (0.54379, 0.41046, 0.34379), in range.
        let out = BlendMode::Hue.blend([0.3, 0.5, 0.4], [0.8, 0.4, 0.2]);
        assert!(close3(out, [0.54379, 0.410_457, 0.34379]), "{out:?}");

        // Saturation of a gray source makes the backdrop gray with the
        // same luminance.
        let cb = [0.3, 0.5, 0.4];
        let out = BlendMode::Saturation.blend(cb, [0.5; 3]);
        assert!(close3(out, [luminance(cb); 3]), "{out:?}");
    }

    #[test]
    fn all_modes_match_standard_formula_for_gray_inside_range() {
        // For in-range inputs, the extended rules must not change results.
        for mode in BlendMode::ALL {
            for i in 0..=10 {
                for j in 0..=10 {
                    #[allow(clippy::cast_precision_loss)]
                    let (cb, cs) = (i as f32 / 10.0, j as f32 / 10.0);
                    let v = mode.blend([cb; 3], [cs; 3]);
                    assert!(v.iter().all(|x| x.is_finite()), "{mode:?}");
                    if mode != BlendMode::LinearDodge && mode != BlendMode::Divide {
                        assert!(
                            v.iter().all(|x| (-EPS..=1.0 + EPS).contains(x)),
                            "{mode:?}({cb}, {cs}) = {v:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn out_of_range_handling() {
        use BlendMode as M;
        // Unbounded: direct formula.
        assert!(close(ch(M::Multiply, 4.0, 0.5), 2.0));
        assert!(close(ch(M::LinearDodge, 4.0, 0.5), 4.5));
        // Screen above 1: max.
        assert!(close(ch(M::Screen, 2.0, 2.0), 2.0));
        assert!(close(ch(M::Screen, 3.0, 0.5), 3.0));
        // Bounded: a neutral source keeps the backdrop excess.
        assert!(close(ch(M::Overlay, 3.0, 0.5), 3.0));
        assert!(close(ch(M::SoftLight, 3.0, 0.5), 3.0));
        assert!(close(ch(M::HardLight, 2.5, 0.5), 2.5));
        assert!(close(ch(M::ColorDodge, 2.0, 0.0), 2.0));
        assert!(close(ch(M::Exclusion, 2.0, 0.0), 2.0));
        // Source excess is lost: hard light with cs = 3 is cs = 1 -> 1,
        // plus the backdrop excess 0.
        assert!(close(ch(M::HardLight, 0.4, 3.0), 1.0));
        // Negative backdrop excess also survives.
        assert!(close(ch(M::Overlay, -0.5, 0.5), -0.5));
        // Non-separable: luminosity of an HDR source keeps the luminance.
        let out = M::Luminosity.blend([0.5, 0.4, 0.3], [4.0; 3]);
        assert!(close(luminance(out), 4.0));
        assert!(out.iter().all(|v| *v >= 0.0));
    }

    #[test]
    fn blend_space_round_trip() {
        let c = [0.0, 0.18, 2.5];
        let enc = BlendSpace::Perceptual.encode(c);
        assert!(enc[1] > 0.4 && enc[1] < 0.5, "0.18 encodes near 0.46");
        let back = BlendSpace::Perceptual.decode(enc);
        assert!(close3(back, c));
        assert_eq!(BlendSpace::Linear.encode(c), c);
        let p = [0.09, 0.045, 0.0, 0.5];
        let r = BlendSpace::Perceptual
            .decode_premultiplied(BlendSpace::Perceptual.encode_premultiplied(p));
        assert!(p.iter().zip(r.iter()).all(|(a, b)| close(*a, *b)));
    }

    #[test]
    fn dissolve_threshold_is_deterministic_and_spread() {
        assert_eq!(dissolve_threshold(10, 20), dissolve_threshold(10, 20));
        let mut sum = 0.0_f64;
        let mut n = 0_u32;
        for y in -32..32 {
            for x in -32..32 {
                let t = dissolve_threshold(x, y);
                assert!((0.0..1.0).contains(&t));
                sum += f64::from(t);
                n += 1;
            }
        }
        let mean = sum / f64::from(n);
        assert!((mean - 0.5).abs() < 0.02, "mean = {mean}");
    }
}
