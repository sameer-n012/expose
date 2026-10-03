//! GPU tests: the fused kernel agrees with the CPU reference. Each test
//! skips (prints a message and passes) when the machine has no GPU adapter.

#![allow(clippy::expect_used)]

mod common;

use common::{settings_set, test_image};
use expose_core::develop::DevelopSettings;
use expose_develop::{Pipeline, PreviewRenderer, SourceInfo, downsample_area, render_preview};
use expose_gpu::{GpuContext, GpuContextOptions, GpuError};

/// Relative tolerance (absolute below magnitude 1).
const TOLERANCE: f32 = 1e-4;

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

/// Returns the largest error relative to `max(|expected|, 1)`.
fn max_error(actual: &[f32], expected: &[f32]) -> f32 {
    assert_eq!(actual.len(), expected.len());
    actual
        .iter()
        .zip(expected)
        .map(|(a, e)| (a - e).abs() / e.abs().max(1.0))
        .fold(0.0, f32::max)
}

#[test]
fn gpu_matches_cpu_reference() {
    let Some(ctx) = context() else { return };
    let img = test_image();
    let mut all = settings_set();
    all.push(DevelopSettings::default());
    for (i, s) in all.iter().enumerate() {
        let p = Pipeline::new(s, &SourceInfo::RENDERED).expect("pipeline");
        let cpu = p.render_cpu(&img);
        let gpu = pollster::block_on(p.render_image_gpu(&ctx, &img)).expect("GPU render");
        let err = max_error(gpu.as_slice(), cpu.as_slice());
        eprintln!("settings {i}: max relative error {err:e}");
        assert!(err <= TOLERANCE, "settings {i}: error {err} > {TOLERANCE}");
    }
}

#[test]
fn preview_matches_cpu_on_downsampled_source() {
    let Some(ctx) = context() else { return };
    let img = test_image();
    let s = &settings_set()[1];
    let p = Pipeline::new(s, &SourceInfo::RENDERED).expect("pipeline");
    let preview = pollster::block_on(render_preview(&ctx, &img, &SourceInfo::RENDERED, s, 16))
        .expect("preview");
    assert_eq!((preview.width(), preview.height()), (16, 10));
    let cpu = p.render_cpu(&downsample_area(&img, 16).expect("downsample"));
    assert!(max_error(preview.as_slice(), cpu.as_slice()) <= TOLERANCE);

    // A renderer reuses its GPU images for several settings.
    let r = pollster::block_on(PreviewRenderer::new(&ctx, &img, 16)).expect("renderer");
    assert_eq!(r.size(), (16, 10));
    for s in settings_set() {
        let p = Pipeline::new(&s, &SourceInfo::RENDERED).expect("pipeline");
        let out = pollster::block_on(r.render(&ctx, &p)).expect("render");
        let cpu = p.render_cpu(&downsample_area(&img, 16).expect("downsample"));
        assert!(max_error(out.as_slice(), cpu.as_slice()) <= TOLERANCE);
    }
}
