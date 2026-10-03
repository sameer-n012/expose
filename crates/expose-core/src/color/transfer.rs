//! Transfer functions (encoding curves) of RGB color spaces.
//!
//! `decode` converts an encoded value to linear light. `encode` converts
//! linear light to an encoded value. The two are inverses.
//!
//! Negative inputs use an odd extension: `f(-x) = -f(x)`. This keeps
//! scene-referred values below zero stable through a round trip. Inputs
//! above 1.0 continue the curve formula.

/// The exponent of the opRGB transfer function (563/256, about 2.2).
pub const OPRGB_GAMMA: f32 = 563.0 / 256.0;

/// BT.2020 OETF constant alpha (exact value for continuity of the curve).
const REC2020_ALPHA: f32 = 1.099_296_8;
/// BT.2020 OETF constant beta (linear segment limit).
const REC2020_BETA: f32 = 0.018_053_97;

/// A transfer function.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransferFunction {
    /// No encoding. Values are linear light.
    Linear,
    /// The sRGB piecewise curve (IEC 61966-2-1). Display P3 uses it too.
    Srgb,
    /// A pure power curve: `encoded = linear^(1/gamma)`. opRGB uses
    /// [`OPRGB_GAMMA`].
    Gamma(f32),
    /// The `ProPhoto` RGB (ROMM RGB) curve: gamma 1.8 with a linear segment
    /// below 1/512.
    ProPhoto,
    /// The BT.2020 OETF (the same form as BT.709, with the precise 12-bit
    /// constants). This is the curve in common Rec. 2020 ICC profiles. It is
    /// not the BT.1886 display EOTF.
    Rec2020,
}

impl TransferFunction {
    /// Converts an encoded value to linear light.
    #[inline]
    pub fn decode(self, encoded: f32) -> f32 {
        let a = encoded.abs();
        let lin = match self {
            Self::Linear => a,
            Self::Srgb => {
                if a <= 0.040_45 {
                    a / 12.92
                } else {
                    ((a + 0.055) / 1.055).powf(2.4)
                }
            }
            Self::Gamma(g) => a.powf(g),
            Self::ProPhoto => {
                if a < 16.0 / 512.0 {
                    a / 16.0
                } else {
                    a.powf(1.8)
                }
            }
            Self::Rec2020 => {
                if a < 4.5 * REC2020_BETA {
                    a / 4.5
                } else {
                    ((a + REC2020_ALPHA - 1.0) / REC2020_ALPHA).powf(1.0 / 0.45)
                }
            }
        };
        lin.copysign(encoded)
    }

    /// Converts linear light to an encoded value.
    #[inline]
    pub fn encode(self, linear: f32) -> f32 {
        let a = linear.abs();
        let enc = match self {
            Self::Linear => a,
            Self::Srgb => {
                if a <= 0.003_130_8 {
                    a * 12.92
                } else {
                    1.055 * a.powf(1.0 / 2.4) - 0.055
                }
            }
            Self::Gamma(g) => a.powf(1.0 / g),
            Self::ProPhoto => {
                if a < 1.0 / 512.0 {
                    a * 16.0
                } else {
                    a.powf(1.0 / 1.8)
                }
            }
            Self::Rec2020 => {
                if a < REC2020_BETA {
                    a * 4.5
                } else {
                    REC2020_ALPHA * a.powf(0.45) - (REC2020_ALPHA - 1.0)
                }
            }
        };
        enc.copysign(linear)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [TransferFunction; 5] = [
        TransferFunction::Linear,
        TransferFunction::Srgb,
        TransferFunction::Gamma(OPRGB_GAMMA),
        TransferFunction::ProPhoto,
        TransferFunction::Rec2020,
    ];

    #[test]
    fn encode_decode_round_trip() {
        for tf in ALL {
            for i in 0..=1000 {
                #[allow(clippy::cast_precision_loss)]
                let x = (i as f32 / 1000.0) * 1.5 - 0.25;
                let rt = tf.decode(tf.encode(x));
                assert!((rt - x).abs() < 1e-5, "{tf:?}: {x} -> {rt}");
                let rt2 = tf.encode(tf.decode(x));
                assert!((rt2 - x).abs() < 1e-5, "{tf:?}: {x} -> {rt2}");
            }
        }
    }

    #[test]
    fn end_points_are_fixed() {
        for tf in ALL {
            assert!(tf.encode(0.0).abs() < 1e-7, "{tf:?}");
            assert!((tf.encode(1.0) - 1.0).abs() < 1e-5, "{tf:?}");
            assert!((tf.decode(1.0) - 1.0).abs() < 1e-5, "{tf:?}");
        }
    }

    #[test]
    fn known_values() {
        // sRGB mid gray: encoded 0.5 is about 0.214 linear.
        assert!((TransferFunction::Srgb.decode(0.5) - 0.214_041).abs() < 1e-5);
        // ProPhoto: linear segment edge.
        assert!((TransferFunction::ProPhoto.encode(1.0 / 512.0) - 16.0 / 512.0).abs() < 1e-6);
        // opRGB: 0.5 encoded is 0.5^2.19921875.
        let expected = 0.5_f32.powf(OPRGB_GAMMA);
        assert!((TransferFunction::Gamma(OPRGB_GAMMA).decode(0.5) - expected).abs() < 1e-7);
    }

    #[test]
    fn curves_are_continuous_at_segment_edges() {
        for (tf, edge) in [
            (TransferFunction::Srgb, 0.003_130_8_f32),
            (TransferFunction::ProPhoto, 1.0 / 512.0),
            (TransferFunction::Rec2020, REC2020_BETA),
        ] {
            let below = tf.encode(edge - 1e-6);
            let above = tf.encode(edge + 1e-6);
            assert!((above - below).abs() < 1e-4, "{tf:?}");
        }
    }

    #[test]
    fn negative_values_are_odd() {
        for tf in ALL {
            assert!((tf.encode(-0.3) + tf.encode(0.3)).abs() < 1e-7);
        }
    }
}
