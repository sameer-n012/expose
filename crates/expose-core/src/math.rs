//! Shared numeric helpers.

use crate::develop::CurvePoint;

/// A monotone cubic spline through control points.
///
/// The spline is a cubic Hermite spline with Fritsch–Carlson tangents. Where
/// the points are monotone, the spline is monotone too (no overshoot). The
/// spline is flat outside the first and last point. Math runs in `f64`.
#[derive(Debug, Clone, PartialEq)]
pub struct MonotoneSpline {
    xs: Vec<f64>,
    ys: Vec<f64>,
    /// Tangent at each point.
    ms: Vec<f64>,
}

impl MonotoneSpline {
    /// Makes the spline. The `x` values must be strictly increasing and all
    /// values must be finite (see `ToneCurve::validate`).
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

    /// Evaluates the spline at `x` (flat outside the points).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_points_and_is_flat_outside() {
        let p = [
            CurvePoint::new(0.2, 0.1),
            CurvePoint::new(0.5, 0.4),
            CurvePoint::new(0.8, 0.7),
        ];
        let s = MonotoneSpline::new(&p).unwrap();
        for q in &p {
            assert!((s.eval(f64::from(q.x)) - f64::from(q.y)).abs() < 1e-6);
        }
        assert!((s.eval(0.0) - 0.1).abs() < 1e-6);
        assert!((s.eval(1.0) - 0.7).abs() < 1e-6);
        assert!(s.slope_below().abs() < 1e-12);
    }

    #[test]
    fn fewer_than_two_points_is_none() {
        assert!(MonotoneSpline::new(&[CurvePoint::new(0.0, 0.0)]).is_none());
    }
}
