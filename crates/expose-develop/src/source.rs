//! Image-specific inputs of the pipeline: the as-shot white point.
//!
//! The develop settings are the same for all photos. Some stages also need
//! data that comes from the source file. The decoder supplies this data in
//! [`SourceInfo`].

use expose_core::color::{
    Chromaticity, ColorSpaceId, Mat3, PLANCKIAN_MAX_KELVIN, PLANCKIAN_MIN_KELVIN, TINT_SCALE,
    WORKING_WHITE, planckian_chromaticity, temperature_tint_to_chromaticity,
};
use expose_core::{CoreError, CoreResult};

/// The white point of the scene light, as the source file records it.
///
/// The develop pipeline adapts this white to D65 (the working white) when the
/// white balance mode is "as shot".
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SourceWhite {
    /// The source is already white balanced for display (JPEG, PNG, TIFF, and
    /// other rendered files). Its neutral colors are D65 in the working
    /// space. The as-shot white balance is the identity.
    #[default]
    Rendered,
    /// The CIE 1931 xy chromaticity of the scene light.
    Chromaticity(Chromaticity),
    /// The linear working-space (Rec. 2020) RGB value of a neutral (gray)
    /// surface under the scene light ("as-shot neutral"). Only the ratio of
    /// the three values is important.
    NeutralRgb([f64; 3]),
}

impl SourceWhite {
    /// Makes the source white from camera white balance multipliers.
    ///
    /// - `multipliers`: the camera RGB gains that make a neutral surface
    ///   gray (for example from the RAW metadata). All three must be finite
    ///   and greater than 0.
    /// - `camera_to_working`: the matrix from camera RGB to linear working
    ///   RGB, without white balance.
    ///
    /// The neutral in camera RGB is `1 / multiplier` for each channel. The
    /// function converts it to the working space.
    pub fn from_camera_multipliers(
        multipliers: [f64; 3],
        camera_to_working: &Mat3,
    ) -> CoreResult<Self> {
        let mut neutral = [0.0; 3];
        for (n, m) in neutral.iter_mut().zip(multipliers) {
            if !m.is_finite() {
                return Err(CoreError::NonFinite {
                    name: "white balance multiplier",
                });
            }
            if m <= 0.0 {
                return Err(CoreError::OutOfRange {
                    name: "white balance multiplier",
                    value: m,
                    min: f64::MIN_POSITIVE,
                    max: f64::MAX,
                });
            }
            *n = 1.0 / m;
        }
        Ok(Self::NeutralRgb(camera_to_working.mul_vec(neutral)))
    }

    /// Returns the chromaticity of the scene light.
    ///
    /// [`SourceWhite::Rendered`] gives D65. Returns an error if the value
    /// does not give a valid chromaticity (for example a neutral with a sum
    /// of zero, or a non-finite value).
    pub fn chromaticity(&self) -> CoreResult<Chromaticity> {
        match *self {
            Self::Rendered => Ok(WORKING_WHITE),
            Self::Chromaticity(c) => {
                // `to_xyz` checks that the coordinate is finite and y > 0.
                c.to_xyz()?;
                Ok(c)
            }
            Self::NeutralRgb(rgb) => {
                if rgb.iter().any(|v| !v.is_finite()) {
                    return Err(CoreError::NonFinite {
                        name: "as-shot neutral",
                    });
                }
                let to_xyz = ColorSpaceId::LinearRec2020.definition().to_xyz()?;
                let c = Chromaticity::from_xyz(to_xyz.mul_vec(rgb))?;
                c.to_xyz()?;
                Ok(c)
            }
        }
    }
}

/// Image-specific inputs of the develop pipeline.
///
/// `Default` is a rendered source (as-shot white balance is the identity).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SourceInfo {
    /// The white point of the scene light (the "as shot" white balance).
    pub as_shot: SourceWhite,
}

