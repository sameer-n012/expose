//! Adjustment layer parameters and their CPU apply functions.
//!
//! # Encoding
//!
//! The parameters of these adjustments are defined on perceptual values
//! (for example, a curve point at 0.5 is mid-gray, and levels use display
//! black and white points). Thus each adjustment encodes the linear working
//! color with the sRGB transfer curve, applies its formula, and decodes the
//! result. This does not depend on the
//! [`BlendSpace`](crate::BlendSpace) of the document.
//!
//! # Values outside 0..1
//!
//! Each formula `f` is defined on `0..=1`. For an encoded value `v` outside
//! that range, the adjustment returns `f(clamp(v)) + (v - clamp(v))`. Thus
//! values above 1.0 (highlights) keep their distance above the value that
//! the formula gives at 1.0, and negative values keep their distance below
//! the value at 0.0. The result is continuous at 0 and 1.
//!
//! Alpha does not change. All functions take and return **straight**
//! (not premultiplied) linear colors.

use expose_core::color::{TransferFunction, luminance};
use expose_core::develop::{CurvePoint, MAX_CURVE_POINTS};
use expose_core::math::MonotoneSpline;

use crate::error::{EditorError, EditorResult, check_range};

/// Applies `f` on `0..=1` and keeps the excess outside that range.
#[inline]
fn extend(v: f32, f: impl Fn(f32) -> f32) -> f32 {
    let c = v.clamp(0.0, 1.0);
    f(c) + (v - c)
}

#[inline]
fn encode(c: [f32; 3]) -> [f32; 3] {
    c.map(|v| TransferFunction::Srgb.encode(v))
}

#[inline]
fn decode(c: [f32; 3]) -> [f32; 3] {
    c.map(|v| TransferFunction::Srgb.decode(v))
}

/// Brightness and contrast.
///
/// - Brightness `b` in `-1..=1`: for `b >= 0`, `v + (1 - v) * b`; for
///   `b < 0`, `v * (1 + b)`.
/// - Contrast `c` in `-1..=1`: `(v - 0.5) * k + 0.5` with
///   `k = tan((c + 1) * pi / 4)`. At `c = -1` the result is flat mid-gray.
///   At `c = 1` (`k` is infinite) the result is a threshold at 0.5.
///
/// Brightness applies first, then contrast.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BrightnessContrast {
    /// Brightness, `-1.0..=1.0`.
    pub brightness: f32,
    /// Contrast, `-1.0..=1.0`.
    pub contrast: f32,
}

impl BrightnessContrast {
    /// Checks the parameter ranges.
    pub fn validate(&self) -> EditorResult<()> {
        check_range("brightness", self.brightness, -1.0, 1.0)?;
        check_range("contrast", self.contrast, -1.0, 1.0)
    }

