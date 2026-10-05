//! GPU tests. Each test skips (prints a message and passes) when the machine
//! has no GPU adapter, so that CI without a GPU still passes.

// Test helpers panic on failure, and test indices are small (and clamped to
// be non-negative before a cast to `usize`).
#![allow(
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use expose_gpu::{GpuContext, GpuContextOptions, GpuError, GpuImage, ImageFormat, Kernel, kernels};

/// An odd size: rows are not 256-byte aligned, and the dispatch has partial
/// workgroups at the right and bottom edges.
const WIDTH: u32 = 37;
const HEIGHT: u32 = 23;

/// Creates a context, or returns `None` when no adapter is available.
fn context() -> Option<GpuContext> {
    match pollster::block_on(GpuContext::new(GpuContextOptions::default())) {
        Ok(ctx) => {
            let info = ctx.adapter_summary();
            eprintln!("GPU test adapter: {} ({})", info.name, info.backend);
            Some(ctx)
        }
        Err(GpuError::NoAdapter(reason)) => {
            eprintln!("SKIPPED: no GPU adapter ({reason})");
            None
        }
        Err(error) => panic!("GPU context creation failed: {error}"),
    }
}

/// A test image with unique values, negative values, and values above 1.0.
fn test_pixels(width: u32, height: u32) -> Vec<f32> {
    let count = width as usize * height as usize * 4;
    (0..count)
        .map(|i| {
            let x = i as f32;
            match i % 7 {
                0 => -0.25 - x * 1e-3,
                1 => 3.5 + x * 1e-2,
                2 => 1e4 + x,
                _ => x / count as f32,
            }
        })
        .collect()
}

/// Asserts that each sample is within `tolerance` (relative, for values
/// above 1.0 in magnitude; absolute below).
fn assert_close(actual: &[f32], expected: &[f32], tolerance: f32) {
    assert_eq!(actual.len(), expected.len());
    for (index, (a, e)) in actual.iter().zip(expected).enumerate() {
        let limit = tolerance * e.abs().max(1.0);
        assert!(
            (a - e).abs() <= limit,
            "sample {index}: got {a}, expected {e} (tolerance {limit})"
        );
    }
}

fn run(
    ctx: &GpuContext,
    kernel: &Kernel,
    params: &impl bytemuck::Pod,
    pixels: &[f32],
    format: ImageFormat,
) -> Vec<f32> {
    pollster::block_on(async {
        let input = GpuImage::from_rgba_f32(ctx, WIDTH, HEIGHT, format, pixels).await?;
        let output = GpuImage::new(ctx, WIDTH, HEIGHT, format).await?;
        ctx.run_kernel(kernel, params, &input, &output).await?;
        output.read_rgba_f32(ctx).await
    })
    .expect("kernel run failed")
}

#[test]
fn context_reports_adapter_and_limits() {
    let Some(ctx) = context() else { return };
    assert!(!ctx.adapter_summary().name.is_empty());
    assert!(ctx.limits().max_texture_dimension_2d >= 8192);
}

#[test]
fn upload_and_read_back_round_trip() {
    let Some(ctx) = context() else { return };
    let pixels = test_pixels(WIDTH, HEIGHT);
    let image = pollster::block_on(GpuImage::from_rgba_f32(
        &ctx,
        WIDTH,
        HEIGHT,
        ImageFormat::Rgba32Float,
        &pixels,
    ))
    .expect("upload failed");
    let back = pollster::block_on(image.read_rgba_f32(&ctx)).expect("read-back failed");
    // A copy without a kernel must be bit-exact.
    assert_eq!(back, pixels);
}

#[test]
fn copy_kernel_is_identity_and_keeps_out_of_range_values() {
    let Some(ctx) = context() else { return };
    let pixels = test_pixels(WIDTH, HEIGHT);
    let result = run(&ctx, &kernels::COPY, &(), &pixels, ImageFormat::Rgba32Float);
    assert_close(&result, &pixels, 1e-6);
    assert!(result.iter().any(|v| *v < 0.0), "negative values lost");
    assert!(result.iter().any(|v| *v > 1.0), "values above 1.0 lost");
}