impl SourceInfo {
    /// Information for a rendered source (JPEG, PNG, TIFF): as shot is D65.
    pub const RENDERED: Self = Self {
        as_shot: SourceWhite::Rendered,
    };

    /// Returns the as-shot white as `(temperature in kelvin, tint)`, so that
    /// the UI can show the as-shot values on the sliders.
    ///
    /// See [`chromaticity_to_temperature_tint`].
    pub fn as_shot_temperature_tint(&self) -> CoreResult<(f64, f64)> {
        chromaticity_to_temperature_tint(self.as_shot.chromaticity()?)
    }
}

/// Converts CIE 1931 xy to CIE 1960 UCS uv.
fn to_uv(c: Chromaticity) -> (f64, f64) {
    let d = -2.0 * c.x + 12.0 * c.y + 3.0;
    (4.0 * c.x / d, 6.0 * c.y / d)
}

/// The number of coarse search steps on the mired scale.
const COARSE_STEPS: u32 = 256;
/// The number of golden-section refinement steps.
const REFINE_STEPS: u32 = 80;

/// Converts a white point chromaticity to `(temperature in kelvin, tint)`.
///
/// This is the inverse of
/// [`expose_core::color::temperature_tint_to_chromaticity`]:
///
/// 1. Find the temperature whose Planckian point is nearest to `c` in CIE
///    1960 uv. The search is on the mired scale (1e6 / kelvin), first with
///    a coarse scan, then with a golden-section search.
/// 2. The start tint is the signed distance from that point along the unit
///    normal of the locus (positive v is green): `tint = -Duv * TINT_SCALE`.
/// 3. Refine both values with Newton steps on the forward function, so
///    that the forward function gives `c` again. The result is the pair
///    with the smallest round-trip error.
///
/// If the nearest point is outside the Planckian range (1667 K to
/// 25000 K), the result is the nearest end of the range. Then the round
/// trip is not exact. Near 2222 K and 4000 K (the kinks of the locus
/// approximation), the forward function is not continuous; there the
/// round-trip error can be up to `1e-3` in uv. Elsewhere it is below
/// `1e-9`. The tint is not clamped to the slider range; the
/// caller must clamp it.
pub fn chromaticity_to_temperature_tint(c: Chromaticity) -> CoreResult<(f64, f64)> {
    c.to_xyz()?;
    let target = to_uv(c);
    let distance = |mired: f64| -> CoreResult<f64> {
        let (u, v) = to_uv(planckian_chromaticity(1e6 / mired)?);
        Ok((u - target.0).powi(2) + (v - target.1).powi(2))
    };
    let lo = 1e6 / PLANCKIAN_MAX_KELVIN;
    let hi = 1e6 / PLANCKIAN_MIN_KELVIN;
    let step = (hi - lo) / f64::from(COARSE_STEPS);

    // Coarse scan.
    let mut best = (0_u32, f64::INFINITY);
    for i in 0..=COARSE_STEPS {
        let d = distance(lo + step * f64::from(i))?;
        if d < best.1 {
            best = (i, d);
        }
    }
    // Golden-section search in the neighbor interval of the best sample.
    let mut a = (lo + step * (f64::from(best.0) - 1.0)).max(lo);
    let mut b = (lo + step * (f64::from(best.0) + 1.0)).min(hi);
    let ratio = (5.0_f64.sqrt() - 1.0) / 2.0;
    for _ in 0..REFINE_STEPS {
        let m1 = b - ratio * (b - a);
        let m2 = a + ratio * (b - a);
        if distance(m1)? < distance(m2)? {
            b = m2;
        } else {
            a = m1;
        }
    }
    let mired = 0.5 * (a + b);
    let frame = locus_frame(1e6 / mired)?;
    let duv = (target.0 - frame.u) * frame.nu + (target.1 - frame.v) * frame.nv;
    let start = (mired, -duv * TINT_SCALE);

    // Newton refinement on the forward function itself. Near a kink of
    // the locus approximation (2222 K and 4000 K) with a large tint, the
    // nearest point is not exactly the foot of the forward normal, so the
    // start value does not round trip. The search keeps the pair with the
    // smallest round-trip error (the start value is a candidate too).
    let residual = |m: f64, t: f64| -> CoreResult<(f64, f64)> {
        let kelvin = (1e6 / m).clamp(PLANCKIAN_MIN_KELVIN, PLANCKIAN_MAX_KELVIN);
        let (u, v) = to_uv(temperature_tint_to_chromaticity(kelvin, t)?);
        Ok((u - target.0, v - target.1))
    };
    let norm = |r: (f64, f64)| r.0.hypot(r.1);
    let (mut m, mut t) = start;
    let mut best = (m, t, norm(residual(m, t)?));
    for _ in 0..NEWTON_STEPS {
        let r = residual(m, t)?;
        if norm(r) < best.2 {
            best = (m, t, norm(r));
        }
        if norm(r) < NEWTON_TOLERANCE {
            break;
        }
        // Forward differences, inward at the end of the range.
        let hm = if m + NEWTON_H_MIRED <= hi {
            NEWTON_H_MIRED
        } else {
            -NEWTON_H_MIRED
        };
        let rm = residual(m + hm, t)?;
        let rt = residual(m, t + NEWTON_H_TINT)?;
        let j = [
            [(rm.0 - r.0) / hm, (rt.0 - r.0) / NEWTON_H_TINT],
            [(rm.1 - r.1) / hm, (rt.1 - r.1) / NEWTON_H_TINT],
        ];
        let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
        if !det.is_finite() || det.abs() < f64::MIN_POSITIVE {
            break;
        }
        let dm = (-r.0 * j[1][1] + r.1 * j[0][1]) / det;
        let dt = (-r.1 * j[0][0] + r.0 * j[1][0]) / det;
        m = (m + dm).clamp(lo, hi);
        t += dt;
    }
    let r = residual(m, t)?;
    if norm(r) < best.2 {
        best = (m, t, norm(r));
    }
    let kelvin = (1e6 / best.0).clamp(PLANCKIAN_MIN_KELVIN, PLANCKIAN_MAX_KELVIN);
    Ok((kelvin, best.1))
}

