//! RGB color space definitions.

use serde::{Deserialize, Serialize};

use crate::color::chromaticity::{Chromaticity, Primaries, bradford_adaptation, white};
use crate::color::matrix::Mat3;
use crate::color::transfer::{OPRGB_GAMMA, TransferFunction};
use crate::error::CoreResult;

/// The white point of the working space and of the XYZ connection space.
pub const WORKING_WHITE: Chromaticity = white::D65;

/// An RGB color space: primaries, white point, and transfer function.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorSpace {
    /// Chromaticities of the primaries.
    pub primaries: Primaries,
    /// Chromaticity of the white point.
    pub white: Chromaticity,
    /// Transfer function of encoded values.
    pub transfer: TransferFunction,
}

impl ColorSpace {
    /// Returns the matrix from linear RGB in this space to CIE XYZ under the
    /// space's own white point.
    pub fn to_xyz(&self) -> CoreResult<Mat3> {
        self.primaries.rgb_to_xyz(self.white)
    }

    /// Returns the matrix from linear RGB in this space to CIE XYZ under D65.
    ///
    /// If the white point of the space is not D65, the matrix includes a
    /// Bradford adaptation to D65.
    pub fn to_xyz_d65(&self) -> CoreResult<Mat3> {
        let m = self.to_xyz()?;
        if self.white == WORKING_WHITE {
            Ok(m)
        } else {
            Ok(bradford_adaptation(self.white, WORKING_WHITE)? * m)
        }
    }

    /// Returns the matrix from linear RGB in this space to linear RGB in
    /// `dst`. The matrix adapts white points with Bradford through D65.
    pub fn conversion_to(&self, dst: &ColorSpace) -> CoreResult<Mat3> {
        Ok(dst.to_xyz_d65()?.inverse()? * self.to_xyz_d65()?)
    }
}

/// Rec. 2020 (ITU-R BT.2020) primaries.
pub const REC2020_PRIMARIES: Primaries = Primaries {
    red: Chromaticity::new(0.708, 0.292),
    green: Chromaticity::new(0.170, 0.797),
    blue: Chromaticity::new(0.131, 0.046),
};

/// sRGB and BT.709 primaries.
pub const SRGB_PRIMARIES: Primaries = Primaries {
    red: Chromaticity::new(0.64, 0.33),
    green: Chromaticity::new(0.30, 0.60),
    blue: Chromaticity::new(0.15, 0.06),
};

/// Display P3 (DCI-P3 primaries with D65 white) primaries.
pub const DISPLAY_P3_PRIMARIES: Primaries = Primaries {
    red: Chromaticity::new(0.680, 0.320),
    green: Chromaticity::new(0.265, 0.690),
    blue: Chromaticity::new(0.150, 0.060),
};

/// opRGB (IEC 61966-2-5) primaries.
pub const OPRGB_PRIMARIES: Primaries = Primaries {
    red: Chromaticity::new(0.64, 0.33),
    green: Chromaticity::new(0.21, 0.71),
    blue: Chromaticity::new(0.15, 0.06),
};

/// `ProPhoto` RGB (ROMM RGB, ISO 22028-2) primaries.
pub const PROPHOTO_PRIMARIES: Primaries = Primaries {
    red: Chromaticity::new(0.7347, 0.2653),
    green: Chromaticity::new(0.1596, 0.8404),
    blue: Chromaticity::new(0.0366, 0.0001),
};

/// Luminance weights of linear Rec. 2020 RGB under D65 (the Y row of its
/// RGB to XYZ matrix).
pub const REC2020_LUMINANCE: [f32; 3] = [0.2627, 0.6780, 0.0593];

/// Returns the relative luminance (CIE Y) of a linear Rec. 2020 RGB value.
#[inline]
pub fn luminance(rgb: [f32; 3]) -> f32 {
    REC2020_LUMINANCE[0] * rgb[0] + REC2020_LUMINANCE[1] * rgb[1] + REC2020_LUMINANCE[2] * rgb[2]
}

/// Identifiers of the built-in color spaces.
///
/// The serialized names are stable: `linear_rec2020`, `srgb`, `display_p3`,
/// `oprgb`, `prophoto_rgb`, `rec2020`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColorSpaceId {
    /// Linear Rec. 2020, D65. This is the working space of Expose.
    #[serde(rename = "linear_rec2020")]
    LinearRec2020,
    /// sRGB (IEC 61966-2-1), D65, sRGB curve.
    #[serde(rename = "srgb")]
    Srgb,
    /// Display P3, D65, sRGB curve.
    #[serde(rename = "display_p3")]
    DisplayP3,
    /// opRGB (IEC 61966-2-5), D65, gamma 563/256.
    #[serde(rename = "oprgb")]
    OpRgb,
    /// `ProPhoto` RGB (ROMM RGB), D50, gamma 1.8 with a linear segment.
    #[serde(rename = "prophoto_rgb")]
    ProPhotoRgb,
    /// Rec. 2020, D65, BT.2020 OETF curve.
    #[serde(rename = "rec2020")]
    Rec2020,
}

impl ColorSpaceId {
    /// All built-in color spaces.
    pub const ALL: [Self; 6] = [
        Self::LinearRec2020,
        Self::Srgb,
        Self::DisplayP3,
        Self::OpRgb,
        Self::ProPhotoRgb,
        Self::Rec2020,
    ];

