//! The export request from the UI, its validation, and the output size.
//!
//! The UI sends an [`ExportRequest`] as JSON (camelCase). [`validate`]
//! checks all values and returns an [`ExportPlan`], which the job runner
//! uses. A request that is not valid gives an
//! [`ErrorKind::InvalidInput`](crate::error::ErrorKind::InvalidInput)
//! error, and the job does not start.

use std::path::PathBuf;

use expose_core::buffer::{ResizeMode, fit_size};
use expose_core::color::ColorSpaceId;
use expose_raw::{BitDepth, ChromaSubsampling, EncodeOptions, OutputFormat, TiffCompression};
use serde::Deserialize;

use crate::error::CommandError;

/// The maximum number of photos in one request.
pub const MAX_PHOTOS: usize = 10_000;
/// The maximum length of a name suffix, in characters.
pub const MAX_SUFFIX_CHARS: usize = 64;
/// The maximum pixel length for the edge and side size modes.
pub const MAX_PIXELS_PER_SIDE: u32 = 65_535;
/// The maximum value of the megapixels size mode.
pub const MAX_MEGAPIXELS: f64 = 1000.0;
/// The minimum and maximum value of the percent size mode.
pub const PERCENT_RANGE: (f64, f64) = (1.0, 400.0);

/// An export request, as the UI sends it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportRequest {
    /// The photos to export, in order.
    pub photo_ids: Vec<i64>,
    /// The absolute destination folder.
    pub destination: String,
    /// The file format and its options.
    pub format: ExportFormat,
    /// The output color space.
    pub color_space: ExportColorSpace,
    /// The output size.
    pub size: ExportSize,
    /// If `true`, the output can be larger than the source.
    pub allow_upscale: bool,
    /// How the output files are named.
    pub naming: Naming,
    /// What to do when an output file exists.
    pub on_conflict: ConflictPolicy,
    /// If `true`, 8-bit output uses an ordered dither.
    pub dither: bool,
}

/// The output file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ExportFormat {
    /// Baseline JPEG, 8 bits.
    Jpeg {
        /// Quality, 1 to 100.
        quality: u32,
        /// Chroma subsampling: `"444"`, `"422"`, or `"420"`.
        subsampling: ChromaSubsampling,
    },
    /// PNG without alpha.
    Png {
        /// 8 or 16.
        bit_depth: u32,
    },
    /// TIFF without alpha.
    Tiff {
        /// 8 or 16.
        bit_depth: u32,
        /// `"none"`, `"lzw"`, or `"deflate"`.
        compression: TiffCompression,
    },
}

/// The output color spaces that export offers. The linear working space
/// is not offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum ExportColorSpace {
    /// sRGB.
    #[serde(rename = "srgb")]
    Srgb,
    /// Display P3.
    #[serde(rename = "display_p3")]
    DisplayP3,
    /// opRGB.
    #[serde(rename = "oprgb")]
    OpRgb,
    /// `ProPhoto` RGB.
    #[serde(rename = "prophoto_rgb")]
    ProPhotoRgb,
    /// Rec. 2020 (non-linear).
    #[serde(rename = "rec2020")]
    Rec2020,
}

impl From<ExportColorSpace> for ColorSpaceId {
    fn from(value: ExportColorSpace) -> Self {
        match value {
            ExportColorSpace::Srgb => Self::Srgb,
            ExportColorSpace::DisplayP3 => Self::DisplayP3,
            ExportColorSpace::OpRgb => Self::OpRgb,
            ExportColorSpace::ProPhotoRgb => Self::ProPhotoRgb,
            ExportColorSpace::Rec2020 => Self::Rec2020,
        }
    }
}

/// The output size.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum ExportSize {
    /// The size after develop (no resize).
    Original,
    /// The longer side gets `pixels`.
    LongEdge {
        /// 1 to 65535.
        pixels: u32,
    },
    /// The shorter side gets `pixels`.
    ShortEdge {
        /// 1 to 65535.
        pixels: u32,
    },
    /// The width gets `pixels`.
    Width {
        /// 1 to 65535.
        pixels: u32,
    },
    /// The height gets `pixels`.
    Height {
        /// 1 to 65535.
        pixels: u32,
    },
    /// The pixel count is at most this number of megapixels.
    Megapixels {
        /// More than 0, at most 1000.
        megapixels: f64,
    },
    /// Each side is this percent of the source side.
    Percent {
        /// 1 to 400. More than 100 needs `allowUpscale`.
        percent: f64,
    },
}

