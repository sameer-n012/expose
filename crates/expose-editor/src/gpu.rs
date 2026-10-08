//! GPU compositor.
//!
//! [`GpuCompositor`] gives the same result as the CPU reference
//! [`composite`](crate::composite::composite), within float tolerance. The
//! model (coverage, pixel layers, adjustment layers, groups, dissolve) is
//! the same. See [`crate::composite`].
//!
//! # Design
//!
//! - The compositor uses the `expose-gpu` kernel API only. All buffers are
//!   `rgba32float` textures of the region size with **premultiplied** alpha.
//! - Each layer is **one kernel dispatch**. The kernel reads the current
//!   buffer (the backdrop), the layer data, and the mask, and writes a new
//!   buffer. A kernel output cannot be one of its inputs, so the
//!   compositor uses a pool of region-size buffers and takes a new one for
//!   each step. Two buffers are sufficient for a flat layer list. Each group
//!   level adds one or two buffers.
//! - The blend space encode and decode (sRGB transfer curve) occur per
//!   pixel in the kernel. Perceptual blending (the default) thus needs no
//!   extra passes.
//! - Each pixel layer is uploaded once for each call. Only the part that is
//!   inside the region is uploaded. A full layer is uploaded without a CPU
//!   copy. A later cache of layer textures (for example, keyed by
//!   [`LayerId`](crate::LayerId) and a content version) can replace the
//!   upload step: the kernel gets the canvas position and the size of the
//!   layer texture as parameters, so a full-layer texture also works.
//! - Masks are uploaded as `rgba32float` with **four mask values in each
//!   texel** (texel `x / 4`, channel `x % 4`). This uses the memory of a
//!   one-channel `f32` texture. Only the part inside the region is uploaded.
//! - Curves use a table of [`CURVE_TABLE_SIZE`] samples for each channel
//!   (the color curve, then the master curve), with linear interpolation.
//!   The other adjustments use the exact formulas.
//!
//! # Differences from the CPU reference
//!
//! - GPU float math (for example `pow`, division, fused multiply-add) can
//!   differ from the CPU in the last bits. The tests use a tolerance of
//!   `5e-5 * max(1, |value|)` for each sample (measured: at most about
//!   1.3e-5). The divide mode uses `5e-4`, because a small divisor makes
//!   the rounding error larger (measured: at most about 1.1e-4).
//! - Curves: the table interpolation adds an error of less than about
//!   `1e-6` where the curve is smooth. At a curve end point that is not on
//!   a table sample (`x` not a multiple of `1 / 4096`), the error is at
//!   most about `slope / 16384`.
//! - Steps (hard mix, threshold contrast, dissolve, color selection, and
//!   the branches of some blend modes) can give a different result when a
//!   value is within float error of the step.

use bytemuck::{Pod, Zeroable};
use expose_core::buffer::{Rect, WorkingImage};
use expose_gpu::{CHANNELS, GpuContext, GpuImage, ImageFormat, Kernel};

use crate::adjust::{Prepared, PreparedAdjustment};
use crate::blend::{BlendMode, BlendSpace};
use crate::document::Document;
use crate::error::{EditorError, EditorResult};
use crate::layer::{GroupIsolation, Layer, LayerKind, Mask, PixelLayer};

/// The number of samples in the curve table of a curves adjustment. The
/// sample `i` is at `x = i / 4096`, so the usual curve points (for example
/// 0, 0.25, 0.5) are on samples.
pub const CURVE_TABLE_SIZE: u32 = 4097;

/// The GPU format of all compositor buffers.
const FORMAT: ImageFormat = ImageFormat::Rgba32Float;

/// Blends a source image (pixel layer or isolated group) onto the backdrop.
const BLEND_KERNEL: Kernel = Kernel {
    name: "expose-editor/blend-layer",
    source: concat!(
        include_str!("shaders/common.wgsl"),
        include_str!("shaders/blend_layer.wgsl")
    ),
};

/// Applies an adjustment layer.
const ADJUST_KERNEL: Kernel = Kernel {
    name: "expose-editor/adjust",
    source: concat!(
        include_str!("shaders/common.wgsl"),
        include_str!("shaders/adjust.wgsl")
    ),
};

/// Mixes a pass-through group result with the state before the group.
const PASS_MIX_KERNEL: Kernel = Kernel {
    name: "expose-editor/pass-mix",
    source: concat!(
        include_str!("shaders/common.wgsl"),
        include_str!("shaders/pass_mix.wgsl")
    ),
};

/// Fills a buffer with transparent black.
const CLEAR_KERNEL: Kernel = Kernel {
    name: "expose-editor/clear",
    source: concat!(
        include_str!("shaders/common.wgsl"),
        include_str!("shaders/clear.wgsl")
    ),
};

/// Adjustment kinds in the kernel (`params.flags.w`).
const ADJUST_BRIGHTNESS_CONTRAST: u32 = 0;
const ADJUST_LEVELS: u32 = 1;
const ADJUST_CURVES: u32 = 2;
const ADJUST_HUE_SATURATION: u32 = 3;

/// The kernel parameter block. It agrees with `Params` in
/// `shaders/common.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Params {
    /// Canvas position and size of the region.
    region: [i32; 4],
    /// Canvas position and size of the auxiliary image (blend kernel).
    source: [i32; 4],
    /// Canvas position and size (in mask pixels) of the uploaded mask part.
    mask: [i32; 4],
    /// Blend mode ID, blend space (0 = linear, 1 = perceptual), mask flag,
    /// adjustment kind.
    flags: [u32; 4],
    /// Coverage base (opacity * fill), mask fill value, unused, unused.
    coef: [f32; 4],
    /// Adjustment data (see `shaders/adjust.wgsl`).
    adj: [[f32; 4]; 5],
}

