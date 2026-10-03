//! White balance: chromatic adaptation from the scene white to D65.
//!
//! # Algorithm
//!
//! 1. Find the scene white chromaticity from the settings:
//!    - as shot: [`SourceInfo::as_shot`] (D65 for a rendered source),
//!    - preset: the preset temperature and tint,
//!    - custom: the temperature and tint sliders.
//!
//!    Temperature and tint go through
//!    [`temperature_tint_to_chromaticity`] (Planckian locus plus a
//!    perpendicular `Duv` offset).
//! 2. Make the Bradford adaptation matrix from the scene white to D65 in
//!    CIE XYZ, and express it in the working space:
//!    `M = XYZ_to_working * Bradford(scene -> D65) * working_to_XYZ`.
//!
//! The matrix maps a neutral surface under the scene light to neutral gray,
//! and keeps its luminance. Temperature and tint describe the *light*:
//! a higher temperature tells the pipeline that the light was bluer, so the
//! result becomes warmer.
//!
//! For a rendered source (white already at D65), a custom temperature of
//! about 6500 K with a tint of about -10 is near the identity (see
//! [`SourceInfo::as_shot_temperature_tint`]).
//!
//! RAW decoders can also apply white balance as camera RGB gains before the
//! camera matrix. This pipeline uses one adaptation in the working space for
//! all sources, so that the same code handles rendered and RAW sources.

use expose_core::CoreResult;
use expose_core::color::{
    Chromaticity, ColorSpaceId, Mat3, WORKING_WHITE, bradford_adaptation,
    temperature_tint_to_chromaticity,
};
use expose_core::develop::{WhiteBalance, WhiteBalanceMode};

use crate::error::{DevelopError, DevelopResult};
use crate::source::{SourceInfo, SourceWhite};

/// Returns the scene white for the settings, or `None` when the white
/// balance is the identity (as shot on a rendered source).
pub(crate) fn scene_white(
    settings: &WhiteBalance,
    source: &SourceInfo,
) -> DevelopResult<Option<Chromaticity>> {
    let from_temperature = |kelvin: f32, tint: f32| {
        temperature_tint_to_chromaticity(f64::from(kelvin), f64::from(tint))
            .map(Some)
            .map_err(DevelopError::Settings)
    };
    match settings.mode {
        WhiteBalanceMode::AsShot => match source.as_shot {
            SourceWhite::Rendered => Ok(None),
            other => other.chromaticity().map(Some).map_err(DevelopError::Source),
        },
        WhiteBalanceMode::Preset(preset) => {
            let (kelvin, tint) = preset.temperature_tint();
            from_temperature(kelvin, tint)
        }
        WhiteBalanceMode::Custom => from_temperature(settings.temperature, settings.tint),
    }
}

/// Returns the working-space matrix that adapts `scene_white` to D65.
///
/// See the module documentation for the formula. For `scene_white` = D65
/// the result is the identity within `1e-12`.
pub fn white_balance_matrix(scene_white: Chromaticity) -> CoreResult<Mat3> {
    let to_xyz = ColorSpaceId::LinearRec2020.definition().to_xyz()?;
    let adapt = bradford_adaptation(scene_white, WORKING_WHITE)?;
    Ok(to_xyz.inverse()? * adapt * to_xyz)
}

#[cfg(test)]
mod tests {
    use super::*;
    use expose_core::color::{luminance, mul_vec_f32, white};
    use expose_core::develop::WhiteBalancePreset;

    #[test]
    fn d65_gives_identity() {
        let m = white_balance_matrix(white::D65).unwrap();
        for r in 0..3 {
            for c in 0..3 {
                let expected = if r == c { 1.0 } else { 0.0 };
                assert!((m.rows[r][c] - expected).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn scene_neutral_becomes_gray_with_same_luminance() {
        let warm = temperature_tint_to_chromaticity(3000.0, 0.0).unwrap();
        let m = white_balance_matrix(warm).unwrap();
        // The working RGB of a neutral surface under the warm light.
        let to_xyz = ColorSpaceId::LinearRec2020.definition().to_xyz().unwrap();
        let neutral = to_xyz.inverse().unwrap().mul_vec(warm.to_xyz().unwrap());
        #[allow(clippy::cast_possible_truncation)]
        let neutral_f32 = neutral.map(|v| (v * 0.4) as f32);
        let out = mul_vec_f32(&m.to_f32(), neutral_f32);
        for v in out {
            assert!((v - 0.4).abs() < 1e-5, "{out:?}");
        }
        assert!((luminance(neutral_f32) - luminance(out)).abs() < 1e-5);
    }

    #[test]
    fn higher_temperature_makes_result_warmer() {
        let gray = [0.5_f32; 3];
        let apply = |kelvin| {
            let c = temperature_tint_to_chromaticity(kelvin, 0.0).unwrap();
            mul_vec_f32(&white_balance_matrix(c).unwrap().to_f32(), gray)
        };
        let cool = apply(4000.0);
        let warm = apply(9000.0);
        assert!(warm[0] / warm[2] > cool[0] / cool[2]);
    }

    #[test]
    fn scene_white_modes() {
        let mut wb = WhiteBalance::default();
        assert_eq!(scene_white(&wb, &SourceInfo::RENDERED).unwrap(), None);
        let raw = SourceInfo {
            as_shot: SourceWhite::Chromaticity(white::D50),
        };
        assert_eq!(scene_white(&wb, &raw).unwrap(), Some(white::D50));
        wb.mode = WhiteBalanceMode::Preset(WhiteBalancePreset::Tungsten);
        let c = scene_white(&wb, &raw).unwrap().unwrap();
        assert_eq!(c, temperature_tint_to_chromaticity(2850.0, 0.0).unwrap());
        wb.mode = WhiteBalanceMode::Custom;
        wb.temperature = 7000.0;
        wb.tint = 20.0;
        let c = scene_white(&wb, &SourceInfo::RENDERED).unwrap().unwrap();
        assert_eq!(c, temperature_tint_to_chromaticity(7000.0, 20.0).unwrap());
        let bad = SourceInfo {
            as_shot: SourceWhite::NeutralRgb([0.0; 3]),
        };
        wb.mode = WhiteBalanceMode::AsShot;
        assert!(matches!(
            scene_white(&wb, &bad),
            Err(DevelopError::Source(_))
        ));
    }
}
