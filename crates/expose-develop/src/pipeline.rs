//! The develop pipeline: settings to stages, and the CPU reference.

use expose_core::CoreError;
use expose_core::buffer::{Rgba, WorkingImage};
use expose_core::color::{Chromaticity, Mat3, TransferFunction, luminance, mul_vec_f32};
use expose_core::develop::{DevelopSettings, NormalizedRect};
use expose_gpu::{GpuContext, GpuImage, ImageFormat};

use crate::curve::CurveTables;
use crate::error::{DevelopError, DevelopResult};
use crate::geometry::Geometry;
use crate::gpu::{DEVELOP_KERNEL, GpuParams};
use crate::sharpen::Sharpener;
use crate::source::SourceInfo;
use crate::tone::{ToneLut, ToneMap, lut_delta};
use crate::white_balance::{scene_white, white_balance_matrix};

/// A white balance matrix closer than this to the identity (each element)
/// is treated as the identity. The limit is far below `f32` resolution
/// (about `6e-8` at 1.0), so the result is the same in `f32`.
const IDENTITY_MATRIX_TOLERANCE: f64 = 1e-9;

/// The transfer function of the tone curve encoding.
const CURVE_ENCODING: TransferFunction = TransferFunction::Srgb;

/// One active stage of the pipeline, in pipeline order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stage {
    /// Flip, straighten, and crop (see [`crate::geometry`]). The output
    /// size depends on the source size: see [`Pipeline::output_size`].
    Geometry {
        /// The crop rect, normalized to the source frame.
        rect: NormalizedRect,
        /// Straighten angle in degrees, clockwise.
        angle: f32,
        /// Mirror left to right.
        flip_horizontal: bool,
        /// Mirror top to bottom.
        flip_vertical: bool,
    },
    /// Chromatic adaptation from `scene_white` to D65.
    WhiteBalance {
        /// The white point of the scene light.
        scene_white: Chromaticity,
    },
    /// Multiply by `2^ev`.
    Exposure {
        /// Exposure change in EV.
        ev: f32,
    },
    /// Highlights, shadows, whites, and blacks (slider values).
    ToneZones {
        /// Highlights slider.
        highlights: f32,
        /// Shadows slider.
        shadows: f32,
        /// Whites slider.
        whites: f32,
        /// Blacks slider.
        blacks: f32,
    },
    /// Contrast S-curve around mid gray.
    Contrast {
        /// Contrast slider.
        amount: f32,
    },
    /// Point tone curve.
    ToneCurve {
        /// Active curves: master, red, green, blue.
        active: [bool; 4],
    },
    /// Luminance sharpening (see [`crate::sharpen`]). At a small pixel
    /// scale the stage can be a no-op (see [`Sharpener::new`]).
    Sharpen {
        /// Amount slider.
        amount: f32,
        /// Radius slider, in source pixels.
        radius: f32,
    },
}