    fn prepare(self) -> PreparedBc {
        let k = if self.contrast >= 1.0 {
            None
        } else {
            let angle = (f64::from(self.contrast) + 1.0) * std::f64::consts::FRAC_PI_4;
            // The angle is in 0..pi/2, so tan is finite and >= 0. The
            // narrowing to f32 is intended.
            #[allow(clippy::cast_possible_truncation)]
            Some(angle.tan() as f32)
        };
        PreparedBc {
            brightness: self.brightness,
            k,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PreparedBc {
    pub(crate) brightness: f32,
    /// Contrast slope. `None` is an infinite slope (threshold).
    pub(crate) k: Option<f32>,
}

impl PreparedBc {
    fn apply(&self, v: f32) -> f32 {
        let b = self.brightness;
        let v = if b >= 0.0 {
            v + (1.0 - v) * b
        } else {
            v * (1.0 + b)
        };
        match self.k {
            Some(k) => (v - 0.5) * k + 0.5,
            None if v > 0.5 => 1.0,
            None if v < 0.5 => 0.0,
            None => 0.5,
        }
        .clamp(0.0, 1.0)
    }
}

/// Levels for one channel.
///
/// `t = clamp((v - input_black) / (input_white - input_black), 0, 1)`,
/// then `t = t^(1 / gamma)`, then
/// `out = output_black + t * (output_white - output_black)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelsChannel {
    /// Input black point, `0.0..1.0`.
    pub input_black: f32,
    /// Input white point, `0.0..=1.0`. Must be more than `input_black`.
    pub input_white: f32,
    /// Midtone gamma, `0.01..=10.0`. Values above 1 make midtones lighter.
    pub gamma: f32,
    /// Output black, `0.0..=1.0`.
    pub output_black: f32,
    /// Output white, `0.0..=1.0`. It can be less than `output_black` (this
    /// inverts the channel).
    pub output_white: f32,
}

impl Default for LevelsChannel {
    fn default() -> Self {
        Self {
            input_black: 0.0,
            input_white: 1.0,
            gamma: 1.0,
            output_black: 0.0,
            output_white: 1.0,
        }
    }
}

/// The lowest permitted levels gamma.
pub const LEVELS_GAMMA_MIN: f32 = 0.01;
/// The highest permitted levels gamma.
pub const LEVELS_GAMMA_MAX: f32 = 10.0;

impl LevelsChannel {
    /// Checks the parameter ranges.
    pub fn validate(&self) -> EditorResult<()> {
        check_range("input_black", self.input_black, 0.0, 1.0)?;
        check_range("input_white", self.input_white, 0.0, 1.0)?;
        if self.input_white <= self.input_black {
            return Err(EditorError::OutOfRange {
                name: "input_white",
                value: f64::from(self.input_white),
                min: f64::from(self.input_black),
                max: 1.0,
            });
        }
        check_range("gamma", self.gamma, LEVELS_GAMMA_MIN, LEVELS_GAMMA_MAX)?;
        check_range("output_black", self.output_black, 0.0, 1.0)?;
        check_range("output_white", self.output_white, 0.0, 1.0)
    }

    fn apply(&self, v: f32) -> f32 {
        // validate() proved input_white > input_black.
        let t = ((v - self.input_black) / (self.input_white - self.input_black)).clamp(0.0, 1.0);
        let t = t.powf(1.0 / self.gamma);
        self.output_black + t * (self.output_white - self.output_black)
    }
}

/// Levels with one master channel and one channel for each color.
///
/// The color channels apply first, then the master channel.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Levels {
    /// Levels for all three channels.
    pub master: LevelsChannel,
    /// Levels for red.
    pub red: LevelsChannel,
    /// Levels for green.
    pub green: LevelsChannel,
    /// Levels for blue.
    pub blue: LevelsChannel,
}

impl Levels {
    /// Checks all channels.
    pub fn validate(&self) -> EditorResult<()> {
        self.master.validate()?;
        self.red.validate()?;
        self.green.validate()?;
        self.blue.validate()
    }
}

/// Curves with one master curve and one curve for each color.
///
/// Each curve has 2 to [`MAX_CURVE_POINTS`] points. Coordinates are in
/// `0.0..=1.0`, and `x` values are strictly increasing. The curve between
/// the points is a monotone cubic (Fritsch-Carlson). Outside the first and
/// last point, the curve is flat. The color curves apply first, then the
/// master curve.
#[derive(Debug, Clone, PartialEq)]
pub struct Curves {
    /// Curve for all three channels.
    pub master: Vec<CurvePoint>,
    /// Curve for red.
    pub red: Vec<CurvePoint>,
    /// Curve for green.
    pub green: Vec<CurvePoint>,
    /// Curve for blue.
    pub blue: Vec<CurvePoint>,
}

fn identity_points() -> Vec<CurvePoint> {
    vec![CurvePoint::new(0.0, 0.0), CurvePoint::new(1.0, 1.0)]
}

impl Default for Curves {
    fn default() -> Self {
        Self {
            master: identity_points(),
            red: identity_points(),
            green: identity_points(),
            blue: identity_points(),
        }
    }
}

impl Curves {
    /// Checks all curves.
    pub fn validate(&self) -> EditorResult<()> {
        validate_curve("master", &self.master)?;
        validate_curve("red", &self.red)?;
        validate_curve("green", &self.green)?;
        validate_curve("blue", &self.blue)
    }
}

fn validate_curve(channel: &'static str, points: &[CurvePoint]) -> EditorResult<()> {
    let err = |reason| EditorError::InvalidCurve { channel, reason };
    if points.len() < 2 {
        return Err(err("a curve needs at least 2 points"));
    }
    if points.len() > MAX_CURVE_POINTS {
        return Err(err("too many points"));
    }
    for p in points {
        if !p.x.is_finite() || !p.y.is_finite() {
            return Err(err("a coordinate is not finite"));
        }
        if !(0.0..=1.0).contains(&p.x) || !(0.0..=1.0).contains(&p.y) {
            return Err(err("a coordinate is outside 0..=1"));
        }
    }
    if points.windows(2).any(|w| w[1].x <= w[0].x) {
        return Err(err("x values must be strictly increasing"));
    }
    Ok(())
}

/// A monotone cubic spline (Fritsch-Carlson) through control points.
///
/// Where the points are monotone, the spline is monotone too (no
/// overshoot). The spline is flat outside the first and last point. The
/// output is clamped to `0..=1`.
#[derive(Debug, Clone, PartialEq)]
pub struct MonotoneCubic {
    spline: MonotoneSpline,
    identity: bool,
}

impl MonotoneCubic {
    /// Makes a spline. The points must pass the curve validation (2 or
    /// more points, finite, `x` strictly increasing).
    pub fn new(points: &[CurvePoint]) -> EditorResult<Self> {
        validate_curve("curve", points)?;
        // Exact comparison is intended: only the exact default curve is
        // skipped. A near-identity curve is evaluated normally.
        #[allow(clippy::float_cmp)]
        let identity = points.len() == 2
            && points[0].x == 0.0
            && points[0].y == 0.0
            && points[1].x == 1.0
            && points[1].y == 1.0;
        let spline = MonotoneSpline::new(points).ok_or(EditorError::InvalidCurve {
            channel: "curve",
            reason: "a curve needs at least 2 points",
        })?;
        Ok(Self { spline, identity })
    }

