//! Fast interactive preview: downsample once, then develop the small image
//! on the GPU for each slider change.
//!
//! # Downsample filter
//!
//! [`downsample_area`] uses an area (box) filter: each output pixel is the
//! mean of the source area that it covers, with fractional weights for the
//! source pixels at its edges. The filter works in linear light, so the
//! mean brightness stays correct. It is separable (rows, then columns) and
//! keeps only one filtered source row in memory.
//!
//! An area filter has no ringing and no negative lobes. It is slightly soft
//! compared with a windowed sinc filter; for a screen preview this is not
//! important.
//!
//! The downsample runs on the CPU. Do it once for each photo (see
//! [`PreviewRenderer`]), not for each slider change.

use expose_core::CoreError;
use expose_core::buffer::{Rgba, WorkingImage};
use expose_core::develop::DevelopSettings;
use expose_gpu::{GpuContext, GpuImage, ImageFormat};

use crate::develop_gpu::DevelopTargets;
use crate::error::{DevelopError, DevelopResult};
use crate::pipeline::{Pipeline, read_working_image};
use crate::source::SourceInfo;

/// Returns the preview size: the source size scaled so that the long edge
/// is at most `max_long_edge`, with the aspect ratio kept. A source that is
/// already small enough keeps its size (no upscaling). Each side is at
/// least 1.
pub fn preview_size(width: u32, height: u32, max_long_edge: u32) -> DevelopResult<(u32, u32)> {
    if max_long_edge == 0 {
        return Err(DevelopError::InvalidPreviewSize);
    }
    let long = width.max(height);
    if long <= max_long_edge {
        return Ok((width, height));
    }
    let scale = |side: u32| -> u32 {
        // side * max / long < side <= u32::MAX, so the result fits in u32.
        let v =
            (u64::from(side) * u64::from(max_long_edge) + u64::from(long) / 2) / u64::from(long);
        u32::try_from(v).unwrap_or(u32::MAX).max(1)
    };
    Ok((scale(width), scale(height)))
}

/// The source pixels and weights of one output pixel on one axis.
struct Taps {
    first: usize,
    weights: Vec<f32>,
}

/// Builds the area filter taps for `src` pixels to `dst` pixels (`dst <=
/// src`). Output pixel `d` covers the source interval `[d * s, (d + 1) * s)`
/// with `s = src / dst`. The weights are the overlap lengths divided by `s`,
/// so they sum to 1.
fn area_taps(src: u32, dst: u32) -> Vec<Taps> {
    let scale = f64::from(src) / f64::from(dst);
    (0..dst)
        .map(|d| {
            let start = f64::from(d) * scale;
            let end = (f64::from(d) + 1.0) * scale;
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "start and end are in 0..=src, which fits in usize"
            )]
            let (first, last) = (
                start.floor() as usize,
                (end.ceil() as usize).min(src as usize),
            );
            let weights = (first..last)
                .map(|i| {
                    #[allow(clippy::cast_precision_loss, reason = "pixel index fits in f64")]
                    let (lo, hi) = (i as f64, i as f64 + 1.0);
                    let overlap = hi.min(end) - lo.max(start);
                    #[allow(clippy::cast_possible_truncation, reason = "weights are in 0..=1")]
                    let w = (overlap.max(0.0) / scale) as f32;
                    w
                })
                .collect();
            Taps { first, weights }
        })
        .collect()
}

/// Filters one source row horizontally into `out`.
fn filter_row(row: &[Rgba], taps: &[Taps], out: &mut [Rgba]) {
    for (o, t) in out.iter_mut().zip(taps) {
        let mut acc = [0.0_f32; 4];
        // `t.first = floor(d * s) < dst * s = row.len()`, so the slice
        // start is in bounds.
        for (p, w) in row[t.first..].iter().zip(&t.weights) {
            for c in 0..4 {
                acc[c] += p[c] * w;
            }
        }
        *o = acc;
    }
}