/// A compiled develop pipeline for one set of settings and one source.
///
/// [`Pipeline::new`] validates the settings and precomputes all per-stage
/// data once. Then each pixel needs only a 3x3 matrix, one LUT read, and
/// (with a tone curve) two curve reads per channel.
///
/// # Stage groups
///
/// The full develop has three groups, in this order:
///
/// 1. Geometry (flip, straighten, crop). It needs the source size, so the
///    pipeline keeps the crop settings and makes a [`Geometry`] for each
///    size ([`Pipeline::geometry`]).
/// 2. Tone stages (per pixel): white balance, exposure, tone zones,
///    contrast, tone curve.
/// 3. Sharpening. It needs the pixel scale, so the pipeline makes a
///    [`Sharpener`] for each scale ([`Pipeline::sharpener`]).
///
/// [`Pipeline::develop_cpu`] and [`Pipeline::develop_gpu`] run all three
/// groups. [`Pipeline::apply_cpu`], [`Pipeline::render_cpu`], and
/// [`Pipeline::render_gpu`] run the tone stages only.
///
/// # Geometry switch
///
/// [`Pipeline::set_apply_geometry`] turns the geometry group off (for
/// example, the crop tool shows the full image while the user drags the
/// frame). Then the full develop, [`Pipeline::output_size`],
/// [`Pipeline::stages`], and [`Pipeline::is_identity`] ignore the crop
/// settings.
///
/// # Fused stages
///
/// - White balance and exposure are both linear, so they are one matrix:
///   `2^ev * WB`.
/// - Highlights, shadows, whites, blacks, and contrast all multiply RGB by a
///   luminance ratio. Thus they are one function of luminance, kept in one
///   LUT (see [`crate::tone`]).
///
/// Fusion does not change the result: it is the same math in the same
/// order.
#[derive(Debug, Clone)]
pub struct Pipeline {
    settings: DevelopSettings,
    /// All stages in order. When `has_geometry`, `stages[0]` is the
    /// geometry stage.
    stages: Vec<Stage>,
    has_geometry: bool,
    has_sharpening: bool,
    apply_geometry: bool,
    matrix: Option<[[f32; 3]; 3]>,
    tone_lut: Option<Box<ToneLut>>,
    curves: Option<CurveTables>,
    gpu_params: Box<GpuParams>,
}

impl Pipeline {
    /// Validates `settings` (a copy; see `DevelopSettings::validate`) and
    /// compiles the stages for `source`.
    ///
    /// # Errors
    ///
    /// - [`DevelopError::Settings`] when the settings are not valid.
    /// - [`DevelopError::Source`] when the as-shot white is not valid.
    pub fn new(settings: &DevelopSettings, source: &SourceInfo) -> DevelopResult<Self> {
        let mut settings = settings.clone();
        settings.validate().map_err(DevelopError::Settings)?;
        let mut stages = Vec::new();

        // 0. Geometry. The check does not need the source size. A rect that
        // is a little smaller than the frame can round to the full frame
        // for a small source; then `Geometry::is_identity` is true and the
        // full develop skips the stage.
        let crop = &settings.crop;
        let has_geometry = crop.rect != NormalizedRect::FULL
            || crop.angle != 0.0
            || crop.flip_horizontal
            || crop.flip_vertical;
        if has_geometry {
            stages.push(Stage::Geometry {
                rect: crop.rect,
                angle: crop.angle,
                flip_horizontal: crop.flip_horizontal,
                flip_vertical: crop.flip_vertical,
            });
        }

        // 1. White balance.
        let mut matrix: Option<Mat3> = None;
        if let Some(white) = scene_white(&settings.white_balance, source)? {
            let m = white_balance_matrix(white).map_err(DevelopError::Settings)?;
            if !is_near_identity(&m) {
                stages.push(Stage::WhiteBalance { scene_white: white });
                matrix = Some(m);
            }
        }

        // 2. Exposure.
        let ev = settings.tone.exposure;
        if ev != 0.0 {
            stages.push(Stage::Exposure { ev });
            let gain = f64::from(ev).exp2();
            matrix = Some(Mat3::diagonal([gain; 3]) * matrix.unwrap_or(Mat3::IDENTITY));
        }

        // 3. and 4. Tone zones and contrast.
        let tone = &settings.tone;
        let tone_map = ToneMap::from_sliders(tone);
        if !tone_map.zones_are_identity() {
            stages.push(Stage::ToneZones {
                highlights: tone.highlights,
                shadows: tone.shadows,
                whites: tone.whites,
                blacks: tone.blacks,
            });
        }
        if tone_map.contrast != 0.0 {
            stages.push(Stage::Contrast {
                amount: tone.contrast,
            });
        }
        let tone_lut = (!tone_map.is_identity()).then(|| tone_map.build_lut());

        // 5. Tone curve.
        let curves = CurveTables::build(&settings.tone_curve).map_err(DevelopError::Settings)?;
        if let Some(c) = &curves {
            stages.push(Stage::ToneCurve { active: c.active });
        }

        // 6. Sharpening.
        let sharpening = &settings.sharpening;
        let has_sharpening = sharpening.amount > 0.0;
        if has_sharpening {
            stages.push(Stage::Sharpen {
                amount: sharpening.amount,
                radius: sharpening.radius,
            });
        }

        let matrix = matrix.map(|m| m.to_f32());
        let gpu_params = GpuParams::build(matrix.as_ref(), tone_lut.as_deref(), curves.as_ref());
        Ok(Self {
            settings,
            stages,
            has_geometry,
            has_sharpening,
            apply_geometry: true,
            matrix,
            tone_lut,
            curves,
            gpu_params,
        })
    }

