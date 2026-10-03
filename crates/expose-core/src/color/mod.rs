//! Color math: matrices, primaries, white points, transfer functions, and
//! color space definitions.
//!
//! The working space of Expose is linear Rec. 2020 with a D65 white point
//! (see ADR 0002). All matrices convert linear RGB. Use the transfer
//! function of a space to decode values before a matrix, and to encode them
//! after a matrix.

mod chromaticity;
mod matrix;
mod space;
mod transfer;

pub use chromaticity::{
    Chromaticity, DAYLIGHT_MAX_KELVIN, DAYLIGHT_MIN_KELVIN, PLANCKIAN_MAX_KELVIN,
    PLANCKIAN_MIN_KELVIN, Primaries, TINT_SCALE, bradford_adaptation, daylight_chromaticity,
    planckian_chromaticity, temperature_tint_to_chromaticity, white,
};
pub use matrix::{Mat3, mul_vec_f32};
pub use space::{
    ColorSpace, ColorSpaceId, DISPLAY_P3_PRIMARIES, OPRGB_PRIMARIES, PROPHOTO_PRIMARIES,
    REC2020_LUMINANCE, REC2020_PRIMARIES, SRGB_PRIMARIES, WORKING_WHITE, luminance,
};
pub use transfer::{OPRGB_GAMMA, TransferFunction};