/// The maximum number of Newton steps.
const NEWTON_STEPS: u32 = 30;
/// Newton stops when the uv error is below this value.
const NEWTON_TOLERANCE: f64 = 1e-13;
/// The finite difference step in mired.
const NEWTON_H_MIRED: f64 = 1e-4;
/// The finite difference step in tint units.
const NEWTON_H_TINT: f64 = 1e-3;

/// A point on the Planckian locus in uv, with its unit normal (toward
/// positive v).
struct LocusFrame {
    u: f64,
    v: f64,
    nu: f64,
    nv: f64,
}

/// Returns the locus frame at `kelvin`. The normal comes from a 1 K finite
/// difference tangent, the same as in `temperature_tint_to_chromaticity`.
fn locus_frame(kelvin: f64) -> CoreResult<LocusFrame> {
    let on_locus = planckian_chromaticity(kelvin)?;
    let (u, v) = to_uv(on_locus);
    let step = if kelvin + 1.0 <= PLANCKIAN_MAX_KELVIN {
        1.0
    } else {
        -1.0
    };
    let (u2, v2) = to_uv(planckian_chromaticity(kelvin + step)?);
    let (du, dv) = (u2 - u, v2 - v);
    let len = du.hypot(dv);
    if len < f64::MIN_POSITIVE {
        return Err(CoreError::InvalidChromaticity {
            x: on_locus.x,
            y: on_locus.y,
        });
    }
    let (mut nu, mut nv) = (-dv / len, du / len);
    if nv < 0.0 {
        nu = -nu;
        nv = -nv;
    }
    Ok(LocusFrame { u, v, nu, nv })
}

