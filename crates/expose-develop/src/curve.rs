//! Point tone curve: monotone cubic interpolation, encoding, and LUTs.
//!
//! # Interpolation
//!
//! The curve goes through all control points. Between points it is a cubic
//! Hermite spline with Fritsch–Carlson tangents (monotone cubic
//! interpolation). Where the points increase, the curve increases; where
//! they decrease, it decreases. The curve does not overshoot the points.
//!
//! # Encoding
//!
//! The curve works on display-like values, not on linear values. For each
//! channel the pipeline does: encode with the sRGB transfer function, apply
//! the master curve, apply the curve of the channel, decode. The curves act
//! on R, G, and B separately (this is the expected behavior of a point
//! curve: a master S-curve also adds saturation).
//!
//! # Range
//!
//! - Inside `0..=1`, before the first point and after the last point, the
//!   curve is flat (the first or last `y`). This gives the expected "black
//!   point" and "white point" behavior.
//! - Below 0 and above 1 (scene-referred values): the curve continues as a
//!   straight line. The slope is the end tangent when the end point is at
//!   `x = 0` (or `x = 1`), else 0 (the curve is flat there). Thus an identity
//!   end keeps values above 1.0, and a moved end point clips, as the user
//!   asked.
//!
//! # LUT
//!
//! The pipeline samples each curve at [`CURVE_LUT_SIZE`] points in `0..=1`
//! and interpolates linearly between them. The error of the LUT is less
//! than `1e-6` for normal curves.

use expose_core::develop::{CurvePoint, ToneCurve};
use expose_core::{CoreError, CoreResult};

/// The names of the four curves, in table order.
const CHANNEL_NAMES: [&str; 4] = ["master", "red", "green", "blue"];

/// The number of samples in each curve LUT.
pub const CURVE_LUT_SIZE: usize = 1024;

/// The index of the last LUT sample, as `f32`.
#[allow(clippy::cast_precision_loss, reason = "1023 is exact in f32")]
pub(crate) const CURVE_LAST_INDEX_F32: f32 = (CURVE_LUT_SIZE - 1) as f32;

/// A monotone cubic curve through control points.
#[derive(Debug, Clone, PartialEq)]
pub struct MonotoneCurve {
    xs: Vec<f64>,
    ys: Vec<f64>,
    /// Tangent at each point.
    ms: Vec<f64>,
}

impl MonotoneCurve {
    /// Makes the curve. The points must obey the rules of
    /// [`ToneCurve`] (2 or more points, `x` strictly increasing, all values
    /// finite and in `0..=1`). Call `ToneCurve::validate` first.
    ///
    /// Returns `None` if there are fewer than 2 points.
    pub fn new(points: &[CurvePoint]) -> Option<Self> {
        if points.len() < 2 {
            return None;
        }
        let xs: Vec<f64> = points.iter().map(|p| f64::from(p.x)).collect();
        let ys: Vec<f64> = points.iter().map(|p| f64::from(p.y)).collect();
        let n = xs.len();
        let secants: Vec<f64> = (0..n - 1)
            .map(|k| (ys[k + 1] - ys[k]) / (xs[k + 1] - xs[k]))
            .collect();

        // Initial tangents: one-sided at the ends, the mean of the secants
        // inside, and 0 at a local extremum.
        let mut ms = vec![0.0; n];
        ms[0] = secants[0];
        ms[n - 1] = secants[n - 2];
        for k in 1..n - 1 {
            let (a, b) = (secants[k - 1], secants[k]);
            ms[k] = if a * b <= 0.0 { 0.0 } else { 0.5 * (a + b) };
        }
        // Fritsch–Carlson: limit the tangents so that each segment is
        // monotone.
        for k in 0..n - 1 {
            let d = secants[k];
            if d == 0.0 {
                ms[k] = 0.0;
                ms[k + 1] = 0.0;
                continue;
            }
            let alpha = ms[k] / d;
            let beta = ms[k + 1] / d;
            let s = alpha * alpha + beta * beta;
            if s > 9.0 {
                let tau = 3.0 / s.sqrt();
                ms[k] = tau * alpha * d;
                ms[k + 1] = tau * beta * d;
            }
        }
        Some(Self { xs, ys, ms })
    }

    /// Evaluates the curve at `x` in `0..=1` (flat outside the points).
    pub fn eval(&self, x: f64) -> f64 {
        let n = self.xs.len();
        if x <= self.xs[0] {
            return self.ys[0];
        }
        if x >= self.xs[n - 1] {
            return self.ys[n - 1];
        }
        // The segment k with xs[k] <= x < xs[k + 1]. `x` is strictly inside
        // the point range here, so 1 <= p <= n - 1 and k is valid.
        let k = self.xs.partition_point(|&xk| xk <= x) - 1;
        let h = self.xs[k + 1] - self.xs[k];
        let t = (x - self.xs[k]) / h;
        let (t2, t3) = (t * t, t * t * t);
        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;
        h00 * self.ys[k] + h10 * h * self.ms[k] + h01 * self.ys[k + 1] + h11 * h * self.ms[k + 1]
    }

