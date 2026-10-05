//! GPU tests of the full develop: the geometry and sharpening kernels and
//! [`Pipeline::develop_gpu`] agree with the CPU reference. Each test skips
//! (prints a message and passes) when the machine has no GPU adapter.

#![allow(
    clippy::expect_used,
    clippy::float_cmp,
    clippy::cast_precision_loss,
    reason = "tests panic on failure, compare exact zeros, and use small sizes"
)]

mod common;

use common::{all_stages_settings, crop, edge_image, rect, sharpening, smooth_image, test_image};
use expose_core::buffer::WorkingImage;
use expose_core::develop::{BasicTone, DevelopSettings, NormalizedRect};
use expose_develop::{
    DevelopTargets, Geometry, Pipeline, PreviewRenderer, SourceInfo, max_inscribed_rect,
};
use expose_gpu::{GpuContext, GpuContextOptions, GpuError, GpuImage, ImageFormat};

/// Absolute tolerance of the rotated geometry (smooth image).
const GEOMETRY_TOLERANCE: f32 = 1e-5;
/// Relative tolerance (absolute below magnitude 1) of sharpening and of the
/// full develop.
const TOLERANCE: f32 = 1e-4;

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

fn pipeline(settings: &DevelopSettings) -> Pipeline {
    Pipeline::new(settings, &SourceInfo::RENDERED).expect("pipeline")
}

fn gpu_develop(ctx: &GpuContext, p: &Pipeline, img: &WorkingImage, scale: f32) -> WorkingImage {
    pollster::block_on(p.develop_image_gpu(ctx, img, scale)).expect("GPU develop")
}

fn bits(image: &WorkingImage) -> Vec<u32> {
    image.as_slice().iter().map(|v| v.to_bits()).collect()
}

fn assert_same_size(a: &WorkingImage, b: &WorkingImage) {
    assert_eq!((a.width(), a.height()), (b.width(), b.height()));
}

/// Returns the largest error relative to `max(|expected|, 1)`.
fn max_error(actual: &WorkingImage, expected: &WorkingImage) -> f32 {
    assert_same_size(actual, expected);
    actual
        .as_slice()
        .iter()
        .zip(expected.as_slice())
        .map(|(a, e)| (a - e).abs() / e.abs().max(1.0))
        .fold(0.0, f32::max)
}

#[test]
fn identity_settings_copy_the_input_exactly() {
    let Some(ctx) = context() else { return };
    let img = test_image();
    let p = pipeline(&DevelopSettings::default());
    assert_eq!(bits(&gpu_develop(&ctx, &p, &img, 1.0)), bits(&img));
}

#[test]
fn crop_and_flip_without_rotation_are_bit_exact() {
    let Some(ctx) = context() else { return };
    let img = test_image();
    for (fh, fv) in [(false, false), (true, false), (false, true), (true, true)] {
        let settings = DevelopSettings {
            crop: crop(rect(0.2, 0.1, 0.5, 0.6), 0.0, fh, fv),
            ..DevelopSettings::default()
        };
        let p = pipeline(&settings);
        let cpu = p.develop_cpu(&img, 1.0).expect("CPU develop");
        let gpu = gpu_develop(&ctx, &p, &img, 1.0);
        assert_same_size(&gpu, &cpu);
        assert_eq!(bits(&gpu), bits(&cpu), "flips {fh} {fv}");
    }
}

#[test]
fn rotated_crop_matches_cpu() {
    let Some(ctx) = context() else { return };
    let (w, h) = (45, 31);
    let img = smooth_image(w, h);
    for (angle, fh) in [(7.0, false), (-12.5, true), (30.0, false)] {
        let inscribed = max_inscribed_rect(w, h, angle, None).expect("rect");
        let settings = DevelopSettings {
            crop: crop(inscribed, angle, fh, false),
            ..DevelopSettings::default()
        };
        let p = pipeline(&settings);
        let cpu = p.develop_cpu(&img, 1.0).expect("CPU develop");
        let gpu = gpu_develop(&ctx, &p, &img, 1.0);
        assert_same_size(&gpu, &cpu);
        let err = gpu
            .as_slice()
            .iter()
            .zip(cpu.as_slice())
            .map(|(a, e)| (a - e).abs())
            .fold(0.0, f32::max);
        eprintln!("angle {angle}: max abs error {err:e}");
        assert!(err <= GEOMETRY_TOLERANCE, "angle {angle}: {err}");
    }
}

#[test]
fn points_outside_the_source_are_transparent() {
    let Some(ctx) = context() else { return };
    let (w, h) = (40, 30);
    let img = smooth_image(w, h);
    let settings = DevelopSettings {
        crop: crop(NormalizedRect::FULL, 10.0, false, false),
        ..DevelopSettings::default()
    };
    let p = pipeline(&settings);
    let g = Geometry::new(&settings.crop, w, h).expect("geometry");
    let gpu = gpu_develop(&ctx, &p, &img, 1.0);
    let cpu = p.develop_cpu(&img, 1.0).expect("CPU develop");
    let (wf, hf) = (w as f32, h as f32);
    let margin = 1e-3;
    let mut outside = 0;
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = g.source_position(x, y);
            let near_edge = sx.abs() < margin
                || (sx - wf).abs() < margin
                || sy.abs() < margin
                || (sy - hf).abs() < margin;
            if near_edge {
                continue;
            }
            let gp = gpu.pixel(x, y).expect("pixel");
            let cp = cpu.pixel(x, y).expect("pixel");
            if !(0.0..=wf).contains(&sx) || !(0.0..=hf).contains(&sy) {
                outside += 1;
                assert_eq!(gp, [0.0; 4], "({x}, {y})");
            }
            for c in 0..4 {
                assert!((gp[c] - cp[c]).abs() <= GEOMETRY_TOLERANCE, "({x}, {y})");
            }
        }
    }
    assert!(
        outside > 0,
        "the corners must be outside the rotated source"
    );
}