/// How the output files are named.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum Naming {
    /// The stem of the original file.
    Original,
    /// The stem of the original file, then `suffix`.
    Suffix {
        /// No path separators, no control characters, at most 64
        /// characters.
        suffix: String,
    },
}

/// What to do when an output file exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictPolicy {
    /// Add `-2`, `-3`, ... to the name until the name is free.
    UniqueName,
    /// Replace the existing file.
    Overwrite,
    /// Do not write the photo.
    Skip,
}

/// A validated export request.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportPlan {
    /// The photos, with duplicates removed (the first one stays).
    pub photo_ids: Vec<i64>,
    /// The absolute destination folder.
    pub destination: PathBuf,
    /// The encoder options.
    pub encode: EncodeOptions,
    /// The output size.
    pub size: ExportSize,
    /// If `true`, the output can be larger than the source.
    pub allow_upscale: bool,
    /// How the output files are named.
    pub naming: Naming,
    /// What to do when an output file exists.
    pub on_conflict: ConflictPolicy,
}

/// Parses a request from JSON.
///
/// The command takes the request as a JSON value and calls this function,
/// so that a value of the wrong type also gives an `InvalidInput` error.
///
/// # Errors
///
/// Returns `InvalidInput` if the JSON does not match [`ExportRequest`].
pub fn parse(value: serde_json::Value) -> Result<ExportRequest, CommandError> {
    serde_json::from_value(value).map_err(|error| {
        CommandError::invalid_input(format!("the export request is not valid: {error}"))
    })
}

/// Checks all values of `request` and returns the plan.
///
/// This function does not touch the file system. The caller checks that
/// the destination is a folder (or makes it).
///
/// # Errors
///
/// Returns `InvalidInput` for the first value that is not valid.
pub fn validate(request: ExportRequest) -> Result<ExportPlan, CommandError> {
    if request.photo_ids.is_empty() {
        return Err(invalid("the export request has no photos"));
    }
    if request.photo_ids.len() > MAX_PHOTOS {
        return Err(invalid(format!(
            "the export request has {} photos; the maximum is {MAX_PHOTOS}",
            request.photo_ids.len()
        )));
    }
    let mut seen = std::collections::HashSet::with_capacity(request.photo_ids.len());
    let photo_ids: Vec<i64> = request
        .photo_ids
        .into_iter()
        .filter(|id| seen.insert(*id))
        .collect();

    let destination = PathBuf::from(&request.destination);
    if request.destination.is_empty() || !destination.is_absolute() {
        return Err(invalid(format!(
            "the destination \"{}\" is not an absolute path",
            request.destination
        )));
    }

    let format = encode_format(request.format)?;
    validate_size(request.size, request.allow_upscale)?;
    if let Naming::Suffix { suffix } = &request.naming {
        validate_suffix(suffix)?;
    }

    Ok(ExportPlan {
        photo_ids,
        destination,
        encode: EncodeOptions {
            format,
            color_space: request.color_space.into(),
            alpha_mode: expose_core::buffer::AlphaMode::Straight,
            dither: request.dither,
        },
        size: request.size,
        allow_upscale: request.allow_upscale,
        naming: request.naming,
        on_conflict: request.on_conflict,
    })
}

fn bit_depth(value: u32) -> Result<BitDepth, CommandError> {
    match value {
        8 => Ok(BitDepth::Eight),
        16 => Ok(BitDepth::Sixteen),
        other => Err(CommandError::invalid_input(format!(
            "the bit depth {other} is not supported; use 8 or 16"
        ))),
    }
}

fn encode_format(format: ExportFormat) -> Result<OutputFormat, CommandError> {
    Ok(match format {
        ExportFormat::Jpeg {
            quality,
            subsampling,
        } => {
            let quality = u8::try_from(quality)
                .ok()
                .filter(|q| (1..=100).contains(q))
                .ok_or_else(|| {
                    CommandError::invalid_input(format!(
                        "the JPEG quality {quality} is not in the range 1 to 100"
                    ))
                })?;
            OutputFormat::Jpeg {
                quality,
                subsampling,
            }
        }
        ExportFormat::Png { bit_depth: depth } => OutputFormat::Png {
            bit_depth: bit_depth(depth)?,
            alpha: false,
        },
        ExportFormat::Tiff {
            bit_depth: depth,
            compression,
        } => OutputFormat::Tiff {
            bit_depth: bit_depth(depth)?,
            alpha: false,
            compression,
        },
    })
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::invalid_input(message)
}

