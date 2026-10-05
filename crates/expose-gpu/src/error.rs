//! Error type of the GPU crate, and the error-scope helper.

use crate::image::ImageFormat;

/// An error from a GPU operation.
#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    /// No GPU adapter matches the request. Tests use this variant to skip
    /// GPU work on machines without a GPU.
    #[error("no GPU adapter is available: {0}")]
    NoAdapter(String),

    /// The adapter does not supply a limit that this crate requires.
    #[error(
        "the GPU adapter does not support limit `{name}`: requires {required}, adapter supplies {supported}"
    )]
    UnsupportedLimit {
        /// The wgpu name of the limit.
        name: &'static str,
        /// The value that this crate requires.
        required: u64,
        /// The value that the adapter supplies.
        supported: u64,
    },

    /// The adapter cannot use a texture format in the way that this crate requires.
    #[error("the GPU adapter does not support {format:?} for {missing}")]
    UnsupportedFormat {
        /// The texture format.
        format: ImageFormat,
        /// The usages that are missing, as text.
        missing: String,
    },

    /// The device request failed.
    #[error("the GPU device request failed: {0}")]
    RequestDevice(String),

    /// The image dimensions are zero or larger than the device limit.
    #[error("invalid image size {width}x{height}: each side must be in 1..={max}")]
    InvalidDimensions {
        /// Requested width in pixels.
        width: u32,
        /// Requested height in pixels.
        height: u32,
        /// The maximum side length of the device.
        max: u32,
    },

    /// The pixel data does not have the length that the image size requires.
    #[error(
        "pixel data has {actual} samples, but the image requires {expected} (width x height x 4)"
    )]
    DataLength {
        /// The number of samples (`f32` or bytes) that the image requires.
        expected: usize,
        /// The number of samples that the caller supplied.
        actual: usize,
    },

    /// The input and output images of a kernel do not have the same size.
    #[error(
        "kernel input is {input_width}x{input_height}, but output is {output_width}x{output_height}"
    )]
    SizeMismatch {
        /// Input width in pixels.
        input_width: u32,
        /// Input height in pixels.
        input_height: u32,
        /// Output width in pixels.
        output_width: u32,
        /// Output height in pixels.
        output_height: u32,
    },

    /// The operation needs an image of a different pixel format (for
    /// example, `read_rgba8` on a float image).
    #[error("the operation requires a {expected:?} image, but the image is {actual:?}")]
    FormatMismatch {
        /// The format that the operation requires.
        expected: ImageFormat,
        /// The format of the image.
        actual: ImageFormat,
    },

    /// A kernel used the same image as input and output.
    #[error("a kernel cannot use the same image as input and output")]
    SameImage,

    /// A kernel run received too few or too many input images.
    #[error("a kernel needs 1 to {max} input images, but received {count}")]
    InputCount {
        /// The number of input images that the caller supplied.
        count: usize,
        /// The maximum number of input images
        /// ([`crate::MAX_KERNEL_INPUTS`]).
        max: usize,
    },

    /// The kernel source does not obey the kernel contract.
    #[error("kernel `{kernel}` is not valid: {reason}")]
    InvalidKernel {
        /// The kernel name.
        kernel: &'static str,
        /// The reason, as text.
        reason: String,
    },

    /// wgpu found a validation error (for example, a WGSL compile error).
    #[error("GPU validation error: {0}")]
    Validation(String),

    /// The GPU has no memory for the operation.
    #[error("GPU out of memory: {0}")]
    OutOfMemory(String),

    /// wgpu reported an internal error.
    #[error("GPU internal error: {0}")]
    Internal(String),

    /// A buffer map for read-back failed.
    #[error("GPU buffer map failed: {0}")]
    BufferMap(String),

    /// A wait for the device failed.
    #[error("GPU device poll failed: {0}")]
    Poll(String),
}

impl From<wgpu::Error> for GpuError {
    fn from(error: wgpu::Error) -> Self {
        match error {
            wgpu::Error::OutOfMemory { source } => Self::OutOfMemory(source.to_string()),
            wgpu::Error::Validation { description, .. } => Self::Validation(description),
            wgpu::Error::Internal { description, .. } => Self::Internal(description),
        }
    }
}

/// Runs `op` inside wgpu error scopes for validation and out-of-memory errors.
///
/// Without a scope, wgpu sends errors to the uncaptured-error handler, which
/// panics by default. With this helper, the errors become [`GpuError`] values.
///
/// The scope guards are popped before the first `.await`, so the returned
/// future does not hold a guard (the guards are not `Send`).
pub(crate) async fn scoped<T>(
    device: &wgpu::Device,
    op: impl FnOnce() -> T,
) -> Result<T, GpuError> {
    let out_of_memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let value = op();
    // Pop in reverse order of push, as wgpu requires.
    let validation = validation.pop();
    let out_of_memory = out_of_memory.pop();
    let validation_error = validation.await;
    let out_of_memory_error = out_of_memory.await;
    if let Some(error) = validation_error {
        return Err(error.into());
    }
    if let Some(error) = out_of_memory_error {
        return Err(error.into());
    }
    Ok(value)
}
