//! Display encode on the GPU: linear Rec. 2020 to 8-bit display RGBA.
//!
//! A developed preview stays on the GPU as linear Rec. 2020 RGBA `f32`.
//! A read-back of this data is 16 bytes per pixel (about 70 MB for a
//! 2560 x 1707 preview). The display encode kernel converts the preview to
//! 8-bit RGBA in a display color space on the GPU first. The read-back is
//! then 4 bytes per pixel, and the bytes go to the screen with no more
//! conversion.
//!
//! The kernel does the same steps as `WorkingImage::to_image::<u8>` with
//! `ChannelLayout::Rgba`:
//!
//! 1. Convert linear Rec. 2020 to linear display RGB with a matrix.
//! 2. Clip each channel to `0.0..=1.0`.
//! 3. Encode the sRGB curve (sRGB and Display P3 both use it).
//! 4. Scale to `0..=255` and round to the nearest code.
//!
//! Alpha is clipped and quantized linearly. The output has straight
//! (not premultiplied) alpha, as `ImageData` in a browser canvas expects.
//!
//! The result is within one code of the CPU result. The GPU `pow` function
//! is not bit-exact, and the GPU can fuse multiplies and adds.

use bytemuck::{Pod, Zeroable};
use expose_core::color::ColorSpaceId;
use expose_gpu::{GpuContext, GpuImage, ImageFormat, Kernel};

use crate::error::DevelopResult;

/// The display color space of an encoded frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DisplaySpace {
    /// sRGB (IEC 61966-2-1). The default for a browser canvas.
    #[default]
    Srgb,
    /// Display P3 (P3 primaries, D65, sRGB curve). For wide-gamut displays
    /// and canvases created with `colorSpace: "display-p3"`.
    DisplayP3,
}

impl DisplaySpace {
    /// The matching built-in color space.
    pub const fn color_space(self) -> ColorSpaceId {
        match self {
            Self::Srgb => ColorSpaceId::Srgb,
            Self::DisplayP3 => ColorSpaceId::DisplayP3,
        }
    }
}

/// The display encode kernel. Parameters: [`DisplayParams`]. The output
/// image must be [`ImageFormat::Rgba8Unorm`] for exact codes.
pub const DISPLAY_ENCODE_KERNEL: Kernel = Kernel {
    name: "expose-develop/display-encode-v1",
    source: DISPLAY_ENCODE_WGSL,
};

/// The WGSL source of [`DISPLAY_ENCODE_KERNEL`].
///
/// The kernel writes `code / 255` for each 8-bit code. The GPU converts
/// this value back to `code` exactly when it stores to `rgba8unorm`.
const DISPLAY_ENCODE_WGSL: &str = r"
// Display encode: linear Rec. 2020 -> clip -> sRGB curve -> 8-bit code.
// Kernel contract: see `expose-gpu/src/kernel.rs`.

struct Params {
    // Rows of the linear Rec. 2020 to linear display RGB matrix (w unused).
    row0: vec4<f32>,
    row1: vec4<f32>,
    row2: vec4<f32>,
}

@group(0) @binding(0) var input_image: texture_2d<f32>;
@group(0) @binding(1) var output_image: texture_storage_2d<OUTPUT_FORMAT, write>;
@group(0) @binding(2) var<uniform> params: Params;

// Clip to 0..=1. NaN gives 0 where the GPU keeps NaN in comparisons.
fn clip_unit(v: f32) -> f32 {
    return select(0.0, min(v, 1.0), v > 0.0);
}

// The sRGB curve for v in 0..=1.
fn encode_srgb(v: f32) -> f32 {
    if (v <= 0.0031308) {
        return v * 12.92;
    }
    return 1.055 * pow(v, 1.0 / 2.4) - 0.055;
}