    /// Returns `true` if the spline is the identity line from (0, 0) to (1, 1).
    fn is_identity(&self) -> bool {
        self.identity
    }

    /// Evaluates the spline at `x`. The output is clamped to `0..=1`.
    pub fn eval(&self, x: f32) -> f32 {
        // The clamp below keeps the value in 0..=1, so the cast loses
        // only precision.
        #[allow(clippy::cast_possible_truncation)]
        let y = self.spline.eval(f64::from(x)) as f32;
        y.clamp(0.0, 1.0)
    }
}

/// Hue, saturation, and lightness.
///
/// - Hue: rotation of the color about the gray axis `r = g = b` by
///   `hue` degrees. +120 changes red to green. This is not the HSL hue
///   change: it keeps the mean of the channels, not the HSL lightness.
///   It works for all values, also values outside `0..=1`.
/// - Saturation `s` in `-1..=1`: `L + (c - L) * (1 + s)`, where `L` is the
///   working-space luminance of the encoded color. -1 gives gray.
/// - Lightness `l` in `-1..=1`: for `l >= 0`, `v + (1 - v) * l`; for
///   `l < 0`, `v * (1 + l)`. 1 gives white, -1 gives black.
///
/// The steps apply in that order.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HueSaturation {
    /// Hue rotation in degrees, `-180.0..=180.0`.
    pub hue: f32,
    /// Saturation, `-1.0..=1.0`.
    pub saturation: f32,
    /// Lightness, `-1.0..=1.0`.
    pub lightness: f32,
}

impl HueSaturation {
    /// Checks the parameter ranges.
    pub fn validate(&self) -> EditorResult<()> {
        check_range("hue", self.hue, -180.0, 180.0)?;
        check_range("saturation", self.saturation, -1.0, 1.0)?;
        check_range("lightness", self.lightness, -1.0, 1.0)
    }

