//! Error type for `expose-raw`.

use expose_core::CoreError;
use thiserror::Error;

use crate::format::ImageFormat;

/// All errors that `expose-raw` can return.
///
/// Decoders return an error for corrupt or truncated input. They never
/// panic on bad input.
#[derive(Debug, Error)]
pub enum DecodeError {
    /// The magic bytes do not match any known image format.
    #[error("unknown image format")]
    UnknownFormat,

    /// The format is known, but this build cannot decode it.
    #[error("unsupported format {format:?}: {reason}")]
    UnsupportedFormat {
        /// The detected format.
        format: ImageFormat,
        /// Why the format is not supported.
        reason: &'static str,
    },

    /// The file uses a feature of a known format that this decoder does not
    /// support (for example, an unusual compression or sample layout).
    #[error("unsupported feature: {0}")]
    UnsupportedFeature(String),

    /// The data is corrupt or does not obey the format specification.
    #[error("invalid data: {0}")]
    InvalidData(String),

    /// The image is larger than the configured limit.
    #[error("image size {width}x{height} is more than the limit of {max_pixels} pixels")]
    TooLarge {
        /// Declared width in pixels.
        width: u64,
        /// Declared height in pixels.
        height: u64,
        /// The pixel limit that applied.
        max_pixels: u64,
    },

    /// An error from the standard format decoder.
    #[error("image decode error: {0}")]
    Image(#[from] image::ImageError),

    /// An error from an `expose-core` operation.
    #[error(transparent)]
    Core(#[from] CoreError),

    /// A file system error (only with the `native` feature).
    #[cfg(feature = "native")]
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl DecodeError {
    /// Makes an [`DecodeError::InvalidData`] error.
    pub(crate) fn invalid(msg: impl Into<String>) -> Self {
        Self::InvalidData(msg.into())
    }

    /// Makes an [`DecodeError::UnsupportedFeature`] error.
    pub(crate) fn unsupported(msg: impl Into<String>) -> Self {
        Self::UnsupportedFeature(msg.into())
    }
}

/// Result type with [`DecodeError`].
pub type DecodeResult<T> = Result<T, DecodeError>;

/// All errors that the encode functions can return.
///
/// This type is separate from [`DecodeError`]: encode errors come from bad
/// options or from the output side, not from bad input files.
#[derive(Debug, Error)]
pub enum EncodeError {
    /// An option has a value outside its permitted range.
    #[error("invalid encode option {name}: {reason}")]
    InvalidOption {
        /// Name of the option.
        name: &'static str,
        /// Why the value is not permitted.
        reason: String,
    },

    /// The combination of format, bit depth, and color space is not
    /// supported (for example, linear output with 8 bits).
    #[error("unsupported encode combination: {0}")]
    Unsupported(&'static str),

    /// The image is too large for the output format.
    #[error("image size {width}x{height} is more than the {format} limit of {max} pixels per side")]
    TooLarge {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// The output format.
        format: &'static str,
        /// Largest permitted side.
        max: u32,
    },

    /// An error from the PNG encoder.
    #[error("image encode error: {0}")]
    Image(#[from] image::ImageError),

    /// An error from the TIFF encoder.
    #[error("TIFF encode error: {0}")]
    Tiff(#[from] tiff::TiffError),

    /// An error from an `expose-core` operation.
    #[error(transparent)]
    Core(#[from] CoreError),

    /// A file system error (only with the `native` feature).
    #[cfg(feature = "native")]
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type with [`EncodeError`].
pub type EncodeResult<T> = Result<T, EncodeError>;
