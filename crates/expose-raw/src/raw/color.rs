//! Camera color: white balance and the camera RGB to working space matrix.
//!
//! # Method
//!
//! The method follows the DNG specification, chapter "Mapping camera color
//! space to CIE XYZ space", with these simplifications:
//!
//! - Only the color matrices (XYZ to camera) are used. Forward matrices,
//!   camera calibration, and analog balance are not used.
//! - With two or more calibrations, the matrix is interpolated linearly in
//!   inverse correlated color temperature (CCT) between the calibration with
//!   the lowest and the highest CCT. The CCT of a white point comes from the
//!   `McCamy` approximation (error < 2 K from 2856 K to 6504 K, which is the
//!   normal calibration range).
//! - The white point of an as-shot neutral is found by iteration, because
//!   the matrix depends on the white point.
//! - Chromatic adaptation from the scene white to D65 uses Bradford. This is
//!   the same as the DNG path (Bradford to D50) followed by Bradford from D50
//!   to D65, because Bradford adaptations compose exactly.
//!
//! # Normalization
//!
//! White balance multipliers are `max(neutral) / neutral`, so the smallest
//! multiplier is 1. The final matrix is scaled so that white-balanced camera
//! RGB `(1, 1, 1)` maps to D65 white with luminance `Y = 1`. Thus a neutral
//! object with normalized raw value `v` (after black subtraction and white
//! level scaling, in the channel with the largest neutral value) gives
//! linear Rec. 2020 `(v, v, v)`.

use expose_core::color::{
    Chromaticity, ColorSpaceId, Mat3, WORKING_WHITE, bradford_adaptation, white,
};

use super::ColorCalibration;
use crate::error::{DecodeError, DecodeResult};

/// Lowest CCT that the interpolation uses, in kelvin.
const MIN_CCT: f64 = 1500.0;
/// Highest CCT that the interpolation uses, in kelvin.
const MAX_CCT: f64 = 50_000.0;
/// Maximum iterations to find the white point of a camera neutral.
const MAX_ITERATIONS: usize = 30;

/// Returns the correlated color temperature, in kelvin, of an EXIF light
/// source code, or `None` if the code has no defined temperature.
pub fn illuminant_kelvin(code: u16) -> Option<f64> {
    Some(match code {
        1 | 4 | 9 => 5500.0, // daylight, flash, fine weather
        2 | 14 => 4150.0,    // fluorescent, cool white fluorescent
        3 => 2850.0,         // tungsten
        10 => 6500.0,        // cloudy
        11 => 7500.0,        // shade
        12 => 6430.0,        // daylight fluorescent
        13 => 5000.0,        // day white fluorescent
        15 => 3450.0,        // white fluorescent
        17 => 2856.0,        // standard light A
        18 => 4874.0,        // standard light B
        19 => 6774.0,        // standard light C
        20 => 5503.0,        // D55
        21 => 6504.0,        // D65
        22 => 7504.0,        // D75
        23 => 5003.0,        // D50
        24 => 3200.0,        // ISO studio tungsten
        _ => return None,
    })
}

/// Returns the CCT of a chromaticity with the `McCamy` approximation,
/// limited to `MIN_CCT..=MAX_CCT`.
fn cct(xy: Chromaticity) -> f64 {
    let denom = 0.1858 - xy.y;
    if denom.abs() < 1e-9 {
        return MAX_CCT;
    }
    let n = (xy.x - 0.3320) / denom;
    let t = 449.0 * n.powi(3) + 3525.0 * n.powi(2) + 6823.3 * n + 5520.33;
    if t.is_finite() {
        t.clamp(MIN_CCT, MAX_CCT)
    } else {
        MAX_CCT
    }
}

/// Returns `a * (1 - w) + b * w` for each element.
fn lerp(a: &Mat3, b: &Mat3, w: f64) -> Mat3 {
    let mut rows = [[0.0; 3]; 3];
    for (r, row) in rows.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v = a.rows[r][c] * (1.0 - w) + b.rows[r][c] * w;
        }
    }
    Mat3::new(rows)
}

/// The interpolation inputs: the calibrations with the lowest and highest
/// known CCT.
struct Interpolator {
    low: (f64, Mat3),
    high: (f64, Mat3),
}

