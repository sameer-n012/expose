//! Chromaticity coordinates, primaries, white points, and chromatic adaptation.

use crate::color::matrix::Mat3;
use crate::error::{CoreError, CoreResult};

/// A CIE 1931 xy chromaticity coordinate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chromaticity {
    /// CIE 1931 x.
    pub x: f64,
    /// CIE 1931 y.
    pub y: f64,
}

impl Chromaticity {
    /// Makes a chromaticity coordinate. This function does no validation.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Converts to CIE XYZ with luminance `Y = 1`.
    ///
    /// Returns [`CoreError::InvalidChromaticity`] if a coordinate is not
    /// finite or `y` is not greater than zero.
    pub fn to_xyz(self) -> CoreResult<[f64; 3]> {
        if !self.x.is_finite() || !self.y.is_finite() || self.y <= 0.0 {
            return Err(CoreError::InvalidChromaticity {
                x: self.x,
                y: self.y,
            });
        }
        Ok([self.x / self.y, 1.0, (1.0 - self.x - self.y) / self.y])
    }

    /// Converts CIE XYZ to a chromaticity coordinate.
    ///
    /// Returns [`CoreError::InvalidChromaticity`] if `X + Y + Z` is zero or
    /// not finite.
    pub fn from_xyz(xyz: [f64; 3]) -> CoreResult<Self> {
        let sum = xyz[0] + xyz[1] + xyz[2];
        if !sum.is_finite() || sum.abs() < f64::EPSILON {
            return Err(CoreError::InvalidChromaticity {
                x: f64::NAN,
                y: f64::NAN,
            });
        }
        Ok(Self::new(xyz[0] / sum, xyz[1] / sum))
    }

    /// Converts to CIE 1960 UCS (u, v).
    fn to_uv(self) -> (f64, f64) {
        let d = -2.0 * self.x + 12.0 * self.y + 3.0;
        (4.0 * self.x / d, 6.0 * self.y / d)
    }

    /// Converts from CIE 1960 UCS (u, v).
    fn from_uv(u: f64, v: f64) -> Self {
        let d = 2.0 * u - 8.0 * v + 4.0;
        Self::new(3.0 * u / d, 2.0 * v / d)
    }
}

/// Standard white points.
pub mod white {
    use super::Chromaticity;

    /// CIE standard illuminant D65 (as used by sRGB and Rec. 2020).
    pub const D65: Chromaticity = Chromaticity::new(0.3127, 0.3290);

    /// CIE standard illuminant D50 (as used by `ProPhoto` RGB and ICC profiles).
    pub const D50: Chromaticity = Chromaticity::new(0.345_67, 0.358_50);
}

/// The chromaticities of the red, green, and blue primaries of an RGB space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Primaries {
    /// Red primary.
    pub red: Chromaticity,
    /// Green primary.
    pub green: Chromaticity,
    /// Blue primary.
    pub blue: Chromaticity,
}

impl Primaries {
    /// Derives the RGB to CIE XYZ matrix for these primaries and a white point.
    ///
    /// This is the standard derivation: the columns are the XYZ values of
    /// the primaries, scaled so that RGB (1, 1, 1) maps to the white point
    /// with `Y = 1`.
    pub fn rgb_to_xyz(&self, white: Chromaticity) -> CoreResult<Mat3> {
        let unscaled = Mat3::from_columns(
            self.red.to_xyz()?,
            self.green.to_xyz()?,
            self.blue.to_xyz()?,
        );
        let scale = unscaled.inverse()?.mul_vec(white.to_xyz()?);
        Ok(unscaled * Mat3::diagonal(scale))
    }
}

/// The Bradford cone response matrix (XYZ to sharpened LMS).
const BRADFORD: Mat3 = Mat3::new([
    [0.8951, 0.2664, -0.1614],
    [-0.7502, 1.7135, 0.0367],
    [0.0389, -0.0685, 1.0296],
]);

/// Returns the Bradford chromatic adaptation matrix from `src` white to
/// `dst` white.
///
/// The matrix converts XYZ under `src` to XYZ under `dst`. It maps the XYZ
/// of `src` exactly to the XYZ of `dst`.
pub fn bradford_adaptation(src: Chromaticity, dst: Chromaticity) -> CoreResult<Mat3> {
    let src_lms = BRADFORD.mul_vec(src.to_xyz()?);
    let dst_lms = BRADFORD.mul_vec(dst.to_xyz()?);
    let mut scale = [0.0; 3];
    for i in 0..3 {
        if src_lms[i].abs() < f64::EPSILON {
            return Err(CoreError::InvalidChromaticity { x: src.x, y: src.y });
        }
        scale[i] = dst_lms[i] / src_lms[i];
    }
    Ok(BRADFORD.inverse()? * Mat3::diagonal(scale) * BRADFORD)
}

