//! Image decode for Expose.
//!
//! # Responsibility
//!
//! This crate detects image formats, reads photo metadata (EXIF), decodes
//! standard formats (JPEG, PNG, TIFF 8/16/32-bit, WebP), and decodes camera
//! RAW files. All decoders give a [`WorkingImage`] (linear Rec. 2020 RGBA
//! `f32`, ADR 0002).
//!
//! # Main functions
//!
//! - [`detect_format`]: format from magic bytes.
//! - [`read_metadata`]: fast metadata read (no pixel decode).
//! - [`decode`] / [`decode_with`]: any supported file to a display-oriented
//!   [`WorkingImage`].
//! - [`raw::decode_raw`] and [`raw::demosaic()`]: RAW sensor data, and the
//!   separate develop step to the working image.
//!
//! All functions read from a byte slice. The file system helpers
//! ([`decode_path`], [`read_metadata_path`]) need the `native` feature (on
//! by default).
//!
//! # Untrusted input
//!
//! Decoders return errors for corrupt or truncated data; they do not panic.
//! [`Limits`] rejects images with too many pixels before the pixel memory is
//! allocated.
//!
//! # Licences
//!
//! - All dependencies are MIT, Apache-2.0, BSD, or zlib licensed, except
//!   `rawler`.
//! - `rawler` (cargo feature `rawler`, on by default) is **LGPL-2.1**. The
//!   operator approved it as an exception, for `rawler` only. A distributed
//!   application that includes it must let users relink the application
//!   against a modified `rawler` (for example, supply the object files or
//!   the application source). Do not add LGPL code to other crates. Build
//!   with `--no-default-features --features native` for a permissive-only
//!   build; RAW decode is then not available.
//!
//! # Dependency rules
//!
//! - This crate can depend on `expose-core` only (of the workspace crates).
//! - Use pure Rust decoders. Do not add C libraries.

#![allow(
    clippy::many_single_char_names,
    reason = "pixel and color math uses conventional short names (x, y, r, g, b)"
)]

mod error;
mod format;
mod icc;
mod limits;
mod metadata;
mod orientation;
pub mod raw;
mod standard;
mod tiff;

use expose_core::buffer::WorkingImage;
use serde::{Deserialize, Serialize};

pub use error::{DecodeError, DecodeResult};
pub use format::{ImageFormat, RawFormat, detect_format};
pub use limits::{DEFAULT_MAX_PIXELS, Limits};
#[cfg(feature = "native")]
pub use metadata::read_metadata_path;
pub use metadata::{
    CaptureTime, CaptureTimeSource, ExifStatus, ExposureTime, GpsPosition, LocalDateTime,
    PhotoMetadata, read_metadata,
};
pub use orientation::Orientation;
pub use standard::{ColorOrigin, SourceColor, SourceTransfer};

use raw::{DemosaicOptions, decode_raw, demosaic};
use standard::{ExifColorHint, decode_standard};

/// Options of [`decode_with`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecodeOptions {
    /// Size limits.
    pub limits: Limits,
    /// If `true`, the output is rotated and mirrored for display, as the
    /// orientation tag says.
    pub apply_orientation: bool,
    /// Options for RAW files.
    pub raw: DemosaicOptions,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            apply_orientation: true,
            raw: DemosaicOptions::default(),
        }
    }
}

/// Information about a RAW decode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawDecodeInfo {
    /// Name of the RAW backend.
    pub backend: String,
    /// Camera maker as the backend reports it.
    pub make: String,
    /// Camera model as the backend reports it.
    pub model: String,
    /// `true` if the file had no as-shot white balance, so D65 was used.
    pub white_balance_assumed: bool,
    /// The white balance multipliers (R, G, B) that were applied.
    pub multipliers: Option<[f32; 3]>,
}

/// Information about a decode.
#[allow(
    clippy::struct_excessive_bools,
    reason = "a plain information record; each flag is independent"
)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodeInfo {
    /// The detected format.
    pub format: ImageFormat,
    /// The color space of the source data. `None` for RAW files: their
    /// camera color is converted with the camera matrix (see `raw`).
    pub source_color: Option<SourceColor>,
    /// `true` if the source color space was assumed because the file has no
    /// usable profile or EXIF color space. Callers can show a warning.
    pub assumed_color_space: bool,
    /// `true` if the file has an embedded ICC profile.
    pub has_icc_profile: bool,
    /// The description of the embedded ICC profile, if readable.
    pub icc_description: Option<String>,
    /// Bits per sample of the source data.
    pub bits_per_sample: u32,
    /// `true` if the source has an alpha channel.
    pub has_alpha: bool,
    /// The orientation of the stored pixels (from the file).
    pub orientation: Orientation,
    /// `true` if the output was transformed for `orientation`.
    pub orientation_applied: bool,
    /// RAW decode details (RAW files only).
    pub raw: Option<RawDecodeInfo>,
}

