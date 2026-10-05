//! The full develop on the GPU: geometry, tone stages, and sharpening.
//!
//! [`Pipeline::develop_gpu`] runs the passes in this order:
//!
//! 1. [`GEOMETRY_KERNEL`]: source to the crop size (only when the geometry
//!    applies and is not the identity at this size).
//! 2. [`DEVELOP_KERNEL`]: the tone stages (only when they are not the
//!    identity, or when no other pass runs, so that the result is always in
//!    a target image).
//! 3. Sharpening, four passes (only when the sharpener is active at the
//!    pixel scale): [`SHARPEN_ENCODE_KERNEL`], [`SHARPEN_BLUR_H_KERNEL`],
//!    [`SHARPEN_BLUR_V_KERNEL`], [`SHARPEN_COMBINE_KERNEL`].
//!
//! [`DevelopTargets`] keeps the intermediate images. It makes a new image
//! only when the necessary size changes, so repeated renders at one size
//! (slider changes) do not allocate.

use expose_core::buffer::WorkingImage;
use expose_gpu::{GpuContext, GpuImage, ImageFormat};

use crate::error::{DevelopError, DevelopResult};
use crate::gpu::DEVELOP_KERNEL;
use crate::gpu_stages::{
    GEOMETRY_KERNEL, GeometryParams, SHARPEN_BLUR_H_KERNEL, SHARPEN_BLUR_V_KERNEL,
    SHARPEN_COMBINE_KERNEL, SHARPEN_ENCODE_KERNEL, SharpenBlurParams, SharpenCombineParams,
    SharpenEncodeParams,
};
use crate::pipeline::{Pipeline, read_working_image};

/// The format of all target images.
const TARGET_FORMAT: ImageFormat = ImageFormat::Rgba32Float;

/// Intermediate and result images of [`Pipeline::develop_gpu`].
///
/// Keep one value for each view (for example, one preview) and give it to
/// each render. All images are `rgba32float`.
#[derive(Debug, Default)]
pub struct DevelopTargets {
    geometry: Option<GpuImage>,
    developed: Option<GpuImage>,
    encoded: Option<GpuImage>,
    blur_h: Option<GpuImage>,
    blurred: Option<GpuImage>,
    sharpened: Option<GpuImage>,
    allocations: u64,
}

impl DevelopTargets {
    /// Makes an empty set. The images are made on the first render.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of GPU images that this set made since it was made. A
    /// render at an unchanged size does not increase it.
    #[must_use]
    pub fn allocation_count(&self) -> u64 {
        self.allocations
    }

    /// Frees all images. The next render makes them again.
    pub fn clear(&mut self) {
        let allocations = self.allocations;
        *self = Self {
            allocations,
            ..Self::default()
        };
    }
}

/// Returns the image in `slot`, and first makes a new one when the slot is
/// empty or has a different size.
async fn ensure<'s>(
    ctx: &GpuContext,
    slot: &'s mut Option<GpuImage>,
    allocations: &mut u64,
    width: u32,
    height: u32,
) -> DevelopResult<&'s GpuImage> {
    let fits = slot
        .as_ref()
        .is_some_and(|image| image.width() == width && image.height() == height);
    if !fits {
        // Free the old image first, so that two large images do not exist
        // at the same time.
        *slot = None;
        *slot = Some(GpuImage::new(ctx, width, height, TARGET_FORMAT).await?);
        *allocations += 1;
    }
    let slot: &'s Option<GpuImage> = slot;
    slot.as_ref()
        .ok_or(DevelopError::Internal("a develop target was not made"))
}

impl Pipeline {
    /// Runs all stages on the GPU from `input` (see the module
    /// documentation). Returns the image that holds the result. The image
    /// belongs to `targets`, and the next render can change it.
    ///
    /// `pixel_scale` has the same meaning as in [`Pipeline::develop_cpu`].
    /// The result has the size [`Pipeline::output_size`] of the input size;
    /// use the size of the returned image for the next step (for example,
    /// the display encode). The function submits the work and does not wait
    /// for it.
    ///
    /// # Errors
    ///
    /// - [`DevelopError::Core`] when `pixel_scale` is not finite or not more
    ///   than 0.
    /// - [`DevelopError::Gpu`] when a GPU image or a kernel run fails.
    pub async fn develop_gpu<'t>(
        &self,
        ctx: &GpuContext,
        input: &GpuImage,
        pixel_scale: f32,
        targets: &'t mut DevelopTargets,
    ) -> DevelopResult<&'t GpuImage> {
        let sharpener = self.sharpener(pixel_scale)?;
        let geometry = self.active_geometry(input.width(), input.height())?;
        let DevelopTargets {
            geometry: geometry_slot,
            developed,
            encoded,
            blur_h,
            blurred,
            sharpened,
            allocations,
        } = targets;
        let mut result: Option<&'t GpuImage> = None;

        if let Some(g) = geometry {
            let (w, h) = g.output_size();
            let out = ensure(ctx, geometry_slot, allocations, w, h).await?;
            ctx.run_kernel_multi(&GEOMETRY_KERNEL, &GeometryParams::new(&g), &[input], out)
                .await?;
            result = Some(out);
        }

        if !self.tone_is_identity() || (result.is_none() && sharpener.is_none()) {
            let src = result.unwrap_or(input);
            let out = ensure(ctx, developed, allocations, src.width(), src.height()).await?;
            ctx.run_kernel(&DEVELOP_KERNEL, self.gpu_params(), src, out)
                .await?;
            result = Some(out);
        }

        if let Some(s) = sharpener {
            let src = result.unwrap_or(input);
            let (w, h) = (src.width(), src.height());
            let e = ensure(ctx, encoded, allocations, w, h).await?;
            let hb = ensure(ctx, blur_h, allocations, w, h).await?;
            let b = ensure(ctx, blurred, allocations, w, h).await?;
            let out = ensure(ctx, sharpened, allocations, w, h).await?;
            let blur = SharpenBlurParams::new(&s);
            ctx.run_kernel(
                &SHARPEN_ENCODE_KERNEL,
                &SharpenEncodeParams::default(),
                src,
                e,
            )
            .await?;
            ctx.run_kernel(&SHARPEN_BLUR_H_KERNEL, &blur, e, hb).await?;
            ctx.run_kernel(&SHARPEN_BLUR_V_KERNEL, &blur, hb, b).await?;
            ctx.run_kernel_multi(
                &SHARPEN_COMBINE_KERNEL,
                &SharpenCombineParams::new(&s),
                &[src, e, b],
                out,
            )
            .await?;
            result = Some(out);
        }

        result.ok_or(DevelopError::Internal("the GPU develop ran no pass"))
    }

    /// Uploads `image`, runs all stages on the GPU (`rgba32float`), and
    /// reads the result back. See [`Pipeline::develop_gpu`].
    ///
    /// # Errors
    ///
    /// See [`Pipeline::develop_gpu`].
    pub async fn develop_image_gpu(
        &self,
        ctx: &GpuContext,
        image: &WorkingImage,
        pixel_scale: f32,
    ) -> DevelopResult<WorkingImage> {
        let input = GpuImage::from_rgba_f32(
            ctx,
            image.width(),
            image.height(),
            TARGET_FORMAT,
            image.as_slice(),
        )
        .await?;
        let mut targets = DevelopTargets::new();
        let out = self
            .develop_gpu(ctx, &input, pixel_scale, &mut targets)
            .await?;
        read_working_image(ctx, out).await
    }
}