#[test]
fn exposure_kernel_matches_cpu_reference() {
    let Some(ctx) = context() else { return };
    let pixels = test_pixels(WIDTH, HEIGHT);
    for ev in [-2.0_f32, -0.5, 0.0, 1.0, 1.7, 3.0] {
        let params = kernels::ExposureParams::from_ev(ev);
        let expected: Vec<f32> = pixels
            .chunks_exact(4)
            .flat_map(|p| {
                [
                    p[0] * params.gain,
                    p[1] * params.gain,
                    p[2] * params.gain,
                    p[3],
                ]
            })
            .collect();
        let result = run(
            &ctx,
            &kernels::EXPOSURE,
            &params,
            &pixels,
            ImageFormat::Rgba32Float,
        );
        assert_close(&result, &expected, 1e-6);
    }
}

#[test]
fn exposure_kernel_on_half_float_images() {
    let Some(ctx) = context() else { return };
    // Values that `f16` holds exactly, so the result is exact.
    let pixels: Vec<f32> = (0..WIDTH * HEIGHT * 4)
        .map(|i| match i % 4 {
            0 => -0.5,
            1 => 2.25,
            2 => 0.125,
            _ => 1.0,
        })
        .collect();
    let params = kernels::ExposureParams::from_ev(1.0);
    let expected: Vec<f32> = pixels
        .chunks_exact(4)
        .flat_map(|p| [p[0] * 2.0, p[1] * 2.0, p[2] * 2.0, p[3]])
        .collect();
    let result = run(
        &ctx,
        &kernels::EXPOSURE,
        &params,
        &pixels,
        ImageFormat::Rgba16Float,
    );
    assert_eq!(result, expected);
}

#[test]
fn half_float_round_trip_is_within_f16_precision() {
    let Some(ctx) = context() else { return };
    let pixels = test_pixels(WIDTH, HEIGHT);
    let result = run(&ctx, &kernels::COPY, &(), &pixels, ImageFormat::Rgba16Float);
    // f16 has a 10-bit mantissa: relative error at most 2^-11.
    assert_close(&result, &pixels, 1.0 / 2048.0);
}

#[test]
fn pipelines_are_compiled_once() {
    let Some(ctx) = context() else { return };
    let pixels = test_pixels(WIDTH, HEIGHT);
    let params = kernels::ExposureParams::from_ev(0.5);
    run(
        &ctx,
        &kernels::EXPOSURE,
        &params,
        &pixels,
        ImageFormat::Rgba32Float,
    );
    run(
        &ctx,
        &kernels::EXPOSURE,
        &params,
        &pixels,
        ImageFormat::Rgba32Float,
    );
    assert_eq!(ctx.cached_pipeline_count(), 1);
    run(
        &ctx,
        &kernels::EXPOSURE,
        &params,
        &pixels,
        ImageFormat::Rgba16Float,
    );
    assert_eq!(ctx.cached_pipeline_count(), 2);
}