    /// The validated settings.
    pub fn settings(&self) -> &DevelopSettings {
        &self.settings
    }

    /// The active stages, in order. Identity stages are not in the list.
    /// When the geometry switch is off, the geometry stage is not in the
    /// list.
    pub fn stages(&self) -> &[Stage] {
        let skip = usize::from(self.has_geometry && !self.apply_geometry);
        // `skip` is 1 only when `stages[0]` exists (the geometry stage).
        self.stages.get(skip..).unwrap_or(&[])
    }

    /// Returns `true` when no stage of any group is active, so the full
    /// develop ([`Self::develop_cpu`]) gives the input unchanged.
    ///
    /// This check does not know the source size or the pixel scale, so it
    /// is conservative: a crop that rounds to the full frame, or a
    /// sharpening that is a no-op at a small scale, still counts as active.
    /// For the tone stages only, use [`Self::tone_is_identity`].
    pub fn is_identity(&self) -> bool {
        self.stages().is_empty()
    }

    /// Returns `true` when all tone stages are the identity, so
    /// [`Self::apply_cpu`], [`Self::render_cpu`], and [`Self::render_gpu`]
    /// do not change the image.
    pub fn tone_is_identity(&self) -> bool {
        self.matrix.is_none() && self.tone_lut.is_none() && self.curves.is_none()
    }

    /// Returns `true` when the crop settings are not the default (a rect
    /// other than the full frame, an angle, or a flip). The geometry switch
    /// does not change this value; see [`Self::applies_geometry`].
    pub fn has_geometry(&self) -> bool {
        self.has_geometry
    }

    /// Returns `true` when the sharpening amount is more than 0.
    pub fn has_sharpening(&self) -> bool {
        self.has_sharpening
    }

    /// Turns the geometry group on or off (on by default). See the type
    /// documentation.
    pub fn set_apply_geometry(&mut self, on: bool) {
        self.apply_geometry = on;
    }

    /// Returns `true` when the full develop applies the geometry group.
    pub fn applies_geometry(&self) -> bool {
        self.apply_geometry
    }

    /// The geometry of the crop settings for a source of the given size.
    /// The geometry switch does not change the result.
    ///
    /// # Errors
    ///
    /// See [`Geometry::new`].
    pub fn geometry(&self, source_width: u32, source_height: u32) -> DevelopResult<Geometry> {
        Geometry::new(&self.settings.crop, source_width, source_height)
    }

    /// The sharpening stage at `pixel_scale` (output pixels for each
    /// full-resolution source pixel), or `None` when it is a no-op at that
    /// scale.
    ///
    /// # Errors
    ///
    /// See [`Sharpener::new`] (for example, `pixel_scale` is not finite or
    /// not more than 0).
    pub fn sharpener(&self, pixel_scale: f32) -> DevelopResult<Option<Sharpener>> {
        Sharpener::new(&self.settings.sharpening, pixel_scale)
    }

    /// The size of the full develop result for a source of the given size.
    /// It is the geometry output size when the geometry group applies, and
    /// the source size otherwise.
    ///
    /// # Errors
    ///
    /// [`DevelopError::Core`] when a side is zero.
    pub fn output_size(&self, source_width: u32, source_height: u32) -> DevelopResult<(u32, u32)> {
        if self.has_geometry && self.apply_geometry {
            return Ok(self.geometry(source_width, source_height)?.output_size());
        }
        if source_width == 0 || source_height == 0 {
            return Err(CoreError::InvalidDimensions {
                width: source_width,
                height: source_height,
            }
            .into());
        }
        Ok((source_width, source_height))
    }