/// Downsamples `image` with an area filter so that its long edge is at most
/// `max_long_edge` (see the module documentation). Returns a copy when the
/// image is already small enough.
pub fn downsample_area(image: &WorkingImage, max_long_edge: u32) -> DevelopResult<WorkingImage> {
    let (sw, sh) = (image.width(), image.height());
    let (dw, dh) = preview_size(sw, sh, max_long_edge)?;
    if (dw, dh) == (sw, sh) {
        return Ok(image.clone());
    }
    let x_taps = area_taps(sw, dw);
    let y_taps = area_taps(sh, dh);
    let src_rows: Vec<&[Rgba]> = image.rows().collect();
    let mut out = WorkingImage::new(dw, dh)?;
    // The last filtered source row and its index. Neighbor output rows
    // share at most one source row, so one cached row is sufficient.
    let mut cached: Option<usize> = None;
    let mut filtered = vec![[0.0_f32; 4]; x_taps.len()];
    for (out_row, taps) in out.rows_mut().zip(&y_taps) {
        for (k, w) in taps.weights.iter().enumerate() {
            let src_y = taps.first + k;
            if cached != Some(src_y) {
                // `area_taps` gives only rows below `sh`. Report an
                // internal error if this is not true; do not skip the row.
                let row = src_rows
                    .get(src_y)
                    .ok_or(DevelopError::Internal("area filter row out of range"))?;
                filter_row(row, &x_taps, &mut filtered);
                cached = Some(src_y);
            }
            for (o, f) in out_row.iter_mut().zip(&filtered) {
                for c in 0..4 {
                    o[c] += f[c] * w;
                }
            }
        }
    }
    Ok(out)
}

/// A preview of one photo on the GPU. It keeps the downsampled source and
/// the output images on the GPU, so that each slider change needs only the
/// kernel runs and one read-back.
///
/// There are two develop paths:
///
/// - [`Self::develop`]: the tone stages only. The result is in
///   [`Self::output`] and has the preview size [`Self::size`].
/// - [`Self::develop_full`]: all stages (geometry, tone, sharpening). The
///   result size is `pipeline.output_size(size)`, which differs from
///   [`Self::size`] when a crop applies. The caller must use the size of the
///   returned image for the next step (for example, the display encode).
#[derive(Debug)]
pub struct PreviewRenderer {
    input: GpuImage,
    output: GpuImage,
    pixel_scale: f32,
    targets: DevelopTargets,
}

impl PreviewRenderer {
    /// Downsamples `source` to `max_long_edge` and uploads it.
    ///
    /// The pixel scale is `preview long edge / source long edge`. When
    /// `source` is itself a reduced copy of the original, call
    /// [`Self::set_original_size`].
    pub async fn new(
        ctx: &GpuContext,
        source: &WorkingImage,
        max_long_edge: u32,
    ) -> DevelopResult<Self> {
        let small = downsample_area(source, max_long_edge)?;
        let (w, h) = (small.width(), small.height());
        let pixel_scale = long_edge_ratio((w, h), (source.width(), source.height()))?;
        let input =
            GpuImage::from_rgba_f32(ctx, w, h, ImageFormat::Rgba32Float, small.as_slice()).await?;
        let output = GpuImage::new(ctx, w, h, ImageFormat::Rgba32Float).await?;
        Ok(Self {
            input,
            output,
            pixel_scale,
            targets: DevelopTargets::new(),
        })
    }

    /// The preview width and height.
    pub fn size(&self) -> (u32, u32) {
        (self.input.width(), self.input.height())
    }

    /// Output pixels for each full-resolution source pixel (at most 1.0
    /// when the preview is not larger than the original).
    pub fn pixel_scale(&self) -> f32 {
        self.pixel_scale
    }

    /// Sets the size of the full-resolution original, so that the pixel
    /// scale is `preview long edge / original long edge`.
    ///
    /// # Errors
    ///
    /// [`DevelopError::Core`] when a side is zero.
    pub fn set_original_size(&mut self, width: u32, height: u32) -> DevelopResult<()> {
        self.pixel_scale = long_edge_ratio(self.size(), (width, height))?;
        Ok(())
    }

    /// Runs the tone stages of `pipeline` on the GPU. The result stays on
    /// the GPU in [`Self::output`] (for display without a read-back).
    /// Geometry and sharpening are not applied; see [`Self::develop_full`].
    pub async fn develop(&self, ctx: &GpuContext, pipeline: &Pipeline) -> DevelopResult<()> {
        pipeline.render_gpu(ctx, &self.input, &self.output).await
    }

    /// Runs all stages of `pipeline` on the GPU at [`Self::pixel_scale`].
    /// Returns the result image (it stays on the GPU). Its size is
    /// `pipeline.output_size(self.size())`. The internal images are made
    /// again only when that size changes.
    ///
    /// # Errors
    ///
    /// See [`Pipeline::develop_gpu`].
    pub async fn develop_full(
        &mut self,
        ctx: &GpuContext,
        pipeline: &Pipeline,
    ) -> DevelopResult<&GpuImage> {
        pipeline
            .develop_gpu(ctx, &self.input, self.pixel_scale, &mut self.targets)
            .await
    }