#[test]
fn invalid_inputs_return_errors() {
    let Some(ctx) = context() else { return };
    pollster::block_on(async {
        let short = vec![0.0; 10];
        let error = GpuImage::from_rgba_f32(&ctx, WIDTH, HEIGHT, ImageFormat::Rgba32Float, &short)
            .await
            .expect_err("short data must fail");
        assert!(matches!(error, GpuError::DataLength { .. }), "{error}");

        let error = GpuImage::new(&ctx, 0, HEIGHT, ImageFormat::Rgba32Float)
            .await
            .expect_err("zero width must fail");
        assert!(
            matches!(error, GpuError::InvalidDimensions { .. }),
            "{error}"
        );

        let too_wide = ctx.limits().max_texture_dimension_2d + 1;
        let error = GpuImage::new(&ctx, too_wide, 1, ImageFormat::Rgba32Float)
            .await
            .expect_err("too wide must fail");
        assert!(
            matches!(error, GpuError::InvalidDimensions { .. }),
            "{error}"
        );

        let a = GpuImage::new(&ctx, WIDTH, HEIGHT, ImageFormat::Rgba32Float)
            .await
            .expect("image");
        let b = GpuImage::new(&ctx, WIDTH + 1, HEIGHT, ImageFormat::Rgba32Float)
            .await
            .expect("image");
        let error = ctx
            .run_kernel(&kernels::COPY, &(), &a, &b)
            .await
            .expect_err("size mismatch must fail");
        assert!(matches!(error, GpuError::SizeMismatch { .. }), "{error}");

        let error = ctx
            .run_kernel(&kernels::COPY, &(), &a, &a)
            .await
            .expect_err("same image must fail");
        assert!(matches!(error, GpuError::SameImage), "{error}");

        let broken = Kernel {
            name: "broken",
            source: "this is not WGSL OUTPUT_FORMAT",
        };
        let c = GpuImage::new(&ctx, WIDTH, HEIGHT, ImageFormat::Rgba32Float)
            .await
            .expect("image");
        let error = ctx
            .run_kernel(&broken, &(), &a, &c)
            .await
            .expect_err("bad WGSL must fail");
        assert!(matches!(error, GpuError::Validation(_)), "{error}");

        let no_token = Kernel {
            name: "no-token",
            source: "@compute @workgroup_size(8, 8, 1) fn main() {}",
        };
        let error = ctx
            .run_kernel(&no_token, &(), &a, &c)
            .await
            .expect_err("missing token must fail");
        assert!(matches!(error, GpuError::InvalidKernel { .. }), "{error}");
    });
}

#[test]
fn rgba8_bytes_round_trip_exactly() {
    let Some(ctx) = context() else { return };
    // Odd width: 37 * 4 = 148 bytes per row, padded to 256 on read-back.
    let bytes: Vec<u8> = (0..WIDTH as usize * HEIGHT as usize * 4)
        .map(|i| (i * 7 % 256) as u8)
        .collect();
    let back = pollster::block_on(async {
        let image = GpuImage::new(&ctx, WIDTH, HEIGHT, ImageFormat::Rgba8Unorm).await?;
        image.write_rgba8(&ctx, &bytes).await?;
        image.read_rgba8(&ctx).await
    })
    .expect("round trip failed");
    assert_eq!(back, bytes);
}

#[test]
fn kernel_writes_exact_codes_to_rgba8() {
    let Some(ctx) = context() else { return };
    // Write `code / 255` for each code; the copy kernel must keep each code.
    // Values outside 0..=1 must clip.
    let mut pixels: Vec<f32> = (0..WIDTH as usize * HEIGHT as usize * 4)
        .map(|i| (i % 256) as f32 / 255.0)
        .collect();
    pixels[0] = -0.5;
    pixels[1] = 7.0;
    let (codes, floats) = pollster::block_on(async {
        let input =
            GpuImage::from_rgba_f32(&ctx, WIDTH, HEIGHT, ImageFormat::Rgba32Float, &pixels).await?;
        let output = GpuImage::new(&ctx, WIDTH, HEIGHT, ImageFormat::Rgba8Unorm).await?;
        ctx.run_kernel(&kernels::COPY, &(), &input, &output).await?;
        Ok::<_, GpuError>((
            output.read_rgba8(&ctx).await?,
            output.read_rgba_f32(&ctx).await?,
        ))
    })
    .expect("kernel run failed");
    assert_eq!(codes[0], 0);
    assert_eq!(codes[1], 255);
    for (i, &code) in codes.iter().enumerate().skip(2) {
        assert_eq!(usize::from(code), i % 256, "sample {i}");
        assert!((floats[i] - f32::from(code) / 255.0).abs() < 1e-7);
    }
}

#[test]
fn rgba8_f32_upload_rounds_and_clips() {
    let Some(ctx) = context() else { return };
    let mut pixels = vec![0.5_f32; 4 * 4];
    pixels[0] = -1.0;
    pixels[1] = 2.0;
    pixels[2] = f32::NAN;
    pixels[3] = 0.1;
    let back = pollster::block_on(async {
        let image = GpuImage::from_rgba_f32(&ctx, 4, 1, ImageFormat::Rgba8Unorm, &pixels).await?;
        image.read_rgba8(&ctx).await
    })
    .expect("round trip failed");
    assert_eq!(&back[..5], &[0, 255, 0, 26, 128]);
}

