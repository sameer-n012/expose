//! The develop pipeline: settings to stages, and the CPU reference.

use expose_core::buffer::{Rgba, WorkingImage};
use expose_core::color::{Chromaticity, Mat3, TransferFunction, luminance, mul_vec_f32};
use expose_core::develop::DevelopSettings;
use expose_gpu::{GpuContext, GpuImage, ImageFormat};

use crate::curve::CurveTables;
use crate::error::{DevelopError, DevelopResult};
use crate::gpu::{DEVELOP_KERNEL, GpuParams};
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
}

/// A compiled develop pipeline for one set of settings and one source.
///
/// [`Pipeline::new`] validates the settings and precomputes all per-stage
/// data once. Then each pixel needs only a 3x3 matrix, one LUT read, and
/// (with a tone curve) two curve reads per channel.
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
///
/// # Not in this version
///
/// Crop, straighten, flip, and sharpening settings are kept in
/// [`Pipeline::settings`], but this pipeline does not apply them. They need
/// geometry and neighborhood stages (later items).
#[derive(Debug, Clone)]
pub struct Pipeline {
    settings: DevelopSettings,
    stages: Vec<Stage>,
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

        let matrix = matrix.map(|m| m.to_f32());
        let gpu_params = GpuParams::build(matrix.as_ref(), tone_lut.as_deref(), curves.as_ref());
        Ok(Self {
            settings,
            stages,
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
    pub fn stages(&self) -> &[Stage] {
        &self.stages
    }

    /// Returns `true` when no stage is active (the output equals the input).
    pub fn is_identity(&self) -> bool {
        self.stages.is_empty()
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

    /// Applies all stages to one pixel. This is the CPU reference: the GPU
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

    /// Applies all stages to `image` in place, on the CPU.
    ///
    /// The result is deterministic: the same input and settings give the
    /// same bits. The loop is single-threaded, so that it also runs in a
    /// WebAssembly build without threads.
    pub fn apply_cpu(&self, image: &mut WorkingImage) {
        if self.is_identity() {
            return;
        }
        for pixel in image.pixels_mut() {
            *pixel = self.apply_pixel(*pixel);
        }
    }

    /// Returns a developed copy of `image`, made on the CPU.
    pub fn render_cpu(&self, image: &WorkingImage) -> WorkingImage {
        let mut out = image.clone();
        self.apply_cpu(&mut out);
        out
    }

    /// Runs the develop kernel from `input` to `output` on the GPU.
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

    /// Uploads `image`, develops it on the GPU (`rgba32float`), and reads
    /// the result back.
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