fn validate_size(size: ExportSize, allow_upscale: bool) -> Result<(), CommandError> {
    match size {
        ExportSize::Original => Ok(()),
        ExportSize::LongEdge { pixels }
        | ExportSize::ShortEdge { pixels }
        | ExportSize::Width { pixels }
        | ExportSize::Height { pixels } => {
            if (1..=MAX_PIXELS_PER_SIDE).contains(&pixels) {
                Ok(())
            } else {
                Err(invalid(format!(
                    "the size {pixels} is not in the range 1 to {MAX_PIXELS_PER_SIDE} pixels"
                )))
            }
        }
        ExportSize::Megapixels { megapixels } => {
            if megapixels.is_finite() && megapixels > 0.0 && megapixels <= MAX_MEGAPIXELS {
                Ok(())
            } else {
                Err(invalid(format!(
                    "the size {megapixels} megapixels is not more than 0 and at most {MAX_MEGAPIXELS}"
                )))
            }
        }
        ExportSize::Percent { percent } => {
            let (min, max) = PERCENT_RANGE;
            if !(percent.is_finite() && (min..=max).contains(&percent)) {
                Err(invalid(format!(
                    "the size {percent} % is not in the range {min} to {max} %"
                )))
            } else if percent > 100.0 && !allow_upscale {
                Err(invalid(format!(
                    "the size {percent} % is larger than the source; turn on upscale"
                )))
            } else {
                Ok(())
            }
        }
    }
}

fn validate_suffix(suffix: &str) -> Result<(), CommandError> {
    let chars = suffix.chars().count();
    if chars > MAX_SUFFIX_CHARS {
        return Err(invalid(format!(
            "the name suffix has {chars} characters; the maximum is {MAX_SUFFIX_CHARS}"
        )));
    }
    if suffix.chars().any(|c| c == '/' || c == '\\') {
        return Err(invalid("the name suffix must not contain a path separator"));
    }
    if suffix.chars().any(char::is_control) {
        return Err(invalid(
            "the name suffix must not contain a control character",
        ));
    }
    Ok(())
}

/// The output size could not be computed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("cannot compute the output size: {0}")]
pub struct SizeError(String);

/// Returns the output size for a developed image of `width` x `height`
/// pixels.
///
/// The percent mode scales each side and rounds it to the nearest
/// integer (at least 1). The other modes use [`fit_size`]. If the result
/// is larger than the source and `allow_upscale` is `false`, the result is
/// the source size.
///
/// # Errors
///
/// Returns [`SizeError`] if a side is 0, a value is out of range, or a
/// side does not fit in `u32`.
pub fn output_size(
    width: u32,
    height: u32,
    size: ExportSize,
    allow_upscale: bool,
) -> Result<(u32, u32), SizeError> {
    let mode = match size {
        ExportSize::Original => ResizeMode::None,
        ExportSize::LongEdge { pixels } => ResizeMode::LongEdge(pixels),
        ExportSize::ShortEdge { pixels } => ResizeMode::ShortEdge(pixels),
        ExportSize::Width { pixels } => ResizeMode::Width(pixels),
        ExportSize::Height { pixels } => ResizeMode::Height(pixels),
        ExportSize::Megapixels { megapixels } => ResizeMode::Megapixels(megapixels),
        ExportSize::Percent { percent } => {
            return percent_size(width, height, percent, allow_upscale);
        }
    };
    fit_size(width, height, mode, allow_upscale).map_err(|error| SizeError(error.to_string()))
}