#[test]
fn rgba8_access_on_float_image_fails() {
    let Some(ctx) = context() else { return };
    pollster::block_on(async {
        let image = GpuImage::new(&ctx, 4, 4, ImageFormat::Rgba32Float)
            .await
            .expect("image");
        let error = image
            .read_rgba8(&ctx)
            .await
            .expect_err("float image must fail");
        assert!(matches!(error, GpuError::FormatMismatch { .. }), "{error}");
        let error = image
            .write_rgba8(&ctx, &[0; 64])
            .await
            .expect_err("float image must fail");
        assert!(matches!(error, GpuError::FormatMismatch { .. }), "{error}");

        let bytes = GpuImage::new(&ctx, 4, 4, ImageFormat::Rgba8Unorm)
            .await
            .expect("image");
        let mut short = [0_u8; 63];
        let error = bytes
            .read_rgba8_into(&ctx, &mut short)
            .await
            .expect_err("short buffer must fail");
        assert!(matches!(error, GpuError::DataLength { .. }), "{error}");
        let mut exact = [1_u8; 64];
        bytes
            .read_rgba8_into(&ctx, &mut exact)
            .await
            .expect("exact buffer");
        assert_eq!(exact, [0_u8; 64]);
    });
}

#[test]
fn tall_image_uses_several_read_back_strips() {
    let Some(ctx) = context() else { return };
    // 1500 x 3000 x 16 bytes = 72 MB > one 64 MiB strip.
    let (width, height) = (1500, 3000);
    let pixels = test_pixels(width, height);
    let back = pollster::block_on(async {
        let image =
            GpuImage::from_rgba_f32(&ctx, width, height, ImageFormat::Rgba32Float, &pixels).await?;
        image.read_rgba_f32(&ctx).await
    })
    .expect("round trip failed");
    assert!(back == pixels, "strip round trip changed data");
}

/// Returns pixel `(x, y)` of an RGBA image, with the coordinates clamped to
/// the image (the edge rule of the built-in crop and downsample kernels).
fn clamped_pixel(pixels: &[f32], width: u32, height: u32, x: i64, y: i64) -> [f32; 4] {
    let x = x.clamp(0, i64::from(width) - 1) as usize;
    let y = y.clamp(0, i64::from(height) - 1) as usize;
    let start = (y * width as usize + x) * 4;
    [
        pixels[start],
        pixels[start + 1],
        pixels[start + 2],
        pixels[start + 3],
    ]
}

/// CPU reference of `kernels::CROP_COPY`.
fn cpu_crop(pixels: &[f32], out_width: u32, out_height: u32, offset: [i32; 2]) -> Vec<f32> {
    (0..out_height)
        .flat_map(|y| (0..out_width).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            clamped_pixel(
                pixels,
                WIDTH,
                HEIGHT,
                i64::from(x) + i64::from(offset[0]),
                i64::from(y) + i64::from(offset[1]),
            )
        })
        .collect()
}

/// Runs `kernel` with `inputs` (all `WIDTH` x `HEIGHT`, `Rgba32Float`) and
/// an `Rgba32Float` output of `out_width` x `out_height`.
fn run_multi(
    ctx: &GpuContext,
    kernel: &Kernel,
    params: &impl bytemuck::Pod,
    inputs: &[&[f32]],
    out_width: u32,
    out_height: u32,
) -> Result<Vec<f32>, GpuError> {
    pollster::block_on(async {
        let mut images = Vec::with_capacity(inputs.len());
        for pixels in inputs {
            images.push(
                GpuImage::from_rgba_f32(ctx, WIDTH, HEIGHT, ImageFormat::Rgba32Float, pixels)
                    .await?,
            );
        }
        let refs: Vec<&GpuImage> = images.iter().collect();
        let output = GpuImage::new(ctx, out_width, out_height, ImageFormat::Rgba32Float).await?;
        ctx.run_kernel_multi(kernel, params, &refs, &output).await?;
        output.read_rgba_f32(ctx).await
    })
}

