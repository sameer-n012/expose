//! GPU processing for Expose.
//!
//! # Responsibility
//!
//! This crate contains the wgpu context, the shader pipelines, and (later)
//! the tiled render graph.
//!
//! # Main types
//!
//! - [`GpuContext`]: a headless device and queue, with a pipeline cache.
//! - [`GpuImage`]: a 2D RGBA texture ([`ImageFormat::Rgba32Float`],
//!   [`ImageFormat::Rgba16Float`], or [`ImageFormat::Rgba8Unorm`]) with
//!   upload from `&[f32]` and read-back to `Vec<f32>`. `Rgba8Unorm` images
//!   also have byte upload and read-back ([`GpuImage::write_rgba8`],
//!   [`GpuImage::read_rgba8`]), for display-encoded frames.
//! - [`Kernel`] and [`GpuContext::run_kernel`]: run a WGSL compute shader
//!   from one input image to one output image of the same size, with a
//!   parameter block.
//! - [`GpuContext::run_kernel_multi`]: run a kernel with 1 to
//!   [`MAX_KERNEL_INPUTS`] input images and an output of any size (for
//!   example a crop, a downsample, or a combine of two images).
//!
//! # Kernel contract
//!
//! Bind group 0 of each kernel (entry point `main`, workgroup 8 x 8 x 1):
//!
//! | Binding | WGSL type                                  | Content       |
//! | ------- | ------------------------------------------ | ------------- |
//! | 0       | `texture_2d<f32>`                          | input image 0 |
//! | 1       | `texture_storage_2d<OUTPUT_FORMAT, write>` | output image  |
//! | 2       | `var<uniform>` struct (optional)           | parameters    |
//! | 3, 4, 5 | `texture_2d<f32>`                          | inputs 1 to 3 |
//!
//! The dispatch covers the output image. [`GpuContext::run_kernel`] requires
//! one input of the output size. [`GpuContext::run_kernel_multi`] accepts any
//! sizes; the shader must keep its reads inside each input. Each call is one
//! queue submit, and the queue keeps the order, so a kernel can read the
//! output of the previous call with no extra synchronization.
//! - [`kernels`]: the built-in kernels.
//!
//! # Example
//!
//! ```no_run
//! use expose_gpu::{GpuContext, GpuContextOptions, GpuImage, ImageFormat, kernels};
//!
//! # async fn run(pixels: &[f32]) -> Result<Vec<f32>, expose_gpu::GpuError> {
//! let ctx = GpuContext::new(GpuContextOptions::default()).await?;
//! let input = GpuImage::from_rgba_f32(&ctx, 37, 23, ImageFormat::Rgba32Float, pixels).await?;
//! let output = GpuImage::new(&ctx, 37, 23, ImageFormat::Rgba32Float).await?;
//! let params = kernels::ExposureParams::from_ev(1.0);
//! ctx.run_kernel(&kernels::EXPOSURE, &params, &input, &output).await?;
//! output.read_rgba_f32(&ctx).await
//! # }
//! ```
//!
//! # Pixel data
//!
//! On the CPU side, pixels are interleaved RGBA `f32`, row-major, with no
//! row padding (see decision record 0002). Values below 0.0 and above 1.0
//! are valid (scene-referred). The kernels do not clamp.
//!
//! # Dependency rules
//!
//! - This crate can depend on `expose-core` only.
//! - This is the only crate that may use wgpu directly. Other crates use the
//!   API of this crate. The public API does not expose wgpu types.
//! - Shaders are WGSL, so that they also run on WebGPU in a browser.
//! - The API is `async` and starts no threads, so that it can run in a
//!   browser (WebAssembly + WebGPU).

mod context;
mod error;
mod image;
mod kernel;
mod oneshot;

pub use context::{AdapterSummary, GpuContext, GpuContextOptions, GpuLimits, PowerPreference};
pub use error::GpuError;
pub use image::{CHANNELS, GpuImage, ImageFormat};
pub use kernel::{
    Kernel, MAX_KERNEL_INPUTS, OUTPUT_FORMAT_TOKEN, WORKGROUP_SIZE, input_binding, kernels,
    workgroup_count,
};

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::CRATE_NAME;

    #[test]
    fn crate_name_is_correct() {
        assert_eq!(CRATE_NAME, "expose-gpu");
    }
}
