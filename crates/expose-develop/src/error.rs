//! Error type for `expose-develop`.

use expose_core::CoreError;
use expose_gpu::GpuError;
use thiserror::Error;

/// All errors that `expose-develop` can return.
#[derive(Debug, Error)]
pub enum DevelopError {
    /// The develop settings are not valid (see `DevelopSettings::validate`).
    #[error("invalid develop settings: {0}")]
    Settings(#[source] CoreError),
    /// The source information is not valid (for example, the as-shot white
    /// point is not a valid chromaticity).
    #[error("invalid source information: {0}")]
    Source(#[source] CoreError),
    /// A color calculation or an image buffer operation failed.
    #[error(transparent)]
    Core(#[from] CoreError),
    /// A GPU operation failed.
    #[error(transparent)]
    Gpu(#[from] GpuError),
    /// The luminance histogram has no samples, so no statistics exist.
    #[error("the luminance histogram has no samples")]
    EmptyHistogram,
    /// The requested preview size is zero.
    #[error("the preview long edge must be at least 1 pixel")]
    InvalidPreviewSize,
    /// An internal rule of this crate is broken (a bug). The text names the
    /// rule.
    #[error("internal error: {0}")]
    Internal(&'static str),
    /// The GPU result does not have the length of the image.
    #[error("the GPU returned {actual} samples, but the image has {expected}")]
    ReadbackLength {
        /// Number of samples of the image.
        expected: usize,
        /// Number of samples that the GPU returned.
        actual: usize,
    },
}

/// A `Result` with [`DevelopError`].
pub type DevelopResult<T> = Result<T, DevelopError>;
