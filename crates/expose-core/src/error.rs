//! Error type for `expose-core`.

use thiserror::Error;

/// All errors that `expose-core` can return.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum CoreError {
    /// The image width or height is zero.
    #[error("invalid image dimensions {width}x{height}: width and height must be at least 1")]
    InvalidDimensions {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
    },

    /// The number of samples for the image size does not fit in `usize`.
    #[error("image size {width}x{height} with {channels} channels overflows the address space")]
    SizeOverflow {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// Samples per pixel.
        channels: usize,
    },

    /// The data length does not agree with the image size.
    #[error("buffer length {actual} does not match the expected length {expected}")]
    BufferLengthMismatch {
        /// Number of samples that the image size needs.
        expected: usize,
        /// Number of samples that the caller supplied.
        actual: usize,
    },

    /// A rectangle is empty or is not fully inside the image.
    #[error("rectangle {x},{y} {width}x{height} is empty or outside the image")]
    InvalidRect {
        /// Left edge.
        x: u32,
        /// Top edge.
        y: u32,
        /// Width.
        width: u32,
        /// Height.
        height: u32,
    },

    /// A 3x3 matrix has no inverse.
    #[error("matrix is singular and has no inverse")]
    SingularMatrix,

    /// A chromaticity coordinate is not valid (for example, `y` is zero).
    #[error("invalid chromaticity x={x}, y={y}")]
    InvalidChromaticity {
        /// CIE 1931 x.
        x: f64,
        /// CIE 1931 y.
        y: f64,
    },

    /// A numeric value is outside its permitted range.
    #[error("{name} = {value} is outside the range {min}..={max}")]
    OutOfRange {
        /// Name of the value.
        name: &'static str,
        /// The value.
        value: f64,
        /// Lowest permitted value.
        min: f64,
        /// Highest permitted value.
        max: f64,
    },

    /// A numeric value is NaN or infinite.
    #[error("{name} is not a finite number")]
    NonFinite {
        /// Name of the value.
        name: &'static str,
    },

    /// A tone curve has a structural problem.
    #[error("invalid tone curve for channel {channel}: {reason}")]
    InvalidToneCurve {
        /// Curve channel name (`master`, `red`, `green`, or `blue`).
        channel: &'static str,
        /// Description of the problem.
        reason: &'static str,
    },

    /// The crop settings have a structural problem.
    #[error("invalid crop: {reason}")]
    InvalidCrop {
        /// Description of the problem.
        reason: &'static str,
    },

    /// The outlines of an image have more points than the permitted limit.
    #[error("the outline has more than {limit} points")]
    ContourTooLarge {
        /// The permitted number of points.
        limit: usize,
    },

    /// The settings come from a newer schema version than this build supports.
    #[error("settings version {found} is newer than the supported version {supported}")]
    UnsupportedVersion {
        /// Version in the settings.
        found: u32,
        /// Highest version that this build supports.
        supported: u32,
    },
}

/// Result type with [`CoreError`].
pub type CoreResult<T> = Result<T, CoreError>;