    /// Returns the hue rotation matrix (Rodrigues rotation about the unit
    /// vector `(1, 1, 1) / sqrt(3)`).
    fn hue_matrix(&self) -> [[f32; 3]; 3] {
        let theta = f64::from(self.hue).to_radians();
        let (sin, cos) = theta.sin_cos();
        let a = cos + (1.0 - cos) / 3.0;
        let b = (1.0 - cos) / 3.0 - sin / 3.0_f64.sqrt();
        let c = (1.0 - cos) / 3.0 + sin / 3.0_f64.sqrt();
        // The values are in -1..=1, so the narrowing to f32 is safe.
        #[allow(clippy::cast_possible_truncation)]
        let (a, b, c) = (a as f32, b as f32, c as f32);
        [[a, b, c], [c, a, b], [b, c, a]]
    }
}

/// An adjustment layer setting.
#[derive(Debug, Clone, PartialEq)]
pub enum Adjustment {
    /// Brightness and contrast.
    BrightnessContrast(BrightnessContrast),
    /// Levels.
    Levels(Levels),
    /// Curves.
    Curves(Curves),
    /// Hue, saturation, and lightness.
    HueSaturation(HueSaturation),
}

impl Adjustment {
    /// Checks the parameters.
    pub fn validate(&self) -> EditorResult<()> {
        match self {
            Self::BrightnessContrast(p) => p.validate(),
            Self::Levels(p) => p.validate(),
            Self::Curves(p) => p.validate(),
            Self::HueSaturation(p) => p.validate(),
        }
    }

    /// Checks the parameters and precomputes the data for
    /// [`PreparedAdjustment::apply`].
    pub fn prepare(&self) -> EditorResult<PreparedAdjustment> {
        self.validate()?;
        let inner = match self {
            Self::BrightnessContrast(p) => Prepared::BrightnessContrast(p.prepare()),
            Self::Levels(p) => Prepared::Levels(*p),
            Self::Curves(p) => {
                let curves = [
                    MonotoneCubic::new(&p.red)?,
                    MonotoneCubic::new(&p.green)?,
                    MonotoneCubic::new(&p.blue)?,
                ];
                let master = MonotoneCubic::new(&p.master)?;
                Prepared::Curves {
                    master: (!master.is_identity()).then_some(master),
                    channels: Box::new(curves),
                }
            }
            Self::HueSaturation(p) => Prepared::HueSaturation {
                matrix: p.hue_matrix(),
                saturation: p.saturation,
                lightness: p.lightness,
            },
        };
        Ok(PreparedAdjustment { inner })
    }
}

/// An adjustment with precomputed data, ready to apply to pixels. Make it
/// with [`Adjustment::prepare`].
#[derive(Debug, Clone)]
pub struct PreparedAdjustment {
    /// The GPU compositor ([`crate::gpu`]) reads this data to make its
    /// kernel parameters.
    pub(crate) inner: Prepared,
}

#[derive(Debug, Clone)]
pub(crate) enum Prepared {
    BrightnessContrast(PreparedBc),
    Levels(Levels),
    Curves {
        /// Master curve, or `None` if it is the identity.
        master: Option<MonotoneCubic>,
        /// Red, green, and blue curves.
        channels: Box<[MonotoneCubic; 3]>,
    },
    HueSaturation {
        matrix: [[f32; 3]; 3],
        saturation: f32,
        lightness: f32,
    },
}