// Unit value -> code / 255, rounded half away from zero (as `f32::round`).
fn quantize(unit: f32) -> f32 {
    return floor(unit * 255.0 + 0.5) / 255.0;
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_image);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let pixel = vec2<i32>(id.xy);
    let src = textureLoad(input_image, pixel, 0);
    let rgb = vec3<f32>(
        dot(params.row0.xyz, src.rgb),
        dot(params.row1.xyz, src.rgb),
        dot(params.row2.xyz, src.rgb),
    );
    let out = vec4<f32>(
        quantize(encode_srgb(clip_unit(rgb.r))),
        quantize(encode_srgb(clip_unit(rgb.g))),
        quantize(encode_srgb(clip_unit(rgb.b))),
        quantize(clip_unit(src.a)),
    );
    textureStore(output_image, pixel, out);
}
";

/// The parameter block of [`DISPLAY_ENCODE_KERNEL`]. The layout matches the
/// WGSL `Params` struct (three `vec4` rows; no implicit padding).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct DisplayParams {
    rows: [[f32; 4]; 3],
}

impl DisplayParams {
    /// Builds the parameters for `space`.
    ///
    /// # Errors
    ///
    /// [`crate::DevelopError::Core`] when the color conversion matrix
    /// cannot be computed (not possible for the built-in spaces).
    pub fn new(space: DisplaySpace) -> DevelopResult<Self> {
        let working = ColorSpaceId::LinearRec2020.definition();
        let matrix = working
            .conversion_to(&space.color_space().definition())?
            .to_f32();
        Ok(Self {
            rows: matrix.map(|[a, b, c]| [a, b, c, 0.0]),
        })
    }
}

/// Encodes developed previews to 8-bit display RGBA on the GPU.
///
/// The encoder owns one `Rgba8Unorm` output image of a fixed size. Create
/// it once for each preview size and use it for each frame.
#[derive(Debug)]
pub struct DisplayEncoder {
    output: GpuImage,
    params: DisplayParams,
    space: DisplaySpace,
}

impl DisplayEncoder {
    /// Creates an encoder for `width` x `height` frames in `space`.
    ///
    /// # Errors
    ///
    /// GPU errors from the image creation (for example, a zero size).
    pub async fn new(
        ctx: &GpuContext,
        width: u32,
        height: u32,
        space: DisplaySpace,
    ) -> DevelopResult<Self> {
        let output = GpuImage::new(ctx, width, height, ImageFormat::Rgba8Unorm).await?;
        Ok(Self {
            output,
            params: DisplayParams::new(space)?,
            space,
        })
    }

    /// The display color space.
    pub fn space(&self) -> DisplaySpace {
        self.space
    }

    /// The frame width and height.
    pub fn size(&self) -> (u32, u32) {
        (self.output.width(), self.output.height())
    }

    /// Encodes `input` (linear Rec. 2020 RGBA, straight alpha) into the
    /// output image. The result stays on the GPU in [`Self::output`].
    ///
    /// # Errors
    ///
    /// [`expose_gpu::GpuError::SizeMismatch`] (as a develop error) when
    /// `input` does not have the encoder size, and other kernel errors.
    pub async fn encode(&self, ctx: &GpuContext, input: &GpuImage) -> DevelopResult<()> {
        ctx.run_kernel(&DISPLAY_ENCODE_KERNEL, &self.params, input, &self.output)
            .await?;
        Ok(())
    }

    /// The encoded frame on the GPU (valid after [`Self::encode`]).
    pub fn output(&self) -> &GpuImage {
        &self.output
    }

    /// Reads the encoded frame back: RGBA8, row-major, no row padding,
    /// `width * height * 4` bytes.
    ///
    /// # Errors
    ///
    /// GPU read-back errors.
    pub async fn read(&self, ctx: &GpuContext) -> DevelopResult<Vec<u8>> {
        Ok(self.output.read_rgba8(ctx).await?)
    }

    /// Reads the encoded frame into `dst` (exactly `width * height * 4`
    /// bytes), for example after a header in one transport buffer.
    ///
    /// # Errors
    ///
    /// [`expose_gpu::GpuError::DataLength`] (as a develop error) for a
    /// wrong `dst` length, and GPU read-back errors.
    pub async fn read_into(&self, ctx: &GpuContext, dst: &mut [u8]) -> DevelopResult<()> {
        Ok(self.output.read_rgba8_into(ctx, dst).await?)
    }