fn percent_size(
    width: u32,
    height: u32,
    percent: f64,
    allow_upscale: bool,
) -> Result<(u32, u32), SizeError> {
    if width == 0 || height == 0 {
        return Err(SizeError(format!(
            "the source size {width} x {height} is empty"
        )));
    }
    if !(percent.is_finite() && percent > 0.0) {
        return Err(SizeError(format!(
            "the percent value {percent} is not valid"
        )));
    }
    if percent > 100.0 && !allow_upscale {
        return Ok((width, height));
    }
    let scale = |side: u32| -> Result<u32, SizeError> {
        let scaled = (f64::from(side) * percent / 100.0).round().max(1.0);
        if scaled > f64::from(u32::MAX) {
            return Err(SizeError(format!(
                "the side {scaled} does not fit in 32 bits"
            )));
        }
        // `scaled` is finite, at least 1, and at most `u32::MAX` (see the
        // checks above), so the cast does not truncate.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(scaled as u32)
    };
    Ok((scale(width)?, scale(height)?))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::error::ErrorKind;

    fn request_json() -> serde_json::Value {
        json!({
            "photoIds": [3, 1, 3, 2, 1],
            "destination": "/tmp/out",
            "format": { "kind": "jpeg", "quality": 90, "subsampling": "444" },
            "colorSpace": "display_p3",
            "size": { "mode": "longEdge", "pixels": 2048 },
            "allowUpscale": false,
            "naming": { "mode": "suffix", "suffix": "_web" },
            "onConflict": "uniqueName",
            "dither": true
        })
    }

    fn with(mut base: serde_json::Value, key: &str, value: serde_json::Value) -> serde_json::Value {
        base[key] = value;
        base
    }

    fn check(value: serde_json::Value) -> Result<ExportPlan, CommandError> {
        validate(parse(value)?)
    }

    #[test]
    fn valid_request_gives_plan() {
        let plan = check(request_json()).unwrap();
        assert_eq!(plan.photo_ids, vec![3, 1, 2]);
        assert_eq!(plan.destination, PathBuf::from("/tmp/out"));
        assert_eq!(
            plan.encode.format,
            OutputFormat::Jpeg {
                quality: 90,
                subsampling: ChromaSubsampling::Yuv444
            }
        );
        assert_eq!(plan.encode.color_space, ColorSpaceId::DisplayP3);
        assert!(plan.encode.dither);
        assert_eq!(plan.size, ExportSize::LongEdge { pixels: 2048 });
        assert_eq!(plan.on_conflict, ConflictPolicy::UniqueName);
        assert_eq!(
            plan.naming,
            Naming::Suffix {
                suffix: "_web".into()
            }
        );
    }

    #[test]
    fn parses_all_formats_and_sizes() {
        let png = with(
            request_json(),
            "format",
            json!({ "kind": "png", "bitDepth": 16 }),
        );
        assert_eq!(
            check(png).unwrap().encode.format,
            OutputFormat::Png {
                bit_depth: BitDepth::Sixteen,
                alpha: false
            }
        );
        let tiff = with(
            request_json(),
            "format",
            json!({ "kind": "tiff", "bitDepth": 8, "compression": "lzw" }),
        );
        assert_eq!(
            check(tiff).unwrap().encode.format,
            OutputFormat::Tiff {
                bit_depth: BitDepth::Eight,
                alpha: false,
                compression: TiffCompression::Lzw
            }
        );
        let sizes = [
            (json!({ "mode": "original" }), ExportSize::Original),
            (
                json!({ "mode": "shortEdge", "pixels": 5 }),
                ExportSize::ShortEdge { pixels: 5 },
            ),
            (
                json!({ "mode": "width", "pixels": 65535 }),
                ExportSize::Width { pixels: 65535 },
            ),
            (
                json!({ "mode": "height", "pixels": 1 }),
                ExportSize::Height { pixels: 1 },
            ),
            (
                json!({ "mode": "megapixels", "megapixels": 12.5 }),
                ExportSize::Megapixels { megapixels: 12.5 },
            ),
            (
                json!({ "mode": "percent", "percent": 50 }),
                ExportSize::Percent { percent: 50.0 },
            ),
        ];
        for (json, expected) in sizes {
            let plan = check(with(request_json(), "size", json)).unwrap();
            assert_eq!(plan.size, expected);
        }
        for (name, space) in [
            ("srgb", ColorSpaceId::Srgb),
            ("oprgb", ColorSpaceId::OpRgb),
            ("prophoto_rgb", ColorSpaceId::ProPhotoRgb),
            ("rec2020", ColorSpaceId::Rec2020),
        ] {
            let plan = check(with(request_json(), "colorSpace", json!(name))).unwrap();
            assert_eq!(plan.encode.color_space, space);
        }
        let plain = with(request_json(), "naming", json!({ "mode": "original" }));
        assert_eq!(check(plain).unwrap().naming, Naming::Original);
        for policy in ["overwrite", "skip"] {
            assert!(check(with(request_json(), "onConflict", json!(policy))).is_ok());
        }
    }

    #[test]
    fn rejects_bad_values() {
        let too_many: Vec<i64> = (0..=10_000).collect();
        let cases = [
            ("photoIds", json!([])),
            ("photoIds", json!(too_many)),
            ("photoIds", json!("1")),
            ("destination", json!("relative/out")),
            ("destination", json!("")),
            (
                "format",
                json!({ "kind": "jpeg", "quality": 0, "subsampling": "420" }),
            ),
            (
                "format",
                json!({ "kind": "jpeg", "quality": 101, "subsampling": "420" }),
            ),
            (
                "format",
                json!({ "kind": "jpeg", "quality": 90, "subsampling": "411" }),
            ),
            (
                "format",
                json!({ "kind": "jpeg", "quality": 90, "subsampling": "420", "bitDepth": 16 }),
            ),
            ("format", json!({ "kind": "png", "bitDepth": 12 })),
            (
                "format",
                json!({ "kind": "tiff", "bitDepth": 16, "compression": "zip" }),
            ),
            ("format", json!({ "kind": "gif" })),
            ("colorSpace", json!("linear_rec2020")),
            ("size", json!({ "mode": "longEdge", "pixels": 0 })),
            ("size", json!({ "mode": "width", "pixels": 65536 })),
            ("size", json!({ "mode": "megapixels", "megapixels": 0 })),
            (
                "size",
                json!({ "mode": "megapixels", "megapixels": 1000.5 }),
            ),
            ("size", json!({ "mode": "percent", "percent": 0.5 })),
            ("size", json!({ "mode": "percent", "percent": 401 })),
            ("size", json!({ "mode": "percent", "percent": 150 })),
            ("naming", json!({ "mode": "suffix", "suffix": "a/b" })),
            ("naming", json!({ "mode": "suffix", "suffix": "a\\b" })),
            ("naming", json!({ "mode": "suffix", "suffix": "a\nb" })),
            (
                "naming",
                json!({ "mode": "suffix", "suffix": "x".repeat(65) }),
            ),
            ("onConflict", json!("rename")),
            ("dither", json!(null)),
        ];
        for (key, value) in cases {
            let text = format!("{key} = {value}");
            let error = check(with(request_json(), key, value)).unwrap_err();
            assert_eq!(
                error.kind,
                ErrorKind::InvalidInput,
                "{text}: {}",
                error.message
            );
        }
        let mut missing = request_json();
        missing.as_object_mut().unwrap().remove("format");
        assert_eq!(check(missing).unwrap_err().kind, ErrorKind::InvalidInput);
    }

    #[test]
    fn upscale_percent_needs_allow_upscale() {
        let mut value = with(
            request_json(),
            "size",
            json!({ "mode": "percent", "percent": 400 }),
        );
        value["allowUpscale"] = json!(true);
        assert!(check(value).is_ok());
        let suffix = with(
            request_json(),
            "naming",
            json!({ "mode": "suffix", "suffix": "é".repeat(64) }),
        );
        assert!(check(suffix).is_ok());
    }

    #[test]
    fn output_size_for_each_mode() {
        let size = |mode, up| output_size(6000, 4000, mode, up).unwrap();
        assert_eq!(size(ExportSize::Original, false), (6000, 4000));
        assert_eq!(
            size(ExportSize::LongEdge { pixels: 3000 }, false),
            (3000, 2000)
        );
        assert_eq!(
            size(ExportSize::ShortEdge { pixels: 1000 }, false),
            (1500, 1000)
        );
        assert_eq!(size(ExportSize::Width { pixels: 600 }, false), (600, 400));
        assert_eq!(size(ExportSize::Height { pixels: 400 }, false), (600, 400));
        assert_eq!(
            size(ExportSize::Megapixels { megapixels: 6.0 }, false),
            (3000, 2000)
        );
        assert_eq!(
            size(ExportSize::Percent { percent: 50.0 }, false),
            (3000, 2000)
        );
        assert_eq!(
            size(ExportSize::Percent { percent: 100.0 }, false),
            (6000, 4000)
        );
        // No upscale without permission.
        assert_eq!(
            size(ExportSize::LongEdge { pixels: 9000 }, false),
            (6000, 4000)
        );
        assert_eq!(
            size(ExportSize::Percent { percent: 200.0 }, false),
            (6000, 4000)
        );
        assert_eq!(
            size(ExportSize::LongEdge { pixels: 9000 }, true),
            (9000, 6000)
        );
        assert_eq!(
            size(ExportSize::Percent { percent: 200.0 }, true),
            (12000, 8000)
        );
    }

    #[test]
    fn percent_size_rounds_and_keeps_one_pixel() {
        assert_eq!(
            output_size(3, 1, ExportSize::Percent { percent: 50.0 }, false),
            Ok((2, 1))
        );
        assert_eq!(
            output_size(1, 1, ExportSize::Percent { percent: 1.0 }, false),
            Ok((1, 1))
        );
        assert_eq!(
            output_size(1001, 333, ExportSize::Percent { percent: 33.3 }, false),
            Ok((333, 111))
        );
        assert!(output_size(0, 1, ExportSize::Percent { percent: 50.0 }, false).is_err());
        assert!(output_size(0, 1, ExportSize::Original, false).is_err());
    }
}