    /// The geometry that the full develop applies to a source of the given
    /// size, or `None` when the group is off or the geometry is the
    /// identity at this size.
    pub(crate) fn active_geometry(
        &self,
        source_width: u32,
        source_height: u32,
    ) -> DevelopResult<Option<Geometry>> {
        if !(self.has_geometry && self.apply_geometry) {
            return Ok(None);
        }
        let geometry = self.geometry(source_width, source_height)?;
        Ok((!geometry.is_identity()).then_some(geometry))
    }

    /// Runs the full develop on the CPU: geometry, then the tone stages,
    /// then sharpening.
    ///
    /// `pixel_scale` is the number of output pixels for each
    /// full-resolution source pixel: 1.0 when `image` is the full-size
    /// source, and `image long edge / original long edge` for a downsampled
    /// image. It sets the sharpening radius in pixels.
    ///
    /// The result has the size [`Self::output_size`]. It is deterministic:
    /// the same input, settings, and scale give the same bits.
    ///
    /// # Errors
    ///
    /// - [`DevelopError::Core`] when `pixel_scale` is not finite or not more
    ///   than 0, or a buffer cannot be made.
    pub fn develop_cpu(
        &self,
        image: &WorkingImage,
        pixel_scale: f32,
    ) -> DevelopResult<WorkingImage> {
        let sharpener = self.sharpener(pixel_scale)?;
        let mut out = match self.active_geometry(image.width(), image.height())? {
            Some(geometry) => geometry.apply_cpu(image)?,
            None => image.clone(),
        };
        self.finish_cpu(&mut out, sharpener.as_ref())?;
        Ok(out)
    }

    /// Same as [`Self::develop_cpu`], but takes the image by value. When the
    /// geometry is the identity, the stages work in place and no copy is
    /// made (use it for an export of a decoded file).
    ///
    /// # Errors
    ///
    /// See [`Self::develop_cpu`].
    pub fn develop_cpu_owned(
        &self,
        image: WorkingImage,
        pixel_scale: f32,
    ) -> DevelopResult<WorkingImage> {
        let sharpener = self.sharpener(pixel_scale)?;
        let mut out = match self.active_geometry(image.width(), image.height())? {
            Some(geometry) => geometry.apply_cpu(&image)?,
            None => image,
        };
        self.finish_cpu(&mut out, sharpener.as_ref())?;
        Ok(out)
    }

    /// The tone stages and the sharpening of the full CPU develop.
    fn finish_cpu(
        &self,
        image: &mut WorkingImage,
        sharpener: Option<&Sharpener>,
    ) -> DevelopResult<()> {
        self.apply_cpu(image);
        if let Some(s) = sharpener {
            s.apply_cpu(image)?;
        }
        Ok(())
    }

    /// The fused white balance and exposure matrix (row-major), or `None`
    /// when both are the identity.
    pub fn matrix(&self) -> Option<&[[f32; 3]; 3]> {
        self.matrix.as_ref()
    }

    /// The parameter block for [`DEVELOP_KERNEL`], for callers that run the
    /// kernel with `GpuContext::run_kernel` in their own render graph.
    pub fn gpu_params(&self) -> &GpuParams {
        &self.gpu_params
    }