/// Lowest color temperature, in kelvin, of [`planckian_chromaticity`].
pub const PLANCKIAN_MIN_KELVIN: f64 = 1667.0;
/// Highest color temperature, in kelvin, of [`planckian_chromaticity`].
pub const PLANCKIAN_MAX_KELVIN: f64 = 25000.0;
/// Lowest color temperature, in kelvin, of [`daylight_chromaticity`].
pub const DAYLIGHT_MIN_KELVIN: f64 = 4000.0;
/// Highest color temperature, in kelvin, of [`daylight_chromaticity`].
pub const DAYLIGHT_MAX_KELVIN: f64 = 25000.0;

/// Tint units per unit of `Duv` (distance from the Planckian locus in CIE
/// 1960 uv). A tint of +150 is a `Duv` of -0.05.
pub const TINT_SCALE: f64 = 3000.0;

fn check_kelvin(kelvin: f64, min: f64, max: f64) -> CoreResult<()> {
    if !kelvin.is_finite() {
        return Err(CoreError::NonFinite { name: "kelvin" });
    }
    if !(min..=max).contains(&kelvin) {
        return Err(CoreError::OutOfRange {
            name: "kelvin",
            value: kelvin,
            min,
            max,
        });
    }
    Ok(())
}

/// Returns the chromaticity of a black-body (Planckian) radiator.
///
/// This function uses the cubic spline approximation of Kim et al. (2002).
/// The approximation is valid from 1667 K to 25000 K. Other values give
/// [`CoreError::OutOfRange`].
pub fn planckian_chromaticity(kelvin: f64) -> CoreResult<Chromaticity> {
    check_kelvin(kelvin, PLANCKIAN_MIN_KELVIN, PLANCKIAN_MAX_KELVIN)?;
    let t = kelvin;
    let (t2, t3) = (t * t, t * t * t);
    let x = if t <= 4000.0 {
        -0.266_123_9e9 / t3 - 0.234_358_9e6 / t2 + 0.877_695_6e3 / t + 0.179_910
    } else {
        -3.025_846_9e9 / t3 + 2.107_037_9e6 / t2 + 0.222_634_7e3 / t + 0.240_390
    };
    let (x2, x3) = (x * x, x * x * x);
    let y = if t <= 2222.0 {
        -1.106_381_4 * x3 - 1.348_110_20 * x2 + 2.185_558_32 * x - 0.202_196_83
    } else if t <= 4000.0 {
        -0.954_947_6 * x3 - 1.374_185_93 * x2 + 2.091_370_15 * x - 0.167_488_67
    } else {
        3.081_758_0 * x3 - 5.873_386_70 * x2 + 3.751_129_97 * x - 0.370_014_83
    };
    Ok(Chromaticity::new(x, y))
}

/// Returns the chromaticity of the CIE daylight locus (D-series illuminants).
///
/// This function uses the CIE 15 formula. It is valid from 4000 K to
/// 25000 K. Other values give [`CoreError::OutOfRange`]. Note: D65 is at
/// 6504 K on this scale.
pub fn daylight_chromaticity(kelvin: f64) -> CoreResult<Chromaticity> {
    check_kelvin(kelvin, DAYLIGHT_MIN_KELVIN, DAYLIGHT_MAX_KELVIN)?;
    let t = kelvin;
    let (t2, t3) = (t * t, t * t * t);
    let x = if t <= 7000.0 {
        -4.607_0e9 / t3 + 2.967_8e6 / t2 + 0.099_11e3 / t + 0.244_063
    } else {
        -2.006_4e9 / t3 + 1.901_8e6 / t2 + 0.247_48e3 / t + 0.237_040
    };
    let y = -3.0 * x * x + 2.870 * x - 0.275;
    Ok(Chromaticity::new(x, y))
}