/// A decoded image with its information and metadata.
#[derive(Debug, Clone)]
pub struct DecodedImage {
    /// The image in the working format (straight alpha).
    pub image: WorkingImage,
    /// Decode information.
    pub info: DecodeInfo,
    /// Photo metadata.
    pub metadata: PhotoMetadata,
}

/// Decodes an image file with default options.
pub fn decode(data: &[u8]) -> DecodeResult<DecodedImage> {
    decode_with(data, &DecodeOptions::default())
}

/// Decodes an image file (feature `native`).
#[cfg(feature = "native")]
pub fn decode_path(path: impl AsRef<std::path::Path>) -> DecodeResult<DecodedImage> {
    let data = std::fs::read(path)?;
    decode(&data)
}

/// Decodes an image file.
///
/// - Standard formats: decoded with the `image` crate and converted from the
///   source color space (see [`SourceColor`]) to linear Rec. 2020.
/// - RAW files: decoded with [`raw::decode_raw`] and developed with
///   [`raw::demosaic()`] and `options.raw`.
/// - HEIF and AVIF: [`DecodeError::UnsupportedFormat`].
pub fn decode_with(data: &[u8], options: &DecodeOptions) -> DecodeResult<DecodedImage> {
    let Some(format) = detect_format(data) else {
        // Rare RAW containers are not detected by magic bytes; a RAW backend
        // can still recognize them.
        return decode_raw_file(data, None, options);
    };
    if format.is_raw() {
        return decode_raw_file(data, Some(format), options);
    }
    let metadata = metadata::read_metadata_for(data, format, false)?;
    let hint = ExifColorHint {
        color_space: metadata.exif_color_space,
        interop_index: metadata.interop_index.as_deref(),
    };
    let (image, sinfo) = decode_standard(data, format, options.limits, hint)?;
    let orientation = metadata.orientation.unwrap_or_default();
    let image = if options.apply_orientation {
        orientation.apply(image)?
    } else {
        image
    };
    Ok(DecodedImage {
        image,
        info: DecodeInfo {
            format,
            source_color: Some(sinfo.color),
            assumed_color_space: sinfo.color.is_assumed(),
            has_icc_profile: sinfo.has_icc_profile,
            icc_description: sinfo.icc_description,
            bits_per_sample: sinfo.bits_per_sample,
            has_alpha: sinfo.has_alpha,
            orientation,
            orientation_applied: options.apply_orientation,
            raw: None,
        },
        metadata,
    })
}

fn decode_raw_file(
    data: &[u8],
    format: Option<ImageFormat>,
    options: &DecodeOptions,
) -> DecodeResult<DecodedImage> {
    let raw = decode_raw(data, &options.limits)?;
    let format = format.unwrap_or(ImageFormat::Raw(RawFormat::OtherTiff));
    let metadata = metadata::read_metadata_for(data, format, false)
        .unwrap_or_else(|e| metadata_error(format, &e));
    let developed = demosaic(&raw, &options.raw)?;
    // EXIF orientation first; the backend value if EXIF has none.
    let orientation = metadata.orientation.unwrap_or(raw.orientation);
    let image = if options.apply_orientation {
        orientation.apply(developed.image)?
    } else {
        developed.image
    };
    let mut metadata = metadata;
    metadata.width = Some(raw.crop.width);
    metadata.height = Some(raw.crop.height);
    Ok(DecodedImage {
        image,
        info: DecodeInfo {
            format,
            source_color: None,
            assumed_color_space: false,
            has_icc_profile: false,
            icc_description: None,
            bits_per_sample: raw.bits_per_sample,
            has_alpha: false,
            orientation,
            orientation_applied: options.apply_orientation,
            raw: Some(RawDecodeInfo {
                backend: raw.backend.to_string(),
                make: raw.make.clone(),
                model: raw.model.clone(),
                white_balance_assumed: developed.white_balance_assumed,
                multipliers: developed.solution.map(|s| s.multipliers),
            }),
        },
        metadata,
    })
}

/// Metadata for a RAW file whose metadata read failed; the pixels are still
/// valid, so the decode continues and the error is reported in the status.
fn metadata_error(format: ImageFormat, e: &DecodeError) -> PhotoMetadata {
    PhotoMetadata::empty(format, ExifStatus::Invalid(e.to_string()))
}