    /// The slope of the straight line below `x = 0`.
    #[allow(
        clippy::float_cmp,
        reason = "an end point exactly at 0 is a user choice"
    )]
    pub fn slope_below(&self) -> f64 {
        if self.xs[0] == 0.0 { self.ms[0] } else { 0.0 }
    }

    /// The slope of the straight line above `x = 1`.
    #[allow(
        clippy::float_cmp,
        reason = "an end point exactly at 1 is a user choice"
    )]
    pub fn slope_above(&self) -> f64 {
        let n = self.xs.len();
        if self.xs[n - 1] == 1.0 {
            self.ms[n - 1]
        } else {
            0.0
        }
    }
}

/// Returns `true` if the points describe the identity `y = x` on `0..=1`.
///
/// All points must be on the diagonal, and the first and last points must
/// be at 0 and 1. Then all secants are 1, all tangents are 1, and the
/// curve is the line `y = x`.
#[allow(
    clippy::float_cmp,
    reason = "only an exact identity may be skipped; a near identity is a real edit"
)]
pub fn is_identity(points: &[CurvePoint]) -> bool {
    points.len() >= 2
        && points.iter().all(|p| p.x == p.y)
        && points.first().is_some_and(|p| p.x == 0.0)
        && points.last().is_some_and(|p| p.x == 1.0)
}

/// The sampled curves of a [`ToneCurve`], for the CPU and the GPU.
#[derive(Debug, Clone, PartialEq)]
pub struct CurveTables {
    /// Which curves are active: master, red, green, blue. An identity curve
    /// is not active, and the pipeline skips it.
    pub active: [bool; 4],
    /// For each curve: `[y at 0, slope below 0, y at 1, slope above 1]`.
    pub ends: [[f32; 4]; 4],
    /// Sample `i` is at `x = i / (CURVE_LUT_SIZE - 1)`. It holds the values
    /// of the master, red, green, and blue curves.
    pub lut: Box<[[f32; 4]; CURVE_LUT_SIZE]>,
}

