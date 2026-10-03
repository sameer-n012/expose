//! RAW backend that uses the `rawler` crate.
//!
//! LICENCE NOTE: `rawler` is LGPL-2.1. The operator approved it as an
//! exception to the permissive-licence rule (for `rawler` only). This module
//! only calls the `rawler` API; it contains no `rawler` code.
//!
//! # Mapping
//!
//! - `rawler` puts the CFA pattern origin and the black level pattern
//!   origin at the top-left of the full sensor data. [`RawImage`] uses the
//!   same convention.
//! - `rawler` gives white balance coefficients (multipliers). The as-shot
//!   neutral is their reciprocal.
//! - `rawler` gives color matrices (XYZ to camera) per illuminant. They are
//!   sorted by illuminant code, so that the result does not depend on hash
//!   map order.
//! - Orientation comes from the EXIF data that `rawler` reads. Some
//!   `rawler` decoders do not set `RawImage::orientation`.
//! - A crop area that is not fully inside the active area is reduced to the
//!   intersection. If there is no intersection, the active area is the crop.

use std::panic::{AssertUnwindSafe, catch_unwind};

use expose_core::buffer::Rect;
use expose_core::color::Mat3;
use rawler::RawImage as RlImage;
use rawler::decoders::RawDecodeParams;
use rawler::rawimage::{RawImageData, RawPhotometricInterpretation};
use rawler::rawsource::RawSource;

use super::{
    AsShotWhite, CfaColor, CfaPattern, ColorCalibration, LevelPattern, RawDecoder, RawImage,
    RawSamples, SensorLayout,
};
use crate::error::{DecodeError, DecodeResult};
use crate::format::{ImageFormat, RawFormat, detect_format};
use crate::limits::Limits;
use crate::orientation::Orientation;
use crate::tiff::TiffReader;

/// The `rawler` backend. Supports all RAW formats that `rawler` supports
/// (see the crate documentation for the list).
#[derive(Debug, Clone, Copy, Default)]
pub struct RawlerDecoder;

/// Name of this backend.
const NAME: &str = "rawler";

fn rl_err(e: &rawler::RawlerError) -> DecodeError {
    match e {
        rawler::RawlerError::Unsupported { .. } => DecodeError::unsupported(e.to_string()),
        rawler::RawlerError::DecoderFailed(_) => DecodeError::invalid(e.to_string()),
    }
}

fn to_u32(v: usize, what: &str) -> DecodeResult<u32> {
    u32::try_from(v).map_err(|_| DecodeError::invalid(format!("{what} {v} is too large")))
}

impl RawDecoder for RawlerDecoder {
    fn name(&self) -> &'static str {
        NAME
    }

    fn supports(&self, format: ImageFormat) -> bool {
        format.is_raw()
    }

    fn probe(&self, data: &[u8]) -> bool {
        let src = RawSource::new_from_slice(data);
        catch_unwind(AssertUnwindSafe(|| rawler::get_decoder(&src).is_ok())).unwrap_or(false)
    }

    fn decode(&self, data: &[u8], limits: &Limits) -> DecodeResult<RawImage> {
        // Size check before the pixel memory is allocated:
        // - DNG: the `rawler` DNG decoder decodes pixels also in its
        //   "dummy" mode, so read the declared sizes from the TIFF
        //   directories instead (cheap).
        // - Other formats: a "dummy" decode reads the size without pixel
        //   decode.
        let dng_size = if detect_format(data) == Some(ImageFormat::Raw(RawFormat::Dng)) {
            let size = TiffReader::new(data, &[])
                .and_then(|r| r.largest_image())
                .ok_or_else(|| DecodeError::invalid("DNG has no image size"))?;
            limits.check(size.0, size.1)?;
            Some(size)
        } else {
            None
        };
        let src = RawSource::new_from_slice(data);
        let params = RawDecodeParams::default();
        // `rawler` can panic on bad input. Catch the panic and return an
        // error, so that a corrupt file never stops the application.
        let result = catch_unwind(AssertUnwindSafe(
            || -> DecodeResult<(RlImage, Option<u16>)> {
                let decoder = rawler::get_decoder(&src).map_err(|e| rl_err(&e))?;
                if dng_size.is_none() {
                    let dummy = decoder
                        .raw_image(&src, &params, true)
                        .map_err(|e| rl_err(&e))?;
                    limits.check(dummy.width as u64, dummy.height as u64)?;
                }
                let image = decoder
                    .raw_image(&src, &params, false)
                    .map_err(|e| rl_err(&e))?;
                // Also check the final size (the declared size can differ).
                limits.check(image.width as u64, image.height as u64)?;
                // Metadata is optional: a file with bad EXIF can still give
                // pixels. The orientation then comes from the raw image.
                let orientation = decoder
                    .raw_metadata(&src, &params)
                    .ok()
                    .and_then(|m| m.exif.orientation);
                Ok((image, orientation))
            },
        ));
        let (image, exif_orientation) = match result {
            Ok(r) => r?,
            Err(_) => {
                return Err(DecodeError::invalid(
                    "the RAW decoder stopped with a panic (corrupt or unsupported file)",
                ));
            }
        };
        convert(&image, exif_orientation)
    }
}