/// Returns the kernel ID of a blend mode: its index in [`BlendMode::ALL`].
const fn mode_id(mode: BlendMode) -> u32 {
    match mode {
        BlendMode::Normal => 0,
        BlendMode::Dissolve => 1,
        BlendMode::Darken => 2,
        BlendMode::Multiply => 3,
        BlendMode::ColorBurn => 4,
        BlendMode::LinearBurn => 5,
        BlendMode::DarkerColor => 6,
        BlendMode::Lighten => 7,
        BlendMode::Screen => 8,
        BlendMode::ColorDodge => 9,
        BlendMode::LinearDodge => 10,
        BlendMode::LighterColor => 11,
        BlendMode::Overlay => 12,
        BlendMode::SoftLight => 13,
        BlendMode::HardLight => 14,
        BlendMode::VividLight => 15,
        BlendMode::LinearLight => 16,
        BlendMode::PinLight => 17,
        BlendMode::HardMix => 18,
        BlendMode::Difference => 19,
        BlendMode::Exclusion => 20,
        BlendMode::Subtract => 21,
        BlendMode::Divide => 22,
        BlendMode::Hue => 23,
        BlendMode::Saturation => 24,
        BlendMode::Color => 25,
        BlendMode::Luminosity => 26,
    }
}

const fn space_id(space: BlendSpace) -> u32 {
    match space {
        BlendSpace::Linear => 0,
        BlendSpace::Perceptual => 1,
    }
}

/// Converts a canvas coordinate or size to `i32` for the kernel.
fn to_i32(v: u32) -> EditorResult<i32> {
    // Canvas values are at most `MAX_DIMENSION`, so this does not fail for
    // a valid document.
    i32::try_from(v).map_err(|_| EditorError::OffsetOverflow)
}

fn rect_param(r: Rect) -> EditorResult<[i32; 4]> {
    Ok([
        to_i32(r.x)?,
        to_i32(r.y)?,
        to_i32(r.width)?,
        to_i32(r.height)?,
    ])
}

/// Returns [`EditorError::InvalidRect`] if `region` is empty or not fully
/// inside the canvas (the same check as the CPU compositor).
fn check_region(document: &Document, region: Rect) -> EditorResult<()> {
    if region.is_empty() || !document.bounds().contains_rect(&region) {
        return Err(EditorError::InvalidRect {
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
        });
    }
    Ok(())
}

/// The GPU compositor.
///
/// Make one compositor for each [`GpuContext`] and use it for all
/// composites. The compositor holds a small placeholder texture. The kernel
/// pipelines are compiled on the first use and stay in the context cache.
#[derive(Debug)]
pub struct GpuCompositor {
    /// A 1 x 1 texture for kernel inputs that a step does not use.
    placeholder: GpuImage,
}

impl GpuCompositor {
    /// Makes a compositor for `ctx`.
    pub async fn new(ctx: &GpuContext) -> EditorResult<Self> {
        Ok(Self {
            placeholder: GpuImage::new(ctx, 1, 1, FORMAT).await?,
        })
    }

    /// Composites the visible layers of `document` in `region` (canvas
    /// coordinates) on the GPU. Returns a premultiplied `rgba32float` image
    /// of the region size.
    ///
    /// The function submits the work and returns. A read-back of the
    /// result waits for the GPU.
    ///
    /// # Errors
    ///
    /// - [`EditorError::InvalidRect`] if `region` is empty or not fully
    ///   inside the canvas.
    /// - Errors of [`Adjustment::prepare`](crate::Adjustment::prepare).
    /// - [`EditorError::Gpu`] if a GPU operation fails (for example, the
    ///   region is larger than the device texture limit).
    pub async fn composite_gpu(
        &self,
        ctx: &GpuContext,
        document: &Document,
        region: Rect,
    ) -> EditorResult<GpuImage> {
        check_region(document, region)?;
        let mut run = Run {
            ctx,
            placeholder: &self.placeholder,
            region,
            region_param: rect_param(region)?,
            space: document.blend_space(),
            pool: Vec::new(),
        };
        let start = run.take_clear().await?;
        let result = run
            .composite_list(document.layers(), Buf::Owned(start))
            .await?;
        run.into_image(result).await
    }

    /// Composites like [`Self::composite_gpu`] and reads the result back
    /// to a premultiplied [`WorkingImage`].
    pub async fn composite_gpu_image(
        &self,
        ctx: &GpuContext,
        document: &Document,
        region: Rect,
    ) -> EditorResult<WorkingImage> {
        let image = self.composite_gpu(ctx, document, region).await?;
        let data = image.read_rgba_f32(ctx).await?;
        let pixels = data
            .chunks_exact(CHANNELS)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect();
        Ok(WorkingImage::from_pixels(
            region.width,
            region.height,
            pixels,
        )?)
    }
}

/// A compositor buffer: owned by the current step, or the backdrop of a
/// pass-through group (owned by the caller, read-only).
enum Buf<'a> {
    Owned(GpuImage),
    Borrowed(&'a GpuImage),
}

impl Buf<'_> {
    fn get(&self) -> &GpuImage {
        match self {
            Self::Owned(image) => image,
            Self::Borrowed(image) => image,
        }
    }
}

/// The state of one composite call.
struct Run<'c> {
    ctx: &'c GpuContext,
    placeholder: &'c GpuImage,
    region: Rect,
    region_param: [i32; 4],
    space: BlendSpace,
    /// Free region-size buffers.
    pool: Vec<GpuImage>,
}