    /// Returns the definition of this color space.
    pub const fn definition(self) -> ColorSpace {
        match self {
            Self::LinearRec2020 => ColorSpace {
                primaries: REC2020_PRIMARIES,
                white: white::D65,
                transfer: TransferFunction::Linear,
            },
            Self::Srgb => ColorSpace {
                primaries: SRGB_PRIMARIES,
                white: white::D65,
                transfer: TransferFunction::Srgb,
            },
            Self::DisplayP3 => ColorSpace {
                primaries: DISPLAY_P3_PRIMARIES,
                white: white::D65,
                transfer: TransferFunction::Srgb,
            },
            Self::OpRgb => ColorSpace {
                primaries: OPRGB_PRIMARIES,
                white: white::D65,
                transfer: TransferFunction::Gamma(OPRGB_GAMMA),
            },
            Self::ProPhotoRgb => ColorSpace {
                primaries: PROPHOTO_PRIMARIES,
                white: white::D50,
                transfer: TransferFunction::ProPhoto,
            },
            Self::Rec2020 => ColorSpace {
                primaries: REC2020_PRIMARIES,
                white: white::D65,
                transfer: TransferFunction::Rec2020,
            },
        }
    }

    /// Returns the transfer function of this color space.
    pub const fn transfer(self) -> TransferFunction {
        self.definition().transfer
    }

    /// Returns the matrix from linear RGB in this space to the working space
    /// (linear Rec. 2020, D65).
    pub fn to_working(self) -> CoreResult<Mat3> {
        self.definition()
            .conversion_to(&Self::LinearRec2020.definition())
    }

    /// Returns the matrix from the working space (linear Rec. 2020, D65) to
    /// linear RGB in this space.
    pub fn from_working(self) -> CoreResult<Mat3> {
        Self::LinearRec2020
            .definition()
            .conversion_to(&self.definition())
    }
}

impl From<ColorSpaceId> for ColorSpace {
    fn from(id: ColorSpaceId) -> Self {
        id.definition()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::matrix::assert_mat_close;

    #[test]
    fn rec2020_matrix_matches_reference() {
        let m = ColorSpaceId::Rec2020.definition().to_xyz().unwrap();
        let reference = Mat3::new([
            [0.6370, 0.1446, 0.1689],
            [0.2627, 0.6780, 0.0593],
            [0.0000, 0.0281, 1.0610],
        ]);
        assert_mat_close(&m, &reference, 1e-4);
        // The luminance constant agrees with the derived Y row.
        for (i, w) in REC2020_LUMINANCE.iter().enumerate() {
            assert!((f64::from(*w) - m.rows[1][i]).abs() < 1e-4);
        }
    }

    #[test]
    fn display_p3_matrix_matches_reference() {
        let m = ColorSpaceId::DisplayP3.definition().to_xyz().unwrap();
        let reference = Mat3::new([
            [0.4866, 0.2657, 0.1982],
            [0.2290, 0.6917, 0.0793],
            [0.0000, 0.0451, 1.0439],
        ]);
        assert_mat_close(&m, &reference, 1e-4);
    }

    #[test]
    fn oprgb_matrix_matches_reference() {
        let m = ColorSpaceId::OpRgb.definition().to_xyz().unwrap();
        let reference = Mat3::new([
            [0.5767, 0.1856, 0.1882],
            [0.2973, 0.6274, 0.0753],
            [0.0270, 0.0707, 0.9911],
        ]);
        // The published values use a D65 white with more digits than
        // (0.3127, 0.3290). This changes the Z row by about 2e-4.
        assert_mat_close(&m, &reference, 5e-4);
    }

    #[test]
    fn prophoto_matrix_matches_reference() {
        let m = ColorSpaceId::ProPhotoRgb.definition().to_xyz().unwrap();
        let reference = Mat3::new([
            [0.7977, 0.1352, 0.0313],
            [0.2880, 0.7119, 0.0001],
            [0.0000, 0.0000, 0.8249],
        ]);
        assert_mat_close(&m, &reference, 1e-3);
    }

    #[test]
    fn srgb_to_rec2020_matches_reference() {
        // BT.2087 matrix from BT.709 to BT.2020.
        let m = ColorSpaceId::Srgb.to_working().unwrap();
        let reference = Mat3::new([
            [0.6274, 0.3293, 0.0433],
            [0.0691, 0.9195, 0.0114],
            [0.0164, 0.0880, 0.8956],
        ]);
        assert_mat_close(&m, &reference, 1e-3);
    }

    #[test]
    fn white_maps_to_white_in_all_spaces() {
        for id in ColorSpaceId::ALL {
            let to = id.to_working().unwrap();
            let from = id.from_working().unwrap();
            let w = to.mul_vec([1.0, 1.0, 1.0]);
            let back = from.mul_vec([1.0, 1.0, 1.0]);
            for i in 0..3 {
                assert!((w[i] - 1.0).abs() < 1e-9, "{id:?} to working: {w:?}");
                assert!(
                    (back[i] - 1.0).abs() < 1e-9,
                    "{id:?} from working: {back:?}"
                );
            }
        }
    }

    #[test]
    fn working_round_trip_is_identity() {
        for id in ColorSpaceId::ALL {
            let rt = id.from_working().unwrap() * id.to_working().unwrap();
            assert_mat_close(&rt, &Mat3::IDENTITY, 1e-9);
        }
        assert_mat_close(
            &ColorSpaceId::LinearRec2020.to_working().unwrap(),
            &Mat3::IDENTITY,
            1e-12,
        );
    }

    #[test]
    fn luminance_of_white_is_one() {
        assert!((luminance([1.0, 1.0, 1.0]) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn serde_names_are_stable() {
        let s = serde_json::to_string(&ColorSpaceId::OpRgb).unwrap();
        assert_eq!(s, "\"oprgb\"");
        let back: ColorSpaceId = serde_json::from_str("\"prophoto_rgb\"").unwrap();
        assert_eq!(back, ColorSpaceId::ProPhotoRgb);
    }
}