impl PreparedAdjustment {
    /// Applies the adjustment to a straight linear color.
    pub fn apply(&self, linear: [f32; 3]) -> [f32; 3] {
        use Prepared as P;
        let e = encode(linear);
        let out = match &self.inner {
            P::BrightnessContrast(p) => e.map(|v| extend(v, |x| p.apply(x))),
            P::Levels(l) => {
                let ch = [l.red, l.green, l.blue];
                let mut o = [0.0_f32; 3];
                for i in 0..3 {
                    o[i] = extend(e[i], |x| l.master.apply(ch[i].apply(x)));
                }
                o
            }
            P::Curves { master, channels } => {
                let mut o = [0.0_f32; 3];
                for i in 0..3 {
                    o[i] = extend(e[i], |x| {
                        let y = channels[i].eval(x);
                        master.as_ref().map_or(y, |m| m.eval(y))
                    });
                }
                o
            }
            P::HueSaturation {
                matrix,
                saturation,
                lightness,
            } => {
                let r = expose_core::color::mul_vec_f32(matrix, e);
                let l = luminance(r);
                let s = r.map(|v| l + (v - l) * (1.0 + saturation));
                let lt = *lightness;
                s.map(|v| {
                    extend(v, |x| {
                        if lt >= 0.0 {
                            x + (1.0 - x) * lt
                        } else {
                            x * (1.0 + lt)
                        }
                    })
                })
            }
        };
        decode(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() < eps
    }

    /// Applies an adjustment to an encoded gray value and returns the
    /// encoded result.
    fn apply_encoded(adj: &Adjustment, v: f32) -> f32 {
        let p = adj.prepare().unwrap();
        let lin = TransferFunction::Srgb.decode(v);
        TransferFunction::Srgb.encode(p.apply([lin; 3])[0])
    }

    #[test]
    fn brightness_contrast_hand_computed() {
        let bc = |b, c| {
            Adjustment::BrightnessContrast(BrightnessContrast {
                brightness: b,
                contrast: c,
            })
        };
        // Identity.
        assert!(close(apply_encoded(&bc(0.0, 0.0), 0.3), 0.3, 1e-4));
        // b = 0.5: 0.2 + 0.8 * 0.5 = 0.6.
        assert!(close(apply_encoded(&bc(0.5, 0.0), 0.2), 0.6, 1e-4));
        // b = -0.5: 0.6 * 0.5 = 0.3.
        assert!(close(apply_encoded(&bc(-0.5, 0.0), 0.6), 0.3, 1e-4));
        // c = 1/3: k = tan(pi/3) = 1.7320508; (0.6 - 0.5) * k + 0.5 = 0.6732.
        assert!(close(
            apply_encoded(&bc(0.0, 1.0 / 3.0), 0.6),
            0.673_205,
            1e-4
        ));
        // c = -1: flat mid-gray.
        assert!(close(apply_encoded(&bc(0.0, -1.0), 0.9), 0.5, 1e-4));
        // c = 1: threshold.
        assert!(close(apply_encoded(&bc(0.0, 1.0), 0.51), 1.0, 1e-4));
        assert!(close(apply_encoded(&bc(0.0, 1.0), 0.49), 0.0, 1e-4));
        assert!(bc(1.5, 0.0).validate().is_err());
        assert!(bc(f32::NAN, 0.0).validate().is_err());
    }

    #[test]
    fn levels_hand_computed() {
        let l = Levels {
            master: LevelsChannel {
                input_black: 0.2,
                input_white: 0.6,
                gamma: 2.0,
                output_black: 0.1,
                output_white: 0.9,
            },
            ..Levels::default()
        };
        let adj = Adjustment::Levels(l);
        // v = 0.3: t = 0.25, t^(1/2) = 0.5, out = 0.1 + 0.5 * 0.8 = 0.5.
        assert!(close(apply_encoded(&adj, 0.3), 0.5, 1e-4));
        // v = 0.7 (above input white): out = 0.9.
        assert!(close(apply_encoded(&adj, 0.7), 0.9, 1e-4));
        // Out of range excess: v = 1.5 -> f(1) + 0.5 = 1.4.
        assert!(close(apply_encoded(&adj, 1.5), 1.4, 1e-3));

        let bad = LevelsChannel {
            input_black: 0.2,
            input_white: 0.1,
            ..LevelsChannel::default()
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn monotone_cubic_properties() {
        let pts = [
            CurvePoint::new(0.0, 0.0),
            CurvePoint::new(0.25, 0.4),
            CurvePoint::new(0.5, 0.5),
            CurvePoint::new(1.0, 1.0),
        ];
        let s = MonotoneCubic::new(&pts).unwrap();
        // Passes through the points.
        for p in &pts {
            assert!(close(s.eval(p.x), p.y, 1e-6));
        }
        // Monotone: no overshoot.
        let mut prev = s.eval(0.0);
        for i in 1..=1000 {
            #[allow(clippy::cast_precision_loss)]
            let y = s.eval(i as f32 / 1000.0);
            assert!(y >= prev - 1e-6, "not monotone at {i}");
            prev = y;
        }
        // Flat outside the end points.
        let s2 =
            MonotoneCubic::new(&[CurvePoint::new(0.2, 0.1), CurvePoint::new(0.8, 0.7)]).unwrap();
        assert!(close(s2.eval(0.0), 0.1, 1e-6));
        assert!(close(s2.eval(1.0), 0.7, 1e-6));
        // Linear for two points: midpoint.
        assert!(close(s2.eval(0.5), 0.4, 1e-6));
        // Bad input.
        assert!(MonotoneCubic::new(&[CurvePoint::new(0.0, 0.0)]).is_err());
        assert!(
            MonotoneCubic::new(&[CurvePoint::new(0.5, 0.0), CurvePoint::new(0.5, 1.0)]).is_err()
        );
    }

    #[test]
    fn curves_apply_and_keep_highlights() {
        let c = Curves {
            master: vec![
                CurvePoint::new(0.0, 0.0),
                CurvePoint::new(0.5, 0.75),
                CurvePoint::new(1.0, 1.0),
            ],
            ..Curves::default()
        };
        let adj = Adjustment::Curves(c);
        assert!(close(apply_encoded(&adj, 0.5), 0.75, 1e-4));
        // Above 1: f(1) = 1 plus the excess.
        assert!(close(apply_encoded(&adj, 2.0), 2.0, 1e-3));
        // Identity curves change nothing.
        let id = Adjustment::Curves(Curves::default());
        assert!(close(apply_encoded(&id, 0.37), 0.37, 1e-4));
    }

    #[test]
    fn hue_saturation_hand_computed() {
        let hs = |h, s, l| {
            Adjustment::HueSaturation(HueSaturation {
                hue: h,
                saturation: s,
                lightness: l,
            })
        };
        // +120 degrees: red -> green (in encoded space; 1.0 encodes to 1.0).
        let p = hs(120.0, 0.0, 0.0).prepare().unwrap();
        let out = p.apply([1.0, 0.0, 0.0]);
        assert!(close(out[0], 0.0, 1e-4) && close(out[1], 1.0, 1e-4) && close(out[2], 0.0, 1e-4));
        // Saturation -1 gives gray at the encoded luminance.
        let p = hs(0.0, -1.0, 0.0).prepare().unwrap();
        let out = p.apply([1.0, 0.0, 0.0]);
        let want = TransferFunction::Srgb.decode(0.2627);
        assert!(out.iter().all(|v| close(*v, want, 1e-4)), "{out:?}");
        // Lightness 1 gives white, -1 gives black.
        let p = hs(0.0, 0.0, 1.0).prepare().unwrap();
        assert!(
            p.apply([0.2, 0.3, 0.4])
                .iter()
                .all(|v| close(*v, 1.0, 1e-4))
        );
        let p = hs(0.0, 0.0, -1.0).prepare().unwrap();
        assert!(
            p.apply([0.2, 0.3, 0.4])
                .iter()
                .all(|v| close(*v, 0.0, 1e-6))
        );
        // Identity.
        let p = hs(0.0, 0.0, 0.0).prepare().unwrap();
        let c = [0.2, 3.0, 0.05];
        let out = p.apply(c);
        assert!(out.iter().zip(c.iter()).all(|(a, b)| close(*a, *b, 1e-3)));
        assert!(hs(200.0, 0.0, 0.0).validate().is_err());
    }
}