impl CurveTables {
    /// Builds the tables. Returns `Ok(None)` if all four curves are the
    /// identity. Returns an error if the curve is not valid.
    pub fn build(curve: &ToneCurve) -> CoreResult<Option<Self>> {
        curve.validate()?;
        let channels = [&curve.master, &curve.red, &curve.green, &curve.blue];
        let active = channels.map(|points| !is_identity(points));
        if !active.iter().any(|a| *a) {
            return Ok(None);
        }
        // Identity curves keep these values: the LUT and ends of y = x.
        let mut ends = [[0.0_f32, 1.0, 1.0, 1.0]; 4];
        let mut lut = Box::new([[0.0_f32; 4]; CURVE_LUT_SIZE]);
        for (i, sample) in lut.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "i < 1024 is exact")]
            let x = i as f32 / CURVE_LAST_INDEX_F32;
            *sample = [x; 4];
        }
        for (c, points) in channels.iter().enumerate() {
            if !active[c] {
                continue;
            }
            // `validate` above guarantees 2 or more points, so `new` gives
            // `Some`. The `else` branch is for safety only.
            let Some(spline) = MonotoneCurve::new(points) else {
                return Err(CoreError::InvalidToneCurve {
                    channel: CHANNEL_NAMES[c],
                    reason: "fewer than 2 points",
                });
            };
            #[allow(clippy::cast_possible_truncation, reason = "LUT precision is f32")]
            for (i, sample) in lut.iter_mut().enumerate() {
                #[allow(clippy::cast_precision_loss, reason = "i < 1024 is exact")]
                let x = i as f64 / f64::from(CURVE_LAST_INDEX_F32);
                sample[c] = spline.eval(x) as f32;
            }
            #[allow(clippy::cast_possible_truncation, reason = "LUT precision is f32")]
            {
                ends[c] = [
                    spline.eval(0.0) as f32,
                    spline.slope_below() as f32,
                    spline.eval(1.0) as f32,
                    spline.slope_above() as f32,
                ];
            }
        }
        Ok(Some(Self { active, ends, lut }))
    }

    /// Applies curve `c` (0 = master, 1 = red, 2 = green, 3 = blue) to the
    /// encoded value `x`. The GPU kernel uses the same arithmetic.
    #[inline]
    pub(crate) fn apply(&self, c: usize, x: f32) -> f32 {
        let ends = self.ends[c];
        if x < 0.0 {
            return ends[0] + ends[1] * x;
        }
        if x > 1.0 {
            return ends[2] + ends[3] * (x - 1.0);
        }
        // NaN fails both tests above and gets here. `t` is then NaN, the
        // cast gives 0, and the result is NaN (no panic).
        let t = x * CURVE_LAST_INDEX_F32;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "x is in 0..=1, so t is in 0..=1023"
        )]
        let i = (t.floor() as usize).min(CURVE_LUT_SIZE - 2);
        #[allow(clippy::cast_precision_loss, reason = "i < 1024 is exact")]
        let f = t - i as f32;
        // `i + 1 < CURVE_LUT_SIZE` from the `min` above.
        let a = self.lut[i][c];
        let b = self.lut[i + 1][c];
        a + f * (b - a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use expose_core::develop::identity_curve;

    fn pts(v: &[(f32, f32)]) -> Vec<CurvePoint> {
        v.iter().map(|&(x, y)| CurvePoint::new(x, y)).collect()
    }

    #[test]
    fn passes_through_points() {
        let p = pts(&[(0.0, 0.05), (0.3, 0.2), (0.6, 0.75), (1.0, 0.95)]);
        let c = MonotoneCurve::new(&p).unwrap();
        for q in &p {
            assert!((c.eval(f64::from(q.x)) - f64::from(q.y)).abs() < 1e-12);
        }
    }

    #[test]
    fn monotone_data_gives_monotone_curve() {
        // Data with a steep step and flat parts, where a natural cubic
        // spline overshoots.
        let p = pts(&[(0.0, 0.0), (0.2, 0.0), (0.25, 0.9), (0.7, 0.92), (1.0, 1.0)]);
        let c = MonotoneCurve::new(&p).unwrap();
        let mut prev = c.eval(0.0);
        for i in 1..=10_000 {
            let y = c.eval(f64::from(i) / 10_000.0);
            assert!(y >= prev - 1e-12, "not monotone at {i}");
            assert!((0.0..=1.0).contains(&y), "overshoot {y}");
            prev = y;
        }
    }

    #[test]
    fn flat_outside_points_and_end_slopes() {
        let c = MonotoneCurve::new(&pts(&[(0.1, 0.2), (0.9, 0.8)])).unwrap();
        // The points are f32, so compare with f32 precision.
        assert!((c.eval(0.0) - 0.2).abs() < 1e-7);
        assert!((c.eval(1.0) - 0.8).abs() < 1e-7);
        assert!(c.slope_below().abs() < 1e-12 && c.slope_above().abs() < 1e-12);
        let id = MonotoneCurve::new(&identity_curve()).unwrap();
        assert!((id.slope_below() - 1.0).abs() < 1e-12);
        assert!((id.slope_above() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn identity_detection() {
        assert!(is_identity(&identity_curve()));
        assert!(is_identity(&pts(&[(0.0, 0.0), (0.5, 0.5), (1.0, 1.0)])));
        assert!(!is_identity(&pts(&[(0.0, 0.0), (0.5, 0.6), (1.0, 1.0)])));
        assert!(!is_identity(&pts(&[(0.1, 0.1), (1.0, 1.0)])));
        assert!(CurveTables::build(&ToneCurve::default()).unwrap().is_none());
        let bad = ToneCurve {
            red: pts(&[(0.5, 0.5)]),
            ..ToneCurve::default()
        };
        assert!(CurveTables::build(&bad).is_err());
    }

    #[test]
    fn tables_match_spline_and_extend_linearly() {
        let curve = ToneCurve {
            master: pts(&[(0.0, 0.0), (0.25, 0.2), (0.75, 0.85), (1.0, 1.0)]),
            blue: pts(&[(0.0, 0.1), (1.0, 0.9)]),
            ..ToneCurve::default()
        };
        let t = CurveTables::build(&curve).unwrap().unwrap();
        assert_eq!(t.active, [true, false, false, true]);
        let master = MonotoneCurve::new(&curve.master).unwrap();
        for i in 0..=1000 {
            let x = f64::from(i) / 1000.0;
            #[allow(clippy::cast_possible_truncation)]
            let got = f64::from(t.apply(0, x as f32));
            assert!((got - master.eval(x)).abs() < 1e-5, "x={x}");
        }
        // Above 1.0 the master continues with its end tangent.
        let slope = master.slope_above();
        #[allow(clippy::cast_possible_truncation)]
        let expected = (1.0 + slope * 0.5) as f32;
        assert!((t.apply(0, 1.5) - expected).abs() < 1e-6);
        // The blue line has slope 0.8 and both ends at 0 and 1.
        assert!((t.apply(3, -0.5) - (0.1 - 0.4)).abs() < 1e-6);
        assert!((t.apply(3, 2.0) - (0.9 + 0.8)).abs() < 1e-6);
        // The identity red channel is y = x everywhere.
        assert!((t.apply(1, 0.37) - 0.37).abs() < 1e-6);
        assert!((t.apply(1, 3.0) - 3.0).abs() < 1e-6);
    }
}