#[cfg(test)]
mod tests {
    use super::*;
    use expose_core::color::white;

    #[test]
    fn rendered_source_is_d65() {
        assert_eq!(SourceInfo::default().as_shot, SourceWhite::Rendered);
        assert_eq!(SourceWhite::Rendered.chromaticity().unwrap(), white::D65);
    }

    #[test]
    fn temperature_tint_round_trip() {
        // 2222 K and 4000 K are the kinks of the locus approximation. Near
        // a kink, two (temperature, tint) pairs can give the same white, so
        // there only the chromaticity must round trip.
        let near_kink = [2222.0, 3999.0, 4000.0, 4001.0];
        for kelvin in [
            2000.0, 2222.0, 2850.0, 3999.0, 4000.0, 4001.0, 5500.0, 6500.0, 9000.0, 20000.0,
        ] {
            for tint in [-150.0, -100.0, -10.0, 0.0, 25.0, 120.0, 150.0] {
                let c = temperature_tint_to_chromaticity(kelvin, tint).unwrap();
                let (k, t) = chromaticity_to_temperature_tint(c).unwrap();
                let back = to_uv(temperature_tint_to_chromaticity(k, t).unwrap());
                let uv = to_uv(c);
                // At a kink, the forward function is not continuous: the
                // locus has a small step at 2222 K, and from 3999 K to
                // 4000 K its 1 K tangent crosses the kink (the normal
                // turns, and points inside the turn have no exact inverse).
                // There, allow 1e-3 in uv (3 tint units).
                let limit = if near_kink.contains(&kelvin) {
                    1e-3
                } else {
                    1e-9
                };
                let error = (back.0 - uv.0).hypot(back.1 - uv.1);
                assert!(
                    error < limit,
                    "{kelvin} K, tint {tint}: got {k} K, {t} (uv error {error})"
                );
                if near_kink.contains(&kelvin) {
                    continue;
                }
                assert!(
                    (k - kelvin).abs() / kelvin < 1e-4,
                    "{kelvin} K, tint {tint}: got {k} K"
                );
                assert!((t - tint).abs() < 0.05, "{kelvin} K, tint {tint}: got {t}");
            }
        }
    }

    #[test]
    fn d65_is_near_6500_kelvin_and_green_of_the_locus() {
        let (k, t) = SourceInfo::RENDERED.as_shot_temperature_tint().unwrap();
        assert!((6400.0..6600.0).contains(&k), "{k}");
        // D65 is above the Planckian locus (toward green): negative tint.
        assert!(t < 0.0, "{t}");
    }

    #[test]
    fn neutral_rgb_of_white_is_d65() {
        let c = SourceWhite::NeutralRgb([0.5, 0.5, 0.5])
            .chromaticity()
            .unwrap();
        assert!((c.x - white::D65.x).abs() < 1e-4);
        assert!((c.y - white::D65.y).abs() < 1e-4);
        assert!(SourceWhite::NeutralRgb([0.0; 3]).chromaticity().is_err());
        assert!(
            SourceWhite::NeutralRgb([f64::NAN, 1.0, 1.0])
                .chromaticity()
                .is_err()
        );
    }

    #[test]
    fn camera_multipliers_give_neutral() {
        let w = SourceWhite::from_camera_multipliers([2.0, 1.0, 1.5], &Mat3::IDENTITY).unwrap();
        let SourceWhite::NeutralRgb(n) = w else {
            panic!("expected a neutral");
        };
        assert!((n[0] - 0.5).abs() < 1e-12 && (n[2] - 1.0 / 1.5).abs() < 1e-12);
        assert!(SourceWhite::from_camera_multipliers([0.0, 1.0, 1.0], &Mat3::IDENTITY).is_err());
        assert!(
            SourceWhite::from_camera_multipliers([f64::INFINITY, 1.0, 1.0], &Mat3::IDENTITY)
                .is_err()
        );
    }
}