impl Interpolator {
    fn new(cals: &[ColorCalibration]) -> DecodeResult<Self> {
        let first = cals
            .first()
            .ok_or_else(|| DecodeError::unsupported("the file has no camera color matrix"))?;
        let mut known: Vec<(f64, Mat3)> = cals
            .iter()
            .filter_map(|c| illuminant_kelvin(c.illuminant).map(|k| (k, c.xyz_to_camera)))
            .collect();
        known.sort_by(|a, b| a.0.total_cmp(&b.0));
        match (known.first(), known.last()) {
            (Some(&low), Some(&high)) => Ok(Self { low, high }),
            // No calibration has a known illuminant: use the first matrix
            // for all white points (interpolation is not possible).
            _ => Ok(Self {
                low: (6504.0, first.xyz_to_camera),
                high: (6504.0, first.xyz_to_camera),
            }),
        }
    }

    /// Returns the XYZ to camera matrix for a white point.
    fn matrix(&self, xy: Chromaticity) -> Mat3 {
        let (t1, m1) = self.low;
        let (t2, m2) = self.high;
        if (t2 - t1).abs() < 1.0 {
            return m1;
        }
        let t = cct(xy);
        // Weight of the high-CCT matrix, linear in 1/T.
        let w = ((1.0 / t1 - 1.0 / t) / (1.0 / t1 - 1.0 / t2)).clamp(0.0, 1.0);
        lerp(&m1, &m2, w)
    }
}

fn check_neutral(n: [f64; 3]) -> DecodeResult<()> {
    if n.iter().all(|v| v.is_finite() && *v > 0.0) {
        Ok(())
    } else {
        Err(DecodeError::invalid(format!(
            "white balance neutral {n:?} is not positive"
        )))
    }
}

/// The result of the color calculation for one white balance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorSolution {
    /// Camera-native RGB of a neutral object (largest component 1).
    pub neutral: [f64; 3],
    /// The white point chromaticity of the scene.
    pub white: Chromaticity,
    /// Multipliers for camera R, G, B (smallest multiplier is 1).
    pub multipliers: [f32; 3],
    /// Matrix from white-balanced camera RGB to linear Rec. 2020 (D65).
    pub camera_to_working: Mat3,
}

impl ColorSolution {
    /// Solves for a camera neutral (for example the as-shot neutral).
    pub fn from_neutral(cals: &[ColorCalibration], neutral: [f64; 3]) -> DecodeResult<Self> {
        check_neutral(neutral)?;
        let interp = Interpolator::new(cals)?;
        // Start at D50 (as the DNG reference code does) and iterate.
        //
        // The interpolation weight is clamped and continuous, so the
        // iteration nearly always converges. If it does not converge in
        // MAX_ITERATIONS steps, the last value is still the white point of a
        // matrix in the calibrated range. The result is deterministic.
        let mut xy = white::D50;
        for _ in 0..MAX_ITERATIONS {
            let m = interp.matrix(xy).inverse()?;
            let next = Chromaticity::from_xyz(m.mul_vec(neutral))?;
            let delta = (next.x - xy.x).abs() + (next.y - xy.y).abs();
            xy = next;
            if delta < 1e-7 {
                break;
            }
        }
        Self::build(&interp, xy, neutral)
    }

    /// Solves for a scene white point chromaticity (for example from a
    /// color temperature and tint).
    pub fn from_white(cals: &[ColorCalibration], xy: Chromaticity) -> DecodeResult<Self> {
        let interp = Interpolator::new(cals)?;
        let neutral = interp.matrix(xy).mul_vec(xy.to_xyz()?);
        check_neutral(neutral)?;
        Self::build(&interp, xy, neutral)
    }