    /// Applies the tone stages to one pixel. This is the CPU reference: the GPU
    /// kernel does the same arithmetic. Alpha is not changed.
    #[inline]
    pub fn apply_pixel(&self, pixel: Rgba) -> Rgba {
        let mut rgb = [pixel[0], pixel[1], pixel[2]];
        if let Some(m) = &self.matrix {
            rgb = mul_vec_f32(m, rgb);
        }
        if let Some(lut) = &self.tone_lut {
            let ratio = lut_delta(lut, luminance(rgb)).exp2();
            rgb = rgb.map(|v| v * ratio);
        }
        if let Some(curves) = &self.curves {
            for (ch, value) in rgb.iter_mut().enumerate() {
                let mut x = CURVE_ENCODING.encode(*value);
                if curves.active[0] {
                    x = curves.apply(0, x);
                }
                if curves.active[ch + 1] {
                    x = curves.apply(ch + 1, x);
                }
                *value = CURVE_ENCODING.decode(x);
            }
        }
        [rgb[0], rgb[1], rgb[2], pixel[3]]
    }

    /// Applies the tone stages to `image` in place, on the CPU. Geometry and
    /// sharpening are not applied; use [`Self::develop_cpu`] for all stages.
    /// When only geometry or sharpening is active, the image does not
    /// change.
    ///
    /// The result is deterministic: the same input and settings give the
    /// same bits. The loop is single-threaded, so that it also runs in a
    /// WebAssembly build without threads.
    pub fn apply_cpu(&self, image: &mut WorkingImage) {
        if self.tone_is_identity() {
            return;
        }
        for pixel in image.pixels_mut() {
            *pixel = self.apply_pixel(*pixel);
        }
    }

    /// Returns a copy of `image` with the tone stages applied, made on the
    /// CPU. Geometry and sharpening are not applied; use
    /// [`Self::develop_cpu`] for all stages.
    pub fn render_cpu(&self, image: &WorkingImage) -> WorkingImage {
        let mut out = image.clone();
        self.apply_cpu(&mut out);
        out
    }

    /// Runs the tone stages (the per-pixel develop kernel) from `input` to
    /// `output` on the GPU. Geometry and sharpening are not applied; use
    /// [`Self::develop_gpu`] for all stages.
    ///
    /// The images must have the same size and must be different images.
    /// The function submits the work and does not wait for it.
    pub async fn render_gpu(
        &self,
        ctx: &GpuContext,
        input: &GpuImage,
        output: &GpuImage,
    ) -> DevelopResult<()> {
        ctx.run_kernel(&DEVELOP_KERNEL, &*self.gpu_params, input, output)
            .await?;
        Ok(())
    }

    /// Uploads `image`, applies the tone stages on the GPU (`rgba32float`),
    /// and reads the result back. Use [`Self::develop_image_gpu`] for all
    /// stages.
    pub async fn render_image_gpu(
        &self,
        ctx: &GpuContext,
        image: &WorkingImage,
    ) -> DevelopResult<WorkingImage> {
        let (width, height) = (image.width(), image.height());
        let input = GpuImage::from_rgba_f32(
            ctx,
            width,
            height,
            ImageFormat::Rgba32Float,
            image.as_slice(),
        )
        .await?;
        let output = GpuImage::new(ctx, width, height, ImageFormat::Rgba32Float).await?;
        self.render_gpu(ctx, &input, &output).await?;
        read_working_image(ctx, &output).await
    }
}

/// Reads a GPU image back into a working image.
pub(crate) async fn read_working_image(
    ctx: &GpuContext,
    image: &GpuImage,
) -> DevelopResult<WorkingImage> {
    let data = image.read_rgba_f32(ctx).await?;
    if data.len() != image.sample_count() {
        return Err(DevelopError::ReadbackLength {
            expected: image.sample_count(),
            actual: data.len(),
        });
    }
    Ok(WorkingImage::from_interleaved(
        image.width(),
        image.height(),
        &data,
    )?)
}

/// Returns `true` if each element of `m` is within
/// [`IDENTITY_MATRIX_TOLERANCE`] of the identity.
fn is_near_identity(m: &Mat3) -> bool {
    m.rows.iter().enumerate().all(|(r, row)| {
        row.iter().enumerate().all(|(c, v)| {
            let expected = if r == c { 1.0 } else { 0.0 };
            (v - expected).abs() <= IDENTITY_MATRIX_TOLERANCE
        })
    })
}