#[test]
fn sharpening_matches_cpu() {
    let Some(ctx) = context() else { return };
    let constant = WorkingImage::new_filled(19, 13, [0.3, 0.2, 0.1, 1.0]).expect("image");
    let edges = edge_image(37, 23);
    for (amount, masking) in [(150.0, 0.0), (80.0, 60.0), (150.0, 100.0)] {
        let settings = DevelopSettings {
            sharpening: sharpening(amount, masking),
            ..DevelopSettings::default()
        };
        let p = pipeline(&settings);
        for scale in [1.0, 2.5] {
            let out = gpu_develop(&ctx, &p, &constant, scale);
            assert!(max_error(&out, &constant) <= 1e-5, "constant image changed");
            let cpu = p.develop_cpu(&edges, scale).expect("CPU develop");
            let gpu = gpu_develop(&ctx, &p, &edges, scale);
            let err = max_error(&gpu, &cpu);
            eprintln!("amount {amount} masking {masking} scale {scale}: error {err:e}");
            assert!(err <= TOLERANCE, "error {err}");
            // Masking can protect all of this small checker (its blurred
            // gradient is low), so check the effect only without masking.
            if masking == 0.0 {
                assert!(
                    max_error(&cpu, &edges) > 1e-2,
                    "the edges must be sharpened"
                );
            }
        }
    }
}

#[test]
fn full_develop_matches_cpu() {
    let Some(ctx) = context() else { return };
    let p = pipeline(&all_stages_settings());
    for img in [smooth_image(41, 27), edge_image(36, 28)] {
        for scale in [1.0, 0.7] {
            let cpu = p.develop_cpu(&img, scale).expect("CPU develop");
            let gpu = gpu_develop(&ctx, &p, &img, scale);
            let err = max_error(&gpu, &cpu);
            eprintln!("scale {scale}: error {err:e}");
            assert!(err <= TOLERANCE, "error {err}");
        }
    }
    // Tone and sharpening without geometry; geometry switched off.
    let mut p = pipeline(&all_stages_settings());
    p.set_apply_geometry(false);
    let img = edge_image(30, 22);
    let err = max_error(
        &gpu_develop(&ctx, &p, &img, 1.0),
        &p.develop_cpu(&img, 1.0).expect("CPU develop"),
    );
    assert!(err <= TOLERANCE, "error {err}");
}

#[test]
fn targets_are_reused_at_one_size() {
    let Some(ctx) = context() else { return };
    let img = smooth_image(33, 21);
    let input = pollster::block_on(GpuImage::from_rgba_f32(
        &ctx,
        33,
        21,
        ImageFormat::Rgba32Float,
        img.as_slice(),
    ))
    .expect("upload");
    let mut targets = DevelopTargets::new();
    assert_eq!(targets.allocation_count(), 0);
    let mut settings = all_stages_settings();
    let p = pipeline(&settings);
    let size = pollster::block_on(p.develop_gpu(&ctx, &input, 1.0, &mut targets))
        .map(|out| (out.width(), out.height()))
        .expect("develop");
    assert_eq!(size, p.output_size(33, 21).expect("size"));
    // Geometry, tone, and four sharpening images.
    let first = targets.allocation_count();
    assert_eq!(first, 6);

    // Other tone settings, same size: no new image.
    settings.tone = BasicTone {
        exposure: 1.0,
        ..BasicTone::default()
    };
    let p = pipeline(&settings);
    pollster::block_on(p.develop_gpu(&ctx, &input, 1.0, &mut targets)).expect("develop");
    assert_eq!(targets.allocation_count(), first);

    // A new crop size makes new images.
    settings.crop.rect = rect(0.0, 0.0, 0.5, 0.5);
    let p = pipeline(&settings);
    pollster::block_on(p.develop_gpu(&ctx, &input, 1.0, &mut targets)).expect("develop");
    assert!(targets.allocation_count() > first);
}

#[test]
fn preview_full_develop_has_the_output_size() {
    let Some(ctx) = context() else { return };
    let img = test_image();
    let mut r = pollster::block_on(PreviewRenderer::new(&ctx, &img, 16)).expect("renderer");
    assert_eq!(r.size(), (16, 10));
    assert!((r.pixel_scale() - 16.0 / 37.0).abs() < 1e-6);
    let mut p = pipeline(&all_stages_settings());
    let expected = p.output_size(16, 10).expect("size");
    let size = pollster::block_on(r.develop_full(&ctx, &p))
        .map(|out| (out.width(), out.height()))
        .expect("develop");
    assert_eq!(size, expected);
    assert_ne!(size, r.size());
    let allocations = r.targets().allocation_count();

    p.set_apply_geometry(false);
    let size = pollster::block_on(r.develop_full(&ctx, &p))
        .map(|out| (out.width(), out.height()))
        .expect("develop");
    assert_eq!(size, r.size());
    assert!(r.targets().allocation_count() > allocations);

    r.set_original_size(160, 100).expect("original size");
    assert!((r.pixel_scale() - 0.1).abs() < 1e-6);
    assert!(r.set_original_size(0, 100).is_err());
}