    fn build(interp: &Interpolator, xy: Chromaticity, neutral: [f64; 3]) -> DecodeResult<Self> {
        let max = neutral.iter().copied().fold(f64::MIN, f64::max);
        let n = neutral.map(|v| v / max);
        #[allow(
            clippy::cast_possible_truncation,
            reason = "multipliers are small positive numbers"
        )]
        let multipliers = n.map(|v| (1.0 / v) as f32);

        let cam_to_xyz = interp.matrix(xy).inverse()?;
        let adapt = bradford_adaptation(xy, WORKING_WHITE)?;
        let xyz_to_working = ColorSpaceId::LinearRec2020
            .definition()
            .to_xyz()?
            .inverse()?;
        // White-balanced (1,1,1) is camera `n`; undo the multipliers first.
        let m = adapt * cam_to_xyz * Mat3::diagonal(n);
        let y = m.mul_vec([1.0, 1.0, 1.0])[1];
        if !y.is_finite() || y <= 0.0 {
            return Err(DecodeError::invalid(
                "camera color matrix maps white to a non-positive luminance",
            ));
        }
        let camera_to_working = xyz_to_working * m * Mat3::diagonal([1.0 / y; 3]);
        Ok(Self {
            neutral: n,
            white: xy,
            multipliers,
            camera_to_working,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// XYZ to linear sRGB (a "camera" that sees linear sRGB).
    fn srgb_camera(illuminant: u16) -> ColorCalibration {
        let m = ColorSpaceId::Srgb
            .definition()
            .to_xyz()
            .unwrap()
            .inverse()
            .unwrap();
        ColorCalibration {
            illuminant,
            xyz_to_camera: m,
        }
    }

    fn close(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < tol)
    }

    #[test]
    fn cct_of_standard_whites() {
        assert!((cct(white::D65) - 6504.0).abs() < 10.0);
        assert!((cct(white::D50) - 5003.0).abs() < 10.0);
        assert!((cct(Chromaticity::new(0.447_57, 0.407_45)) - 2856.0).abs() < 10.0);
    }

    #[test]
    fn neutral_maps_to_working_white() {
        let cals = [srgb_camera(21)];
        let s = ColorSolution::from_neutral(&cals, [1.0, 1.0, 1.0]).unwrap();
        assert!((s.white.x - 0.3127).abs() < 1e-3 && (s.white.y - 0.3290).abs() < 1e-3);
        assert!(s.multipliers.iter().all(|m| (m - 1.0).abs() < 1e-6));
        let w = s.camera_to_working.mul_vec([1.0, 1.0, 1.0]);
        assert!(close(w, [1.0, 1.0, 1.0], 1e-9), "{w:?}");
        // The matrix is sRGB to Rec. 2020.
        let expected = ColorSpaceId::Srgb.to_working().unwrap();
        for r in 0..3 {
            assert!(close(s.camera_to_working.rows[r], expected.rows[r], 1e-6));
        }
    }

    #[test]
    fn tinted_neutral_gives_multipliers_and_white() {
        let cals = [srgb_camera(17), srgb_camera(21)];
        let s = ColorSolution::from_neutral(&cals, [0.5, 1.0, 0.8]).unwrap();
        let m = s.multipliers;
        assert!((m[0] - 2.0).abs() < 1e-6 && (m[1] - 1.0).abs() < 1e-6);
        assert!((m[2] - 1.25).abs() < 1e-6);
        let w = s.camera_to_working.mul_vec([1.0, 1.0, 1.0]);
        assert!(close(w, [1.0, 1.0, 1.0], 1e-9), "{w:?}");
    }

    #[test]
    fn interpolation_between_matrices() {
        let a = ColorCalibration {
            illuminant: 17,
            xyz_to_camera: Mat3::IDENTITY,
        };
        let b = ColorCalibration {
            illuminant: 21,
            xyz_to_camera: Mat3::diagonal([2.0, 2.0, 2.0]),
        };
        let i = Interpolator::new(&[b, a]).unwrap();
        let low = i.matrix(Chromaticity::new(0.447_57, 0.407_45));
        let high = i.matrix(white::D65);
        assert!((low.rows[0][0] - 1.0).abs() < 1e-2);
        assert!((high.rows[0][0] - 2.0).abs() < 1e-2);
        let mid = i.matrix(white::D50);
        assert!(mid.rows[0][0] > 1.2 && mid.rows[0][0] < 1.9);
    }

    #[test]
    fn errors_on_bad_input() {
        let cals = [srgb_camera(21)];
        assert!(ColorSolution::from_neutral(&cals, [0.0, 1.0, 1.0]).is_err());
        assert!(ColorSolution::from_neutral(&cals, [f64::NAN, 1.0, 1.0]).is_err());
        assert!(ColorSolution::from_neutral(&[], [1.0, 1.0, 1.0]).is_err());
        let singular = [ColorCalibration {
            illuminant: 21,
            xyz_to_camera: Mat3::new([[0.0; 3]; 3]),
        }];
        assert!(ColorSolution::from_neutral(&singular, [1.0, 1.0, 1.0]).is_err());
    }
}