impl Run<'_> {
    /// Takes a region-size buffer with any content.
    async fn take(&mut self) -> EditorResult<GpuImage> {
        match self.pool.pop() {
            Some(image) => Ok(image),
            None => {
                Ok(GpuImage::new(self.ctx, self.region.width, self.region.height, FORMAT).await?)
            }
        }
    }

    /// Takes a transparent region-size buffer.
    async fn take_clear(&mut self) -> EditorResult<GpuImage> {
        match self.pool.pop() {
            Some(image) => {
                let p = self.placeholder;
                let params = self.params();
                self.ctx
                    .run_kernel_multi(&CLEAR_KERNEL, &params, &[p, p, p], &image)
                    .await?;
                Ok(image)
            }
            // A new image is zero-filled.
            None => {
                Ok(GpuImage::new(self.ctx, self.region.width, self.region.height, FORMAT).await?)
            }
        }
    }

    /// Puts an owned buffer back in the pool.
    fn recycle(&mut self, buf: Buf<'_>) {
        if let Buf::Owned(image) = buf {
            self.pool.push(image);
        }
    }

    /// Returns `buf` as an owned image. A borrowed buffer is copied.
    async fn into_image(mut self, buf: Buf<'_>) -> EditorResult<GpuImage> {
        match buf {
            Buf::Owned(image) => Ok(image),
            Buf::Borrowed(image) => {
                let out = self.take().await?;
                self.ctx
                    .run_kernel(&expose_gpu::kernels::COPY, &(), image, &out)
                    .await?;
                Ok(out)
            }
        }
    }

    /// The parameters that are the same for all steps.
    fn params(&self) -> Params {
        let mut params = Params::zeroed();
        params.region = self.region_param;
        params.flags[1] = space_id(self.space);
        params
    }

    /// The parameters of a layer step without the mask.
    fn layer_params(&self, layer: &Layer, base: f32) -> Params {
        let mut params = self.params();
        params.flags[0] = mode_id(layer.blend_mode());
        params.coef[0] = base;
        params
    }

    /// Runs one step: `kernel` reads `cur`, `mask`, and `aux` and writes a
    /// new buffer, which becomes `cur`.
    async fn step(
        &mut self,
        cur: &mut Buf<'_>,
        kernel: &Kernel,
        params: &Params,
        mask: &GpuImage,
        aux: &GpuImage,
    ) -> EditorResult<()> {
        let out = self.take().await?;
        self.ctx
            .run_kernel_multi(kernel, params, &[cur.get(), mask, aux], &out)
            .await?;
        let old = std::mem::replace(cur, Buf::Owned(out));
        self.recycle(old);
        Ok(())
    }

    /// Composites `layers` onto `cur` and returns the new buffer.
    async fn composite_list<'a>(
        &mut self,
        layers: &[Layer],
        mut cur: Buf<'a>,
    ) -> EditorResult<Buf<'a>> {
        let placeholder = self.placeholder;
        for layer in layers {
            if !layer.visible() {
                continue;
            }
            let base = layer.opacity() * layer.fill_opacity();
            if base <= 0.0 {
                continue;
            }
            let mut params = self.layer_params(layer, base);
            match layer.kind() {
                LayerKind::Pixel(p) => {
                    let Some(source) = self.upload_layer(p, &mut params).await? else {
                        continue;
                    };
                    let mask = self.upload_mask(layer.mask(), &mut params).await?;
                    let mask = mask.as_ref().unwrap_or(placeholder);
                    self.step(&mut cur, &BLEND_KERNEL, &params, mask, &source)
                        .await?;
                }
                LayerKind::Adjustment(a) => {
                    let prepared = a.prepare()?;
                    let table = self.adjust_params(&prepared, &mut params).await?;
                    let mask = self.upload_mask(layer.mask(), &mut params).await?;
                    let mask = mask.as_ref().unwrap_or(placeholder);
                    let table = table.as_ref().unwrap_or(placeholder);
                    self.step(&mut cur, &ADJUST_KERNEL, &params, mask, table)
                        .await?;
                }
                LayerKind::Group(g) => {
                    let mask = self.upload_mask(layer.mask(), &mut params).await?;
                    let mask = mask.as_ref().unwrap_or(placeholder);
                    match g.isolation() {
                        GroupIsolation::PassThrough => {
                            let out = self.take().await?;
                            let after = Box::pin(
                                self.composite_list(g.children(), Buf::Borrowed(cur.get())),
                            )
                            .await?;
                            self.ctx
                                .run_kernel_multi(
                                    &PASS_MIX_KERNEL,
                                    &params,
                                    &[cur.get(), mask, after.get()],
                                    &out,
                                )
                                .await?;
                            self.recycle(after);
                            let old = std::mem::replace(&mut cur, Buf::Owned(out));
                            self.recycle(old);
                        }
                        GroupIsolation::Isolated => {
                            let start = self.take_clear().await?;
                            let group =
                                Box::pin(self.composite_list(g.children(), Buf::Owned(start)))
                                    .await?;
                            params.source = self.region_param;
                            self.step(&mut cur, &BLEND_KERNEL, &params, mask, group.get())
                                .await?;
                            self.recycle(group);
                        }
                    }
                }
            }
        }
        Ok(cur)
    }

    /// Uploads the part of a pixel layer that is inside the region and sets
    /// `params.source`. Returns `None` if the layer is outside the region.
    async fn upload_layer(
        &self,
        p: &PixelLayer,
        params: &mut Params,
    ) -> EditorResult<Option<GpuImage>> {
        let img = p.image();
        let (ox, oy) = p.offset();
        let Some(ov) = self
            .region
            .intersection_signed(ox, oy, img.width(), img.height())
        else {
            return Ok(None);
        };
        params.source = rect_param(ov)?;
        if ov.width == img.width() && ov.height == img.height() {
            // The full layer is inside the region: no CPU copy.
            let image =
                GpuImage::from_rgba_f32(self.ctx, ov.width, ov.height, FORMAT, img.as_slice())
                    .await?;
            return Ok(Some(image));
        }
        // The overlap is inside the layer, so these values are not
        // negative and fit in usize.
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let (x0, y0) = (
            (i64::from(ov.x) - i64::from(ox)) as usize,
            (i64::from(ov.y) - i64::from(oy)) as usize,
        );
        let (w, h) = (ov.width as usize, ov.height as usize);
        let src_row = img.width() as usize * CHANNELS;
        let all = img.as_slice();
        let mut data = Vec::with_capacity(w * h * CHANNELS);
        for row in y0..y0 + h {
            let start = row * src_row + x0 * CHANNELS;
            data.extend_from_slice(&all[start..start + w * CHANNELS]);
        }
        let image = GpuImage::from_rgba_f32(self.ctx, ov.width, ov.height, FORMAT, &data).await?;
        Ok(Some(image))
    }

    /// Sets the mask parameters and uploads the part of an enabled mask that
    /// is inside the region (four values in each texel). Returns `None` if
    /// there is nothing to upload.
    async fn upload_mask(
        &self,
        mask: Option<&Mask>,
        params: &mut Params,
    ) -> EditorResult<Option<GpuImage>> {
        let Some(m) = mask.filter(|m| m.enabled) else {
            // A disabled mask has the value 1 everywhere: the coverage is
            // the base value.
            params.flags[2] = 0;
            return Ok(None);
        };
        params.flags[2] = 1;
        params.coef[1] = m.fill.value();
        let img = &m.image;
        let Some(ov) =
            self.region
                .intersection_signed(m.offset.0, m.offset.1, img.width(), img.height())
        else {
            // The region is fully outside the mask buffer: the fill value
            // applies everywhere.
            params.mask = [0; 4];
            return Ok(None);
        };
        params.mask = rect_param(ov)?;
        // The overlap is inside the mask, so these values are not negative
        // and fit in usize.
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let (x0, y0) = (
            (i64::from(ov.x) - i64::from(m.offset.0)) as usize,
            (i64::from(ov.y) - i64::from(m.offset.1)) as usize,
        );
        let (w, h) = (ov.width as usize, ov.height as usize);
        let texel_width = ov.width.div_ceil(4);
        let row_len = texel_width as usize * CHANNELS;
        let src_row = img.width() as usize;
        let all = img.data();
        let mut data = vec![0.0_f32; row_len * h];
        for (row, dst) in data.chunks_exact_mut(row_len).enumerate() {
            let start = (y0 + row) * src_row + x0;
            dst[..w].copy_from_slice(&all[start..start + w]);
        }
        let image =
            GpuImage::from_rgba_f32(self.ctx, texel_width, ov.height, FORMAT, &data).await?;
        Ok(Some(image))
    }

    /// Sets the adjustment parameters. For curves, uploads and returns the
    /// curve table.
    async fn adjust_params(
        &self,
        prepared: &PreparedAdjustment,
        params: &mut Params,
    ) -> EditorResult<Option<GpuImage>> {
        match &prepared.inner {
            Prepared::BrightnessContrast(bc) => {
                params.flags[3] = ADJUST_BRIGHTNESS_CONTRAST;
                let (k, threshold) = bc.k.map_or((0.0, 1.0), |k| (k, 0.0));
                params.adj[0] = [bc.brightness, k, threshold, 0.0];
                Ok(None)
            }
            Prepared::Levels(l) => {
                params.flags[3] = ADJUST_LEVELS;
                let ch = [l.red, l.green, l.blue, l.master];
                params.adj[0] = ch.map(|c| c.input_black);
                params.adj[1] = ch.map(|c| c.input_white);
                params.adj[2] = ch.map(|c| c.gamma);
                params.adj[3] = ch.map(|c| c.output_black);
                params.adj[4] = ch.map(|c| c.output_white);
                Ok(None)
            }
            Prepared::Curves { master, channels } => {
                params.flags[3] = ADJUST_CURVES;
                let last = CURVE_TABLE_SIZE - 1;
                let mut data = Vec::with_capacity(CURVE_TABLE_SIZE as usize * CHANNELS);
                for i in 0..CURVE_TABLE_SIZE {
                    // `i` and `last` are at most 4096, so they are exact in
                    // f32, and the quotient is in 0..=1.
                    #[allow(clippy::cast_precision_loss)]
                    let x = i as f32 / last as f32;
                    for curve in channels.iter() {
                        let y = curve.eval(x);
                        data.push(master.as_ref().map_or(y, |m| m.eval(y)));
                    }
                    data.push(0.0);
                }
                let table =
                    GpuImage::from_rgba_f32(self.ctx, CURVE_TABLE_SIZE, 1, FORMAT, &data).await?;
                Ok(Some(table))
            }
            Prepared::HueSaturation {
                matrix,
                saturation,
                lightness,
            } => {
                params.flags[3] = ADJUST_HUE_SATURATION;
                for (dst, row) in params.adj.iter_mut().zip(matrix) {
                    *dst = [row[0], row[1], row[2], 0.0];
                }
                params.adj[3] = [*saturation, *lightness, 0.0, 0.0];
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // Test code: a failed setup is a test failure.
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_possible_wrap,
        clippy::too_many_lines
    )]

    use expose_core::develop::CurvePoint;
    use expose_gpu::{GpuContextOptions, GpuError};

    use super::*;
    use crate::adjust::{Adjustment, BrightnessContrast, Curves, HueSaturation, Levels};
    use crate::composite::composite;
    use crate::document::DocumentSpec;
    use crate::layer::{LayerContent, LayerId, MaskFill, MaskImage, Placement};

    /// The tolerance for each sample: `|gpu - cpu| <= TOL * max(1, |cpu|)`.
    ///
    /// The GPU and the CPU round some operations differently (`pow`,
    /// division, fused multiply-add). After several layers in the
    /// perceptual space (sRGB encode and decode at each step), the
    /// measured error is at most about 1.3e-5 (curves included).
    const TOL: f32 = 5e-5;

    /// The tolerance for the divide mode. A divisor near
    /// [`DIVIDE_MIN_SOURCE`](crate::blend::DIVIDE_MIN_SOURCE) makes the
    /// relative rounding error of the divisor much larger. The measured
    /// error is at most about 1.1e-4.
    const DIVIDE_TOL: f32 = 5e-4;

    fn context() -> Option<GpuContext> {
        match pollster::block_on(GpuContext::new(GpuContextOptions::default())) {
            Ok(ctx) => Some(ctx),
            Err(GpuError::NoAdapter(reason)) => {
                eprintln!("SKIPPED: no GPU adapter ({reason})");
                None
            }
            Err(error) => panic!("GPU context creation failed: {error}"),
        }
    }

    /// A small deterministic random generator (xorshift64*).
    struct Rng(u64);

    impl Rng {
        fn next_u32(&mut self) -> u32 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
        }

        /// A value in `lo..hi`.
        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            let u = (self.next_u32() >> 8) as f32 / 16_777_216.0;
            lo + (hi - lo) * u
        }
    }

    /// Random premultiplied pixels. Colors are in -0.2..2.0 (values below
    /// 0 and above 1 are included). 10 % of the pixels are transparent and
    /// 20 % are opaque.
    fn random_image(rng: &mut Rng, width: u32, height: u32) -> WorkingImage {
        let pixels = (0..width as usize * height as usize)
            .map(|_| {
                let pick = rng.range(0.0, 1.0);
                let a = if pick < 0.1 {
                    0.0
                } else if pick < 0.3 {
                    1.0
                } else {
                    rng.range(0.0, 1.0)
                };
                let c = [
                    rng.range(-0.2, 2.0),
                    rng.range(-0.2, 2.0),
                    rng.range(-0.2, 2.0),
                ];
                [c[0] * a, c[1] * a, c[2] * a, a]
            })
            .collect();
        WorkingImage::from_pixels(width, height, pixels).unwrap()
    }

    fn random_mask(rng: &mut Rng, width: u32, height: u32) -> MaskImage {
        // Values slightly outside 0..=1 test the clamp.
        let data = (0..width as usize * height as usize)
            .map(|_| rng.range(-0.2, 1.2))
            .collect();
        MaskImage::from_data(width, height, data).unwrap()
    }

    fn empty_doc(width: u32, height: u32, space: BlendSpace) -> Document {
        Document::new_empty(DocumentSpec {
            blend_space: space,
            ..DocumentSpec::new(width, height)
        })
        .unwrap()
    }

    /// Adds a layer at the top of `parent` and returns its ID.
    fn add(doc: &mut Document, parent: Option<LayerId>, content: LayerContent) -> LayerId {
        let index = match parent {
            Some(id) => doc.layer(id).unwrap().children().len(),
            None => doc.layers().len(),
        };
        doc.add_layer("layer", content, Placement::new(parent, index))
            .unwrap()
            .0
    }

    fn add_pixel(
        doc: &mut Document,
        rng: &mut Rng,
        parent: Option<LayerId>,
        size: (u32, u32),
        offset: (i32, i32),
    ) -> LayerId {
        let image = random_image(rng, size.0, size.1);
        add(
            doc,
            parent,
            LayerContent::Pixel(PixelLayer::new(image, offset).unwrap()),
        )
    }

    fn set_mask(
        doc: &mut Document,
        rng: &mut Rng,
        id: LayerId,
        size: (u32, u32),
        offset: (i32, i32),
        fill: MaskFill,
    ) {
        let mask = Mask::new(random_mask(rng, size.0, size.1), offset, fill);
        doc.set_mask(id, Some(mask)).unwrap();
    }

    fn curves() -> Curves {
        Curves {
            master: vec![
                CurvePoint::new(0.0, 0.0),
                CurvePoint::new(0.25, 0.3),
                CurvePoint::new(0.75, 0.8),
                CurvePoint::new(1.0, 1.0),
            ],
            red: vec![
                CurvePoint::new(0.0, 0.1),
                CurvePoint::new(0.5, 0.6),
                CurvePoint::new(1.0, 0.9),
            ],
            green: vec![CurvePoint::new(0.0, 0.0), CurvePoint::new(1.0, 1.0)],
            blue: vec![CurvePoint::new(0.25, 0.0), CurvePoint::new(1.0, 1.0)],
        }
    }

    fn levels() -> Levels {
        let mut l = Levels::default();
        l.master.gamma = 1.4;
        l.master.output_black = 0.05;
        l.red.input_black = 0.1;
        l.red.input_white = 0.9;
        l.green.gamma = 0.6;
        l.blue.output_white = 0.2;
        l.blue.output_black = 0.8;
        l
    }

    fn all_adjustments() -> Vec<Adjustment> {
        vec![
            Adjustment::BrightnessContrast(BrightnessContrast {
                brightness: 0.2,
                contrast: 0.3,
            }),
            Adjustment::BrightnessContrast(BrightnessContrast {
                brightness: -0.3,
                contrast: 1.0,
            }),
            Adjustment::Levels(levels()),
            Adjustment::Curves(curves()),
            Adjustment::HueSaturation(HueSaturation {
                hue: 40.0,
                saturation: 0.3,
                lightness: -0.2,
            }),
            Adjustment::HueSaturation(HueSaturation {
                hue: -120.0,
                saturation: -0.5,
                lightness: 0.4,
            }),
        ]
    }

    /// Blend modes with a step in the formula, where a float error in an
    /// input can give a large change of the result:
    ///
    /// - Color burn (and vivid light) at `cb = 1`, `cs -> 0`: the result is
    ///   1 for `cb >= 1`, but about 0 for `cb = 1 - ulp`. Color dodge (and
    ///   vivid light) has the same step at `cb = 0`, `cs -> 1`.
    /// - Hard mix at `cb + cs = 1`.
    /// - Darker color and lighter color when both luminances are equal.
    ///
    /// Values exactly at 1.0 are frequent in the tests (the clamp of
    /// values above 1, and adjustment results at white).
    const STEP_MODES: [BlendMode; 6] = [
        BlendMode::ColorBurn,
        BlendMode::ColorDodge,
        BlendMode::VividLight,
        BlendMode::HardMix,
        BlendMode::DarkerColor,
        BlendMode::LighterColor,
    ];

    /// Compares the GPU and CPU composites of `region` and returns the
    /// largest scaled error of the samples inside the tolerance.
    ///
    /// If `steps` is true (the document uses a mode in [`STEP_MODES`]), at
    /// most 1 sample in 1000 (and at least 1) can be outside the tolerance.
    /// Else all samples must be inside it.
    fn compare(
        ctx: &GpuContext,
        comp: &GpuCompositor,
        doc: &Document,
        region: Rect,
        steps: bool,
        tol: f32,
        what: &str,
    ) -> f32 {
        let cpu = composite(doc, region).unwrap();
        let gpu = pollster::block_on(comp.composite_gpu_image(ctx, doc, region)).unwrap();
        assert_eq!((gpu.width(), gpu.height()), (region.width, region.height));
        let mut worst = 0.0_f32;
        let mut bad = Vec::new();
        for (i, (c, g)) in cpu.as_slice().iter().zip(gpu.as_slice()).enumerate() {
            let err = (g - c).abs() / c.abs().max(1.0);
            assert!(g.is_finite(), "{what}: sample {i} is not finite ({g})");
            if err > tol {
                bad.push((i / 4, i % 4, *c, *g));
            } else {
                worst = worst.max(err);
            }
        }
        let allowed = if steps {
            cpu.as_slice().len() / 1000 + 1
        } else {
            0
        };
        if !bad.is_empty() {
            eprintln!("{what}: {} samples outside the tolerance", bad.len());
        }
        assert!(
            bad.len() <= allowed,
            "{what}: {} samples outside the tolerance, first: {:?}",
            bad.len(),
            &bad[..bad.len().min(5)]
        );
        worst
    }

    #[test]
    fn mode_ids_are_the_menu_index() {
        for (i, mode) in BlendMode::ALL.iter().enumerate() {
            assert_eq!(mode_id(*mode) as usize, i, "{mode:?}");
        }
    }

    #[test]
    fn params_block_size_agrees_with_the_shader() {
        // 5 vec4 fields and an array of 5 vec4: 10 * 16 bytes.
        assert_eq!(std::mem::size_of::<Params>(), 160);
    }

    #[test]
    fn invalid_region_is_rejected() {
        let Some(ctx) = context() else { return };
        let comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let doc = empty_doc(8, 8, BlendSpace::Perceptual);
        for region in [Rect::new(0, 0, 0, 4), Rect::new(4, 4, 5, 2)] {
            let result = pollster::block_on(comp.composite_gpu(&ctx, &doc, region));
            assert!(
                matches!(result, Err(EditorError::InvalidRect { .. })),
                "{region:?}"
            );
        }
    }

    #[test]
    fn empty_document_is_transparent() {
        let Some(ctx) = context() else { return };
        let comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let doc = empty_doc(9, 7, BlendSpace::Perceptual);
        let image = pollster::block_on(comp.composite_gpu_image(&ctx, &doc, doc.bounds())).unwrap();
        assert!(image.as_slice().iter().all(|&v| v == 0.0));
    }

    /// All blend modes in both blend spaces, for pixel layers, adjustment
    /// layers, isolated groups, and pass-through groups, with masks and
    /// offsets.
    #[test]
    fn all_blend_modes_match_cpu() {
        let Some(ctx) = context() else { return };
        let comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(0x1234_5678_9ABC_DEF0);
        let mut worst = 0.0_f32;
        for space in [BlendSpace::Linear, BlendSpace::Perceptual] {
            for (n, mode) in BlendMode::ALL.into_iter().enumerate() {
                let mut doc = empty_doc(40, 32, space);
                add_pixel(&mut doc, &mut rng, None, (40, 32), (0, 0));

                // Pixel layer partly outside the canvas, with a black-fill mask.
                let p = add_pixel(&mut doc, &mut rng, None, (30, 20), (-6, 5));
                doc.set_blend_mode(p, mode).unwrap();
                doc.set_opacity(p, 0.85).unwrap();
                doc.set_fill_opacity(p, 0.9).unwrap();
                set_mask(&mut doc, &mut rng, p, (20, 14), (2, 8), MaskFill::Black);

                // Adjustment layer with a white-fill mask.
                let adjustments = all_adjustments();
                let adj = adjustments[n % adjustments.len()].clone();
                let a = add(&mut doc, None, LayerContent::Adjustment(adj));
                doc.set_blend_mode(a, mode).unwrap();
                doc.set_opacity(a, 0.7).unwrap();
                set_mask(&mut doc, &mut rng, a, (25, 18), (12, -3), MaskFill::White);

                // Isolated group.
                let g = add(
                    &mut doc,
                    None,
                    LayerContent::Group(GroupIsolation::Isolated),
                );
                doc.set_blend_mode(g, mode).unwrap();
                doc.set_opacity(g, 0.9).unwrap();
                add_pixel(&mut doc, &mut rng, Some(g), (25, 25), (10, -4));
                let c = add_pixel(&mut doc, &mut rng, Some(g), (16, 16), (20, 10));
                doc.set_blend_mode(c, BlendMode::Screen).unwrap();

                // Pass-through group with a mask, an adjustment, and a layer.
                let t = add(
                    &mut doc,
                    None,
                    LayerContent::Group(GroupIsolation::PassThrough),
                );
                doc.set_opacity(t, 0.6).unwrap();
                set_mask(&mut doc, &mut rng, t, (30, 30), (5, 5), MaskFill::White);
                let ta = add(
                    &mut doc,
                    Some(t),
                    LayerContent::Adjustment(Adjustment::Levels(levels())),
                );
                doc.set_blend_mode(ta, mode).unwrap();
                let tp = add_pixel(&mut doc, &mut rng, Some(t), (12, 30), (30, 4));
                doc.set_blend_mode(tp, mode).unwrap();

                for region in [doc.bounds(), Rect::new(5, 3, 30, 22)] {
                    let what = format!("{space:?} {mode:?} {region:?}");
                    let steps = STEP_MODES.contains(&mode);
                    let tol = if mode == BlendMode::Divide {
                        DIVIDE_TOL
                    } else {
                        TOL
                    };
                    worst = worst.max(compare(&ctx, &comp, &doc, region, steps, tol, &what));
                }
            }
        }
        eprintln!("all_blend_modes_match_cpu: largest scaled error {worst:e}");
    }

    /// All adjustment kinds, with several blend modes, in both spaces.
    #[test]
    fn adjustments_match_cpu() {
        let Some(ctx) = context() else { return };
        let comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(0x0BAD_CAFE_1234_5678);
        let mut worst = 0.0_f32;
        let modes = [
            BlendMode::Normal,
            BlendMode::Multiply,
            BlendMode::Dissolve,
            BlendMode::Color,
            BlendMode::Overlay,
        ];
        for space in [BlendSpace::Linear, BlendSpace::Perceptual] {
            for adj in all_adjustments() {
                for mode in modes {
                    let mut doc = empty_doc(48, 40, space);
                    add_pixel(&mut doc, &mut rng, None, (48, 40), (0, 0));
                    let a = add(&mut doc, None, LayerContent::Adjustment(adj.clone()));
                    doc.set_blend_mode(a, mode).unwrap();
                    doc.set_opacity(a, 0.8).unwrap();
                    let what = format!("{space:?} {mode:?} {adj:?}");
                    worst = worst.max(compare(&ctx, &comp, &doc, doc.bounds(), false, TOL, &what));
                }
            }
        }
        eprintln!("adjustments_match_cpu: largest scaled error {worst:e}");
    }

    /// Hidden and zero-opacity layers, disabled masks, masks and layers
    /// outside the region, nested groups, and dissolve at all levels.
    #[test]
    fn special_cases_match_cpu() {
        let Some(ctx) = context() else { return };
        let comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(0x5EED_5EED_5EED_5EED);
        let mut worst = 0.0_f32;
        for space in [BlendSpace::Linear, BlendSpace::Perceptual] {
            let mut doc = empty_doc(64, 48, space);
            add_pixel(&mut doc, &mut rng, None, (64, 48), (0, 0));

            // Ignored layers.
            let hidden = add_pixel(&mut doc, &mut rng, None, (64, 48), (0, 0));
            doc.set_visible(hidden, false).unwrap();
            let zero = add_pixel(&mut doc, &mut rng, None, (64, 48), (0, 0));
            doc.set_opacity(zero, 0.0).unwrap();
            let zero_fill = add_pixel(&mut doc, &mut rng, None, (64, 48), (0, 0));
            doc.set_fill_opacity(zero_fill, 0.0).unwrap();

            // Disabled mask: no effect.
            let dm = add_pixel(&mut doc, &mut rng, None, (40, 30), (10, 10));
            set_mask(&mut doc, &mut rng, dm, (10, 10), (0, 0), MaskFill::Black);
            doc.set_mask_enabled(dm, false).unwrap();

            // Mask fully outside the region (black fill): the layer is hidden
            // in the region. Mask fully outside (white fill): no mask effect.
            let mo = add_pixel(&mut doc, &mut rng, None, (64, 48), (0, 0));
            set_mask(&mut doc, &mut rng, mo, (8, 8), (-20, -20), MaskFill::Black);
            let mw = add_pixel(&mut doc, &mut rng, None, (30, 30), (-10, 20));
            doc.set_blend_mode(mw, BlendMode::SoftLight).unwrap();
            set_mask(&mut doc, &mut rng, mw, (8, 8), (100, 100), MaskFill::White);

            // Layer fully outside the canvas.
            add_pixel(&mut doc, &mut rng, None, (10, 10), (70, 0));

            // Dissolve pixel layer with a mask.
            let d = add_pixel(&mut doc, &mut rng, None, (50, 40), (7, 3));
            doc.set_blend_mode(d, BlendMode::Dissolve).unwrap();
            doc.set_opacity(d, 0.6).unwrap();
            set_mask(&mut doc, &mut rng, d, (30, 30), (12, 6), MaskFill::White);

            // Isolated group > pass-through group > isolated group, with an
            // adjustment in the pass-through group that changes the layers
            // below it in the outer isolated group only.
            let outer = add(
                &mut doc,
                None,
                LayerContent::Group(GroupIsolation::Isolated),
            );
            doc.set_blend_mode(outer, BlendMode::Dissolve).unwrap();
            doc.set_opacity(outer, 0.75).unwrap();
            add_pixel(&mut doc, &mut rng, Some(outer), (50, 40), (3, 2));
            let pass = add(
                &mut doc,
                Some(outer),
                LayerContent::Group(GroupIsolation::PassThrough),
            );
            doc.set_opacity(pass, 0.8).unwrap();
            set_mask(&mut doc, &mut rng, pass, (40, 20), (8, 20), MaskFill::Black);
            let pa = add(
                &mut doc,
                Some(pass),
                LayerContent::Adjustment(Adjustment::Curves(curves())),
            );
            doc.set_blend_mode(pa, BlendMode::Dissolve).unwrap();
            doc.set_opacity(pa, 0.5).unwrap();
            let inner = add(
                &mut doc,
                Some(pass),
                LayerContent::Group(GroupIsolation::Isolated),
            );
            doc.set_blend_mode(inner, BlendMode::Luminosity).unwrap();
            add_pixel(&mut doc, &mut rng, Some(inner), (20, 20), (30, 20));
            let ia = add(
                &mut doc,
                Some(inner),
                LayerContent::Adjustment(Adjustment::HueSaturation(HueSaturation {
                    hue: 90.0,
                    saturation: 0.5,
                    lightness: 0.1,
                })),
            );
            doc.set_blend_mode(ia, BlendMode::Hue).unwrap();
            // An empty pass-through group and an empty isolated group.
            add(
                &mut doc,
                Some(outer),
                LayerContent::Group(GroupIsolation::PassThrough),
            );
            add(
                &mut doc,
                None,
                LayerContent::Group(GroupIsolation::Isolated),
            );

            for region in [
                doc.bounds(),
                Rect::new(9, 5, 40, 33),
                Rect::new(63, 47, 1, 1),
            ] {
                let what = format!("special cases {space:?} {region:?}");
                worst = worst.max(compare(&ctx, &comp, &doc, region, false, TOL, &what));
            }
        }
        eprintln!("special_cases_match_cpu: largest scaled error {worst:e}");
    }

    /// The composite of a region is the crop of the full composite (also
    /// for dissolve, which uses canvas coordinates).
    #[test]
    fn region_is_crop_of_full_composite() {
        let Some(ctx) = context() else { return };
        let comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(42);
        let mut doc = empty_doc(32, 24, BlendSpace::Perceptual);
        add_pixel(&mut doc, &mut rng, None, (32, 24), (0, 0));
        let d = add_pixel(&mut doc, &mut rng, None, (32, 24), (0, 0));
        doc.set_blend_mode(d, BlendMode::Dissolve).unwrap();
        doc.set_opacity(d, 0.5).unwrap();
        let full = pollster::block_on(comp.composite_gpu_image(&ctx, &doc, doc.bounds())).unwrap();
        let region = Rect::new(7, 5, 13, 11);
        let part = pollster::block_on(comp.composite_gpu_image(&ctx, &doc, region)).unwrap();
        for y in 0..region.height {
            for x in 0..region.width {
                assert_eq!(
                    part.pixel(x, y),
                    full.pixel(x + region.x, y + region.y),
                    "({x}, {y})"
                );
            }
        }
    }

    /// Benchmark: a 4096 x 4096 document with 10 layers in the perceptual
    /// space, CPU against GPU. Run with
    /// `cargo test -p expose-editor --release -- --ignored --nocapture bench_`.
    #[test]
    #[ignore = "benchmark; run manually in release mode"]
    fn bench_composite_4096_perceptual() {
        use std::time::Instant;

        let Some(ctx) = context() else { return };
        let comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(7);
        let size = 4096;
        let mut doc = empty_doc(size, size, BlendSpace::Perceptual);
        let full_modes = [
            BlendMode::Normal,
            BlendMode::Multiply,
            BlendMode::Screen,
            BlendMode::Overlay,
            BlendMode::SoftLight,
            BlendMode::Color,
        ];
        for mode in full_modes {
            let id = add_pixel(&mut doc, &mut rng, None, (size, size), (0, 0));
            doc.set_blend_mode(id, mode).unwrap();
            doc.set_opacity(id, 0.8).unwrap();
        }
        let half = add_pixel(&mut doc, &mut rng, None, (2048, 2048), (1000, 1500));
        set_mask(
            &mut doc,
            &mut rng,
            half,
            (2048, 2048),
            (1000, 1500),
            MaskFill::Black,
        );
        let add_layer = add_pixel(&mut doc, &mut rng, None, (2048, 2048), (-500, 2600));
        doc.set_blend_mode(add_layer, BlendMode::LinearDodge)
            .unwrap();
        add(
            &mut doc,
            None,
            LayerContent::Adjustment(Adjustment::Curves(curves())),
        );
        add(
            &mut doc,
            None,
            LayerContent::Adjustment(Adjustment::HueSaturation(HueSaturation {
                hue: 20.0,
                saturation: 0.2,
                lightness: 0.0,
            })),
        );
        let region = doc.bounds();

        let t = Instant::now();
        let cpu = composite(&doc, region).unwrap();
        let cpu_ms = t.elapsed().as_secs_f64() * 1e3;

        // The first call compiles the pipelines.
        let first = Instant::now();
        drop(pollster::block_on(comp.composite_gpu(&ctx, &doc, region)).unwrap());
        ctx.wait_idle().unwrap();
        let first_ms = first.elapsed().as_secs_f64() * 1e3;

        let mut gpu_ms = f64::MAX;
        for _ in 0..3 {
            let t = Instant::now();
            drop(pollster::block_on(comp.composite_gpu(&ctx, &doc, region)).unwrap());
            ctx.wait_idle().unwrap();
            gpu_ms = gpu_ms.min(t.elapsed().as_secs_f64() * 1e3);
        }

        // The upload part of the GPU time.
        let t = Instant::now();
        let mut uploads = Vec::new();
        for layer in doc.layers() {
            if let LayerKind::Pixel(p) = layer.kind() {
                let img = p.image();
                uploads.push(
                    pollster::block_on(GpuImage::from_rgba_f32(
                        &ctx,
                        img.width(),
                        img.height(),
                        FORMAT,
                        img.as_slice(),
                    ))
                    .unwrap(),
                );
            }
        }
        ctx.wait_idle().unwrap();
        let upload_ms = t.elapsed().as_secs_f64() * 1e3;
        drop(uploads);

        let t = Instant::now();
        let gpu = pollster::block_on(comp.composite_gpu_image(&ctx, &doc, region)).unwrap();
        let readback_ms = t.elapsed().as_secs_f64() * 1e3 - gpu_ms;
        assert_eq!(gpu.width(), cpu.width());

        eprintln!(
            "bench 4096x4096, 10 layers, perceptual: CPU {cpu_ms:.1} ms; GPU first call \
             {first_ms:.1} ms, GPU {gpu_ms:.1} ms (of which layer upload about \
             {upload_ms:.1} ms, kernels about {:.1} ms); read-back about {readback_ms:.1} ms",
            gpu_ms - upload_ms
        );
    }
}