    /// Runs [`Self::encode`] and then [`Self::read`].
    ///
    /// # Errors
    ///
    /// The errors of [`Self::encode`] and [`Self::read`].
    pub async fn encode_and_read(
        &self,
        ctx: &GpuContext,
        input: &GpuImage,
    ) -> DevelopResult<Vec<u8>> {
        self.encode(ctx, input).await?;
        self.read(ctx).await
    }
}

#[cfg(test)]
mod tests {
    // Test code: a failed setup is a test failure.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::cast_precision_loss)]

    use expose_core::buffer::{ChannelLayout, WorkingImage};
    use expose_gpu::{GpuContextOptions, GpuError};

    use super::*;

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

    /// Pixels that cover the full code range, out-of-gamut colors, values
    /// above white, and negative values.
    fn test_image(width: u32, height: u32) -> WorkingImage {
        let count = width as usize * height as usize;
        let pixels = (0..count)
            .map(|i| {
                let t = i as f32 / count as f32;
                match i % 5 {
                    0 => [t, t, t, 1.0],
                    1 => [t * t, 1.0 - t, t * 0.5, t],
                    2 => [0.0, 1.0, 0.0, 1.0],
                    3 => [4.0 * t, -0.2, 2.0, 1.5],
                    _ => [t.powi(4), (t * 7.0).fract(), (t * 13.0).fract(), 1.0],
                }
            })
            .collect();
        WorkingImage::from_pixels(width, height, pixels).unwrap()
    }

    #[test]
    fn param_rows_keep_white() {
        let srgb = DisplayParams::new(DisplaySpace::Srgb).unwrap();
        let p3 = DisplayParams::new(DisplaySpace::DisplayP3).unwrap();
        assert_ne!(srgb, p3);
        // Rows of a white-preserving matrix sum to 1.
        for row in srgb.rows.iter().chain(&p3.rows) {
            assert!((row[0] + row[1] + row[2] - 1.0).abs() < 1e-4, "{row:?}");
            assert!(row[3].abs() < f32::EPSILON);
        }
    }

    #[test]
    fn gpu_encode_matches_cpu_to_image_within_one_code() {
        let Some(ctx) = context() else { return };
        // Odd size: partial workgroups and padded read-back rows.
        let (w, h) = (101, 67);
        let image = test_image(w, h);
        for space in [DisplaySpace::Srgb, DisplaySpace::DisplayP3] {
            let gpu = pollster::block_on(async {
                let input =
                    GpuImage::from_rgba_f32(&ctx, w, h, ImageFormat::Rgba32Float, image.as_slice())
                        .await?;
                let encoder = DisplayEncoder::new(&ctx, w, h, space).await?;
                assert_eq!(encoder.size(), (w, h));
                assert_eq!(encoder.space(), space);
                encoder.encode_and_read(&ctx, &input).await
            })
            .unwrap();
            let cpu = image
                .to_image::<u8>(space.color_space(), ChannelLayout::Rgba)
                .unwrap();
            assert_eq!(gpu.len(), cpu.as_slice().len());
            let mut exact = 0_usize;
            for (i, (g, c)) in gpu.iter().zip(cpu.as_slice()).enumerate() {
                assert!(g.abs_diff(*c) <= 1, "{space:?} sample {i}: {g} vs {c}");
                exact += usize::from(g == c);
            }
            // Nearly all samples must be exact; a difference is rare.
            assert!(exact * 100 >= gpu.len() * 99, "{space:?}: {exact} exact");
        }
    }

    #[test]
    fn encode_rejects_wrong_input_size() {
        let Some(ctx) = context() else { return };
        let result = pollster::block_on(async {
            let input = GpuImage::new(&ctx, 8, 8, ImageFormat::Rgba32Float).await?;
            let encoder = DisplayEncoder::new(&ctx, 9, 8, DisplaySpace::Srgb).await?;
            encoder.encode(&ctx, &input).await
        });
        assert!(
            matches!(
                result,
                Err(crate::DevelopError::Gpu(GpuError::SizeMismatch { .. }))
            ),
            "{result:?}"
        );
    }
}
