//! Develop pipeline for Expose.
//!
//! # Responsibility
//!
//! This crate contains the non-destructive develop pipeline. It changes
//! develop settings (edit parameters) into stages, and applies them to a
//! working image (linear Rec. 2020, D65, RGBA `f32`). The original image
//! never changes.
//!
//! # Use
//!
//! ```no_run
//! use expose_core::buffer::WorkingImage;
//! use expose_core::develop::DevelopSettings;
//! use expose_develop::{Pipeline, SourceInfo};
//!
//! # fn run(image: &WorkingImage, settings: &DevelopSettings) -> Result<(), expose_develop::DevelopError> {
//! let pipeline = Pipeline::new(settings, &SourceInfo::RENDERED)?;
//! // All stages. 1.0: `image` is the full-size source.
//! let developed = pipeline.develop_cpu(image, 1.0)?; // or `develop_image_gpu`
//! # Ok(())
//! # }
//! ```
//!
//! # Stage order
//!
//! The order is fixed:
//!
//! 0. **Geometry** ([`geometry`]): flip, straighten, and crop. It is first:
//!    it works on the linear source data, and it makes the image smaller
//!    before the other stages.
//! 1. **White balance** ([`white_balance`]): Bradford adaptation from the
//!    scene white to D65. It is first, because it corrects the light of the
//!    scene. All later stages expect neutral colors to be neutral.
//! 2. **Exposure**: multiply by `2^ev`. It is linear, as a change of the
//!    camera exposure is. It comes before the tone stages, so that their
//!    zones (defined relative to mid gray) see the corrected brightness.
//! 3. **Highlights, shadows, whites, blacks** ([`tone`]): a smooth,
//!    monotonic tone map of luminance in log space, by zone.
//! 4. **Contrast** ([`tone`]): an S-curve in log space around mid gray
//!    (0.18). It comes after the zones, so that it acts on the final
//!    distribution of tones.
//! 5. **Tone curve** ([`curve`]): monotone cubic point curves (master,
//!    then per channel) in a display-like (sRGB) encoding. It is the last
//!    creative step, because the user draws it against the result of the
//!    sliders.
//! 6. **Sharpening** ([`sharpen`]): luminance sharpening of the toned
//!    image. It is after the tone stages, so that it sees the final
//!    contrast. Its radius is in source pixels and scales with the pixel
//!    scale of the image.
//! 7. **Output** (not in this crate): conversion to the display or export
//!    color space, and the clip to `0..=1`.
//!
//! Stages that are the identity are skipped. Stages 1 and 2 are fused into
//! one matrix, and stages 3 and 4 into one LUT (see [`Pipeline`]).
//!
//! [`Pipeline::develop_cpu`] and [`Pipeline::develop_gpu`] run all stages.
//! [`Pipeline::apply_cpu`], [`Pipeline::render_cpu`], and
//! [`Pipeline::render_gpu`] run stages 1 to 5 only (the tone stages).
//!
//! # Scene-referred values
//!
//! No stage clips. Values below 0.0 and above 1.0 pass through all stages.
//! The tone curve continues as a straight line outside `0..=1` (see
//! [`curve`]).
//!
//! # CPU and GPU
//!
//! [`Pipeline::apply_pixel`] is the CPU reference of the tone stages. It is
//! deterministic. The GPU kernel ([`DEVELOP_KERNEL`], one fused pass) uses
//! the same tables and the same arithmetic. The GPU result agrees with the
//! CPU result within about `1e-5` relative (the GPU `log2`, `exp2`, and
//! `pow` functions are not bit-exact).
//!
//! The geometry kernel ([`GEOMETRY_KERNEL`]) and the four sharpening
//! kernels use the same math order as [`Geometry::apply_cpu`] and
//! [`Sharpener::apply_cpu`]. A crop or flip without rotation is bit-exact.
//! [`DevelopTargets`] keeps the intermediate GPU images between renders.
//!
//! # Not in this version
//!
//! Masks, HSL, color grading, noise reduction, and lens correction.
//!
//! # Dependency rules
//!
//! - This crate can depend on `expose-core` and `expose-gpu`.
//! - Do not use wgpu directly. Use the API of `expose-gpu`.
//! - No threads: the crate must also build for WebAssembly.

pub mod auto_tone;
#[allow(
    clippy::many_single_char_names,
    reason = "spline math uses the usual one-letter names"
)]
pub mod curve;
mod develop_gpu;
mod display;
mod error;
#[allow(
    clippy::many_single_char_names,
    reason = "geometry math uses the usual one-letter names"
)]
pub mod geometry;
mod gpu;
mod gpu_stages;
mod pipeline;
pub mod preview;
pub mod sharpen;
#[allow(
    clippy::many_single_char_names,
    reason = "colorimetry and Newton steps use the usual one-letter names"
)]
mod source;
#[allow(
    clippy::many_single_char_names,
    reason = "LUT math uses the usual one-letter names"
)]
pub mod tone;
pub mod white_balance;

pub use auto_tone::{LuminanceHistogram, auto_tone};
pub use develop_gpu::DevelopTargets;
pub use display::{DISPLAY_ENCODE_KERNEL, DisplayEncoder, DisplayParams, DisplaySpace};
pub use error::{DevelopError, DevelopResult};
pub use geometry::{Geometry, max_inscribed_rect, rect_is_inside};
pub use gpu::{DEVELOP_KERNEL, GpuParams};
pub use gpu_stages::{
    GEOMETRY_KERNEL, GeometryParams, SHARPEN_BLUR_H_KERNEL, SHARPEN_BLUR_V_KERNEL,
    SHARPEN_COMBINE_KERNEL, SHARPEN_ENCODE_KERNEL, SharpenBlurParams, SharpenCombineParams,
    SharpenEncodeParams,
};
pub use pipeline::{Pipeline, Stage};
pub use preview::{PreviewRenderer, downsample_area, preview_size, render_preview};
pub use sharpen::Sharpener;
pub use source::{SourceInfo, SourceWhite, chromaticity_to_temperature_tint};

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::CRATE_NAME;

    #[test]
    fn crate_name_is_correct() {
        assert_eq!(CRATE_NAME, "expose-develop");
    }
}
