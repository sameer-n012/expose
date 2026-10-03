//! Times upload, exposure, and read-back for a 24-megapixel image.
//!
//! Run with: `cargo run -p expose-gpu --release --example exposure_benchmark`

use std::time::{Duration, Instant};

use expose_gpu::{GpuContext, GpuContextOptions, GpuError, GpuImage, ImageFormat, kernels};

const WIDTH: u32 = 6000;
const HEIGHT: u32 = 4000;
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

async fn run() -> Result<(), GpuError> {
    let start = Instant::now();
    let ctx = match GpuContext::new(GpuContextOptions::default()).await {
        Ok(ctx) => ctx,
        Err(GpuError::NoAdapter(reason)) => {
            println!("SKIPPED: no GPU adapter ({reason})");
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let info = ctx.adapter_summary();
    println!(
        "adapter: {} ({}), context created in {:.1} ms",
        info.name,
        info.backend,
        ms(start.elapsed())
    );

    // `i % 4096` always fits in `u16`, so the `unwrap_or` never applies.
    let pixels: Vec<f32> = (0..WIDTH as usize * HEIGHT as usize * 4)
        .map(|i| f32::from(u16::try_from(i % 4096).unwrap_or(0)) / 4096.0)
        .collect();
    let params = kernels::ExposureParams::from_ev(0.75);

    for format in ImageFormat::ALL {
        println!(
            "\n{format:?}, {WIDTH}x{HEIGHT} ({} MB of f32 on the CPU)",
            pixels.len() * 4 / 1_000_000
        );
        let input = GpuImage::new(&ctx, WIDTH, HEIGHT, format).await?;
        let output = GpuImage::new(&ctx, WIDTH, HEIGHT, format).await?;

        for run in 0..RUNS {
            let t0 = Instant::now();
            input.write_rgba_f32(&ctx, &pixels).await?;
            ctx.wait_idle()?;
            let t1 = Instant::now();
            ctx.run_kernel(&kernels::EXPOSURE, &params, &input, &output)
                .await?;
            ctx.wait_idle()?;
            let t2 = Instant::now();
            let result = output.read_rgba_f32(&ctx).await?;
            let t3 = Instant::now();
            println!(
                "run {run}: upload {:7.1} ms | exposure {:6.1} ms | read-back {:7.1} ms | total {:7.1} ms",
                ms(t1 - t0),
                ms(t2 - t1),
                ms(t3 - t2),
                ms(t3 - t0),
            );
            // Keep the result alive so that the read-back is not optimized out.
            std::hint::black_box(result);
        }
    }
    Ok(())
}