/// Maps the sample arrangement.
fn layout_of(img: &RlImage) -> DecodeResult<SensorLayout> {
    let cpp = img.cpp;
    Ok(match &img.photometric {
        RawPhotometricInterpretation::Cfa(config) if cpp == 1 => {
            let cfa = &config.cfa;
            let mut colors = Vec::with_capacity(cfa.width * cfa.height);
            for row in 0..cfa.height {
                for col in 0..cfa.width {
                    colors.push(match cfa.color_at(row, col) {
                        0 => CfaColor::Red,
                        1 => CfaColor::Green,
                        2 => CfaColor::Blue,
                        other => {
                            return Err(DecodeError::unsupported(format!(
                                "CFA color {other} (only red, green, and blue are supported)"
                            )));
                        }
                    });
                }
            }
            SensorLayout::Cfa(CfaPattern::new(
                to_u32(cfa.width, "CFA width")?,
                to_u32(cfa.height, "CFA height")?,
                colors,
            )?)
        }
        RawPhotometricInterpretation::LinearRaw if cpp == 3 => SensorLayout::LinearRgb,
        RawPhotometricInterpretation::BlackIsZero if cpp == 1 => SensorLayout::Monochrome,
        other => {
            return Err(DecodeError::unsupported(format!(
                "sensor layout {other:?} with {cpp} samples per pixel"
            )));
        }
    })
}

/// Maps the black and white levels.
fn levels_of(img: &RlImage) -> DecodeResult<(LevelPattern, LevelPattern)> {
    let cpp = img.cpp;
    let bl = &img.blacklevel;
    let black = LevelPattern::new(
        to_u32(bl.width, "black level width")?,
        to_u32(bl.height, "black level height")?,
        to_u32(bl.cpp, "black level samples")?,
        bl.levels
            .iter()
            .map(rawler::formats::tiff::Rational::as_f32)
            .collect(),
    )?;

    #[allow(clippy::cast_precision_loss, reason = "white levels are < 2^24")]
    let wl: Vec<f32> = img.whitelevel.0.iter().map(|&v| v as f32).collect();
    let cpp32 = to_u32(cpp, "samples per pixel")?;
    let white = match wl.len() {
        1 => LevelPattern::uniform(wl[0], cpp32)?,
        n if n == cpp => LevelPattern::new(1, 1, cpp32, wl)?,
        // Four values for one-sample CFA data: one per 2x2 position.
        4 if cpp == 1 => LevelPattern::new(2, 2, 1, wl)?,
        n => {
            return Err(DecodeError::invalid(format!(
                "{n} white levels for {cpp} samples per pixel"
            )));
        }
    };
    Ok((black, white))
}

/// Converts a `rawler` image to a [`RawImage`].
fn convert(img: &RlImage, exif_orientation: Option<u16>) -> DecodeResult<RawImage> {
    let width = to_u32(img.width, "width")?;
    let height = to_u32(img.height, "height")?;
    let layout = layout_of(img)?;
    let (black, white) = levels_of(img)?;
    let samples = match &img.data {
        RawImageData::Integer(v) => RawSamples::U16(v.clone()),
        RawImageData::Float(v) => RawSamples::F32(v.clone()),
    };

    let wb = img.wb_coeffs;
    let as_shot = if wb[..3].iter().all(|v| v.is_finite() && *v > 0.0) {
        AsShotWhite::Neutral([
            1.0 / f64::from(wb[0]),
            1.0 / f64::from(wb[1]),
            1.0 / f64::from(wb[2]),
        ])
    } else {
        AsShotWhite::Unknown
    };

    let mut calibrations: Vec<ColorCalibration> = img
        .color_matrix
        .iter()
        .filter(|(_, m)| m.len() >= 9 && m.iter().all(|v| v.is_finite()))
        .map(|(illuminant, m)| ColorCalibration {
            illuminant: *illuminant as u16,
            xyz_to_camera: Mat3::new([
                [f64::from(m[0]), f64::from(m[1]), f64::from(m[2])],
                [f64::from(m[3]), f64::from(m[4]), f64::from(m[5])],
                [f64::from(m[6]), f64::from(m[7]), f64::from(m[8])],
            ]),
        })
        .collect();
    calibrations.sort_by_key(|c| c.illuminant);

    let full = Rect::from_size(width, height);
    let active_area = match img.active_area {
        Some(r) => Rect::new(
            to_u32(r.p.x, "active area x")?,
            to_u32(r.p.y, "active area y")?,
            to_u32(r.d.w, "active area width")?,
            to_u32(r.d.h, "active area height")?,
        ),
        None => full,
    };
    let crop = match img.crop_area {
        Some(r) => {
            let c = Rect::new(
                to_u32(r.p.x, "crop x")?,
                to_u32(r.p.y, "crop y")?,
                to_u32(r.d.w, "crop width")?,
                to_u32(r.d.h, "crop height")?,
            );
            c.intersection(&active_area).unwrap_or(active_area)
        }
        None => active_area,
    };

    let orientation = exif_orientation
        .and_then(Orientation::from_exif)
        .or_else(|| Orientation::from_exif(img.orientation.to_u16()))
        .unwrap_or_default();

    let raw = RawImage {
        width,
        height,
        layout,
        samples,
        black,
        white,
        bits_per_sample: to_u32(img.bps, "bits per sample")?,
        as_shot,
        calibrations,
        active_area,
        crop,
        orientation,
        make: img.clean_make.clone(),
        model: img.clean_model.clone(),
        backend: NAME,
    };
    raw.validate()?;
    Ok(raw)
}