    /// The intermediate images of [`Self::develop_full`] (for example, to
    /// read [`DevelopTargets::allocation_count`]).
    pub fn targets(&self) -> &DevelopTargets {
        &self.targets
    }

    /// The developed preview on the GPU (valid after [`Self::develop`]).
    pub fn output(&self) -> &GpuImage {
        &self.output
    }

    /// Runs `pipeline` and reads the preview back.
    pub async fn render(
        &self,
        ctx: &GpuContext,
        pipeline: &Pipeline,
    ) -> DevelopResult<WorkingImage> {
        self.develop(ctx, pipeline).await?;
        read_working_image(ctx, &self.output).await
    }
}

/// Returns `long edge of size / long edge of original`.
fn long_edge_ratio(size: (u32, u32), original: (u32, u32)) -> DevelopResult<f32> {
    let long = original.0.max(original.1);
    if original.0 == 0 || original.1 == 0 {
        return Err(CoreError::InvalidDimensions {
            width: original.0,
            height: original.1,
        }
        .into());
    }
    let ratio = f64::from(size.0.max(size.1)) / f64::from(long);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the ratio of two u32 values is a finite, positive f32 range value"
    )]
    Ok(ratio as f32)
}

/// Develops a preview of `source`: downsample to `max_long_edge` (area
/// filter), then run the pipeline on the GPU.
///
/// For repeated renders of one photo (slider changes), use
/// [`PreviewRenderer`], so that the downsample and upload happen once.
pub async fn render_preview(
    ctx: &GpuContext,
    source: &WorkingImage,
    info: &SourceInfo,
    settings: &DevelopSettings,
    max_long_edge: u32,
) -> DevelopResult<WorkingImage> {
    let pipeline = Pipeline::new(settings, info)?;
    let renderer = PreviewRenderer::new(ctx, source, max_long_edge).await?;
    renderer.render(ctx, &pipeline).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_size_keeps_aspect_and_does_not_upscale() {
        assert_eq!(preview_size(6000, 4000, 2560).unwrap(), (2560, 1707));
        assert_eq!(preview_size(4000, 6000, 2560).unwrap(), (1707, 2560));
        assert_eq!(preview_size(800, 600, 2560).unwrap(), (800, 600));
        assert_eq!(preview_size(10_000, 1, 100).unwrap(), (100, 1));
        assert!(matches!(
            preview_size(10, 10, 0),
            Err(DevelopError::InvalidPreviewSize)
        ));
    }

    #[test]
    fn taps_sum_to_one() {
        for (src, dst) in [(10, 3), (6000, 2560), (7, 7), (5, 1)] {
            for t in area_taps(src, dst) {
                let s: f32 = t.weights.iter().sum();
                assert!((s - 1.0).abs() < 1e-5, "{src}->{dst}: {s}");
            }
        }
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "quarter weights on small integers are exact"
    )]
    fn integer_factor_is_block_mean() {
        let pixels: Vec<Rgba> = (0..16_u8).map(|i| [f32::from(i), 0.0, 1.0, 1.0]).collect();
        let img = WorkingImage::from_pixels(4, 4, pixels).unwrap();
        let small = downsample_area(&img, 2).unwrap();
        assert_eq!((small.width(), small.height()), (2, 2));
        // Top-left block: 0, 1, 4, 5 -> mean 2.5.
        assert_eq!(small.pixel(0, 0).unwrap(), [2.5, 0.0, 1.0, 1.0]);
        // Bottom-right block: 10, 11, 14, 15 -> mean 12.5.
        assert_eq!(small.pixel(1, 1).unwrap(), [12.5, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn fractional_factor_keeps_mean() {
        let (w, h) = (37_u32, 23_u32);
        let pixels: Vec<Rgba> = (0..w * h)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let v = (i % 11) as f32 * 0.3 - 0.5;
                [v, 2.0 * v, 1.0, 1.0]
            })
            .collect();
        let img = WorkingImage::from_pixels(w, h, pixels).unwrap();
        let small = downsample_area(&img, 10).unwrap();
        assert_eq!((small.width(), small.height()), (10, 6));
        #[allow(clippy::cast_precision_loss)]
        let mean = |im: &WorkingImage| {
            im.pixels().iter().map(|p| f64::from(p[0])).sum::<f64>() / im.pixels().len() as f64
        };
        assert!((mean(&img) - mean(&small)).abs() < 1e-4);
        assert!(small.pixels().iter().all(|p| (p[2] - 1.0).abs() < 1e-5));
    }
}