#[test]
fn crop_kernel_writes_smaller_output_like_cpu_crop() {
    let Some(ctx) = context() else { return };
    let pixels = test_pixels(WIDTH, HEIGHT);
    // Inside the input, with an odd output size (partial workgroups).
    let params = kernels::OffsetParams::new(5, 7);
    let result =
        run_multi(&ctx, &kernels::CROP_COPY, &params, &[&pixels], 20, 11).expect("crop failed");
    assert_eq!(result, cpu_crop(&pixels, 20, 11, [5, 7]));

    // Partly outside the input: the edge pixels repeat.
    let params = kernels::OffsetParams::new(30, -3);
    let result =
        run_multi(&ctx, &kernels::CROP_COPY, &params, &[&pixels], 10, 9).expect("crop failed");
    assert_eq!(result, cpu_crop(&pixels, 10, 9, [30, -3]));
}

#[test]
fn run_kernel_rejects_crop_size_but_multi_accepts_it() {
    let Some(ctx) = context() else { return };
    pollster::block_on(async {
        let input = GpuImage::new(&ctx, WIDTH, HEIGHT, ImageFormat::Rgba32Float)
            .await
            .expect("image");
        let output = GpuImage::new(&ctx, 8, 8, ImageFormat::Rgba32Float)
            .await
            .expect("image");
        let params = kernels::OffsetParams::new(1, 1);
        let error = ctx
            .run_kernel(&kernels::CROP_COPY, &params, &input, &output)
            .await
            .expect_err("run_kernel must keep the same-size rule");
        assert!(matches!(error, GpuError::SizeMismatch { .. }), "{error}");
        ctx.run_kernel_multi(&kernels::CROP_COPY, &params, &[&input], &output)
            .await
            .expect("different sizes are valid for run_kernel_multi");
    });
}

#[test]
fn add2_kernel_sums_two_inputs() {
    let Some(ctx) = context() else { return };
    let a = test_pixels(WIDTH, HEIGHT);
    let b: Vec<f32> = a.iter().rev().map(|v| v * 0.5 - 1.0).collect();
    let expected: Vec<f32> = a.iter().zip(&b).map(|(x, y)| x + y).collect();
    let result =
        run_multi(&ctx, &kernels::ADD2, &(), &[&a, &b], WIDTH, HEIGHT).expect("add failed");
    assert_close(&result, &expected, 1e-6);
}

#[test]
fn one_image_can_be_two_inputs() {
    let Some(ctx) = context() else { return };
    let pixels = test_pixels(WIDTH, HEIGHT);
    let result = pollster::block_on(async {
        let input =
            GpuImage::from_rgba_f32(&ctx, WIDTH, HEIGHT, ImageFormat::Rgba32Float, &pixels).await?;
        let output = GpuImage::new(&ctx, WIDTH, HEIGHT, ImageFormat::Rgba32Float).await?;
        ctx.run_kernel_multi(&kernels::ADD2, &(), &[&input, &input], &output)
            .await?;
        output.read_rgba_f32(&ctx).await
    })
    .expect("add failed");
    let expected: Vec<f32> = pixels.iter().map(|v| v * 2.0).collect();
    assert_eq!(result, expected);
}

#[test]
fn box_downsample_matches_cpu_mean() {
    let Some(ctx) = context() else { return };
    let pixels = test_pixels(WIDTH, HEIGHT);
    let (out_width, out_height) = (WIDTH.div_ceil(2), HEIGHT.div_ceil(2));
    let result = run_multi(
        &ctx,
        &kernels::BOX_DOWNSAMPLE_2X,
        &(),
        &[&pixels],
        out_width,
        out_height,
    )
    .expect("downsample failed");
    let expected: Vec<f32> = (0..out_height)
        .flat_map(|y| (0..out_width).map(move |x| (i64::from(x) * 2, i64::from(y) * 2)))
        .flat_map(|(x, y)| {
            let mut sum = [0.0_f32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = clamped_pixel(&pixels, WIDTH, HEIGHT, x + dx, y + dy);
                for (s, v) in sum.iter_mut().zip(p) {
                    *s += v;
                }
            }
            sum.map(|s| s * 0.25)
        })
        .collect();
    assert_close(&result, &expected, 1e-6);
}