/// Converts a color temperature and a tint to a white point chromaticity.
///
/// - `kelvin`: point on the Planckian locus (see [`planckian_chromaticity`]
///   for the valid range, 1667 K to 25000 K).
/// - `tint`: offset perpendicular to the locus in CIE 1960 uv. The offset is
///   `Duv = -tint / TINT_SCALE`. Positive tint moves toward magenta (below
///   the locus). Negative tint moves toward green (above the locus). The
///   value must be finite.
///
/// The develop settings limit `tint` to -150..=150 (`Duv` -0.05..=0.05).
pub fn temperature_tint_to_chromaticity(kelvin: f64, tint: f64) -> CoreResult<Chromaticity> {
    if !tint.is_finite() {
        return Err(CoreError::NonFinite { name: "tint" });
    }
    let on_locus = planckian_chromaticity(kelvin)?;
    let (u, v) = on_locus.to_uv();

    // Tangent of the locus from a small finite difference. Step inward so
    // that both points stay in the valid range.
    let step = if kelvin + 1.0 <= PLANCKIAN_MAX_KELVIN {
        1.0
    } else {
        -1.0
    };
    let (u2, v2) = planckian_chromaticity(kelvin + step)?.to_uv();
    let (du, dv) = (u2 - u, v2 - v);
    let len = du.hypot(dv);
    if len < f64::MIN_POSITIVE {
        return Err(CoreError::InvalidChromaticity {
            x: on_locus.x,
            y: on_locus.y,
        });
    }
    // Unit normal. Choose the direction with positive v ("above" the locus,
    // toward green).
    let (mut nu, mut nv) = (-dv / len, du / len);
    if nv < 0.0 {
        nu = -nu;
        nv = -nv;
    }
    let duv = -tint / TINT_SCALE;
    Ok(Chromaticity::from_uv(u + duv * nu, v + duv * nv))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::matrix::assert_mat_close;

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "{a} vs {b}");
    }

    #[test]
    fn xyz_round_trip() {
        let c = Chromaticity::new(0.3, 0.4);
        let back = Chromaticity::from_xyz(c.to_xyz().unwrap()).unwrap();
        close(back.x, 0.3, 1e-12);
        close(back.y, 0.4, 1e-12);
        assert!(Chromaticity::new(0.3, 0.0).to_xyz().is_err());
        assert!(Chromaticity::from_xyz([0.0, 0.0, 0.0]).is_err());
    }

    #[test]
    fn srgb_matrix_matches_reference() {
        let p = Primaries {
            red: Chromaticity::new(0.64, 0.33),
            green: Chromaticity::new(0.30, 0.60),
            blue: Chromaticity::new(0.15, 0.06),
        };
        let m = p.rgb_to_xyz(white::D65).unwrap();
        let reference = Mat3::new([
            [0.4124, 0.3576, 0.1805],
            [0.2126, 0.7152, 0.0722],
            [0.0193, 0.1192, 0.9505],
        ]);
        assert_mat_close(&m, &reference, 1e-4);
    }

    #[test]
    fn bradford_d65_to_d50_matches_reference() {
        let m = bradford_adaptation(white::D65, white::D50).unwrap();
        // Published reference values for Bradford D65 -> D50.
        let reference = Mat3::new([
            [1.047_811_2, 0.022_886_6, -0.050_127_0],
            [0.029_542_4, 0.990_484_4, -0.017_049_1],
            [-0.009_234_5, 0.015_043_6, 0.752_131_6],
        ]);
        assert_mat_close(&m, &reference, 1e-3);
    }

    #[test]
    fn bradford_maps_white_to_white() {
        let m = bradford_adaptation(white::D50, white::D65).unwrap();
        let out = m.mul_vec(white::D50.to_xyz().unwrap());
        let expected = white::D65.to_xyz().unwrap();
        for i in 0..3 {
            close(out[i], expected[i], 1e-12);
        }
        let same = bradford_adaptation(white::D65, white::D65).unwrap();
        assert_mat_close(&same, &Mat3::IDENTITY, 1e-12);
    }

    #[test]
    fn planckian_matches_illuminant_a() {
        // CIE illuminant A: 2856 K, xy = (0.44757, 0.40745).
        let c = planckian_chromaticity(2856.0).unwrap();
        close(c.x, 0.447_57, 2e-3);
        close(c.y, 0.407_45, 2e-3);
    }

    #[test]
    fn daylight_6504_is_d65() {
        let c = daylight_chromaticity(6504.0).unwrap();
        close(c.x, 0.3127, 1e-3);
        close(c.y, 0.3290, 1e-3);
        let d50 = daylight_chromaticity(5003.0).unwrap();
        close(d50.x, 0.3457, 1e-3);
        close(d50.y, 0.3585, 1e-3);
    }

    #[test]
    fn kelvin_range_is_checked() {
        assert!(matches!(
            planckian_chromaticity(1000.0),
            Err(CoreError::OutOfRange { .. })
        ));
        assert!(matches!(
            daylight_chromaticity(3000.0),
            Err(CoreError::OutOfRange { .. })
        ));
        assert!(matches!(
            planckian_chromaticity(f64::NAN),
            Err(CoreError::NonFinite { .. })
        ));
        assert!(planckian_chromaticity(PLANCKIAN_MAX_KELVIN).is_ok());
        assert!(temperature_tint_to_chromaticity(PLANCKIAN_MAX_KELVIN, 10.0).is_ok());
    }

    #[test]
    fn zero_tint_is_on_locus() {
        let a = temperature_tint_to_chromaticity(5000.0, 0.0).unwrap();
        let b = planckian_chromaticity(5000.0).unwrap();
        close(a.x, b.x, 1e-12);
        close(a.y, b.y, 1e-12);
    }

    #[test]
    fn tint_moves_perpendicular_with_correct_sign() {
        let base = planckian_chromaticity(6500.0).unwrap().to_uv();
        let magenta = temperature_tint_to_chromaticity(6500.0, 30.0)
            .unwrap()
            .to_uv();
        let green = temperature_tint_to_chromaticity(6500.0, -30.0)
            .unwrap()
            .to_uv();
        // Magenta is below the locus (smaller v), green is above.
        assert!(magenta.1 < base.1);
        assert!(green.1 > base.1);
        // Distance in uv is |tint| / TINT_SCALE.
        let d = (magenta.0 - base.0).hypot(magenta.1 - base.1);
        close(d, 30.0 / TINT_SCALE, 1e-9);
        assert!(temperature_tint_to_chromaticity(6500.0, f64::INFINITY).is_err());
    }
}
