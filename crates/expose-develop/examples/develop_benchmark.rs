//! Times the fused develop kernel on a 24-megapixel image and on a preview
//! with a 2560-pixel long edge (warm runs, all stages active).
//!
//! Run with: `cargo run -p expose-develop --release --example develop_benchmark`

use std::time::{Duration, Instant};

use expose_core::buffer::WorkingImage;
use expose_core::develop::{
    BasicTone, CurvePoint, DevelopSettings, ToneCurve, WhiteBalance, WhiteBalanceMode,
};
use expose_develop::{DevelopError, Pipeline, PreviewRenderer, SourceInfo, downsample_area};
use expose_gpu::{GpuContext, GpuContextOptions, GpuError, GpuImage, ImageFormat};

const WIDTH: u32 = 6000;
const HEIGHT: u32 = 4000;
const PREVIEW_EDGE: u32 = 2560;
const RUNS: u32 = 5;

fn main() {
    if let Err(error) = pollster::block_on(run()) {
        eprintln!("benchmark failed: {error}");
        std::process::exit(1);
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn settings() -> DevelopSettings {
    DevelopSettings {
        white_balance: WhiteBalance {
            mode: WhiteBalanceMode::Custom,
            temperature: 4500.0,
            tint: 10.0,
        },
        tone: BasicTone {
            exposure: 0.5,
            contrast: 30.0,
            highlights: -60.0,
            shadows: 40.0,
            whites: 10.0,
            blacks: -10.0,
        },
        tone_curve: ToneCurve {
            master: vec![
                CurvePoint::new(0.0, 0.0),
                CurvePoint::new(0.3, 0.25),
                CurvePoint::new(0.7, 0.78),
                CurvePoint::new(1.0, 1.0),
            ],
            ..ToneCurve::default()
        },
        ..DevelopSettings::default()
    }
}

async fn run() -> Result<(), DevelopError> {
    let ctx = match GpuContext::new(GpuContextOptions::default()).await {
        Ok(ctx) => ctx,
        Err(GpuError::NoAdapter(reason)) => {
            println!("SKIPPED: no GPU adapter ({reason})");
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    let info = ctx.adapter_summary();
    println!("adapter: {} ({})", info.name, info.backend);

    let samples: Vec<f32> = (0..WIDTH as usize * HEIGHT as usize * 4)
        .map(|i| f32::from(u16::try_from(i % 4099).unwrap_or(0)) / 2048.0)
        .collect();
    let image = WorkingImage::from_interleaved(WIDTH, HEIGHT, &samples)?;

    let t = Instant::now();
    let pipeline = Pipeline::new(&settings(), &SourceInfo::RENDERED)?;
    println!("pipeline build (LUTs): {:.3} ms", ms(t.elapsed()));

    // Full size: kernel only (data stays on the GPU).
    let input = GpuImage::from_rgba_f32(
        &ctx,
        WIDTH,
        HEIGHT,
        ImageFormat::Rgba32Float,
        image.as_slice(),
    )
    .await?;
    let output = GpuImage::new(&ctx, WIDTH, HEIGHT, ImageFormat::Rgba32Float).await?;
    println!("\n{WIDTH}x{HEIGHT} rgba32float");
    for run in 0..RUNS {
        let t0 = Instant::now();
        pipeline.render_gpu(&ctx, &input, &output).await?;
        ctx.wait_idle()?;
        let t1 = Instant::now();
        let back = output.read_rgba_f32(&ctx).await?;
        let t2 = Instant::now();
        std::hint::black_box(back);
        println!(
            "run {run}: develop {:6.2} ms | read-back {:7.1} ms",
            ms(t1 - t0),
            ms(t2 - t1)
        );
    }

    // Preview.
    let t0 = Instant::now();
    let small = downsample_area(&image, PREVIEW_EDGE)?;
    let t1 = Instant::now();
    println!(
        "\npreview {}x{}: CPU area downsample (once per photo) {:.1} ms",
        small.width(),
        small.height(),
        ms(t1 - t0)
    );
    let renderer = PreviewRenderer::new(&ctx, &image, PREVIEW_EDGE).await?;
    for run in 0..RUNS {
        let t0 = Instant::now();
        renderer.develop(&ctx, &pipeline).await?;
        ctx.wait_idle()?;
        let t1 = Instant::now();
        let back = renderer.output().read_rgba_f32(&ctx).await?;
        let t2 = Instant::now();
        std::hint::black_box(back);
        println!(
            "run {run}: develop {:6.2} ms | read-back {:7.1} ms",
            ms(t1 - t0),
            ms(t2 - t1)
        );
    }

    // CPU reference on the preview, for comparison.
    let t0 = Instant::now();
    let cpu = pipeline.render_cpu(&small);
    println!(
        "\nCPU reference on the preview (one thread): {:.1} ms",
        ms(t0.elapsed())
    );
    std::hint::black_box(cpu);
    Ok(())
}