#[test]
fn multi_input_errors() {
    let Some(ctx) = context() else { return };
    pollster::block_on(async {
        let a = GpuImage::new(&ctx, WIDTH, HEIGHT, ImageFormat::Rgba32Float)
            .await
            .expect("image");
        let out = GpuImage::new(&ctx, WIDTH, HEIGHT, ImageFormat::Rgba32Float)
            .await
            .expect("image");

        let error = ctx
            .run_kernel_multi(&kernels::ADD2, &(), &[&a, &out], &out)
            .await
            .expect_err("output as second input must fail");
        assert!(matches!(error, GpuError::SameImage), "{error}");

        let error = ctx
            .run_kernel_multi(&kernels::COPY, &(), &[], &out)
            .await
            .expect_err("zero inputs must fail");
        assert!(
            matches!(error, GpuError::InputCount { count: 0, max: 4 }),
            "{error}"
        );

        let error = ctx
            .run_kernel_multi(&kernels::COPY, &(), &[&a; 5], &out)
            .await
            .expect_err("five inputs must fail");
        assert!(
            matches!(error, GpuError::InputCount { count: 5, max: 4 }),
            "{error}"
        );
        assert_eq!(ctx.cached_pipeline_count(), 0);

        // ADD2 declares binding 3, but a one-input layout does not have it.
        let error = ctx
            .run_kernel_multi(&kernels::ADD2, &(), &[&a], &out)
            .await
            .expect_err("too few inputs for the shader must fail");
        assert!(matches!(error, GpuError::Validation(_)), "{error}");
        assert_eq!(
            ctx.cached_pipeline_count(),
            0,
            "a failed pipeline is cached"
        );

        // The context stays usable after the validation error.
        ctx.run_kernel_multi(&kernels::ADD2, &(), &[&a, &a], &out)
            .await
            .expect("valid run after an error");
    });
}

#[test]
fn pipeline_cache_is_keyed_by_input_count() {
    let Some(ctx) = context() else { return };
    let pixels = test_pixels(WIDTH, HEIGHT);
    let run_count = |kernel: &Kernel, count: usize, format: ImageFormat| {
        pollster::block_on(async {
            let input = GpuImage::from_rgba_f32(&ctx, WIDTH, HEIGHT, format, &pixels).await?;
            let output = GpuImage::new(&ctx, WIDTH, HEIGHT, format).await?;
            let inputs = vec![&input; count];
            ctx.run_kernel_multi(kernel, &(), &inputs, &output).await
        })
        .expect("kernel run failed");
    };

    run(&ctx, &kernels::COPY, &(), &pixels, ImageFormat::Rgba32Float);
    assert_eq!(ctx.cached_pipeline_count(), 1);
    // `run_kernel` and a one-input `run_kernel_multi` share one pipeline.
    run_count(&kernels::COPY, 1, ImageFormat::Rgba32Float);
    assert_eq!(ctx.cached_pipeline_count(), 1);
    // A shader can ignore extra inputs; the pipeline is a new cache entry.
    run_count(&kernels::COPY, 2, ImageFormat::Rgba32Float);
    assert_eq!(ctx.cached_pipeline_count(), 2);
    run_count(&kernels::ADD2, 2, ImageFormat::Rgba32Float);
    assert_eq!(ctx.cached_pipeline_count(), 3);
    run_count(&kernels::ADD2, 2, ImageFormat::Rgba32Float);
    assert_eq!(ctx.cached_pipeline_count(), 3);
    run_count(&kernels::ADD2, 4, ImageFormat::Rgba32Float);
    assert_eq!(ctx.cached_pipeline_count(), 4);
    run_count(&kernels::ADD2, 2, ImageFormat::Rgba16Float);
    assert_eq!(ctx.cached_pipeline_count(), 5);
}
