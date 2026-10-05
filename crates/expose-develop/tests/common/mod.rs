//! Shared test helpers: test images and settings.

// Test helpers panic on failure. Each test file uses only some helpers.
#![allow(dead_code, clippy::cast_precision_loss, clippy::expect_used)]

use expose_core::buffer::{Rgba, WorkingImage};
use expose_core::develop::{
    BasicTone, Crop, CurvePoint, DevelopSettings, NormalizedRect, Sharpening, ToneCurve,
    WhiteBalance, WhiteBalanceMode,
};

/// An odd size: partial GPU workgroups at the right and bottom edges.
pub const WIDTH: u32 = 37;
pub const HEIGHT: u32 = 23;

/// A test image with negative values, values above 1.0, saturated colors,
/// near-black values, zero, and smooth ramps.
pub fn test_image() -> WorkingImage {
    let count = (WIDTH * HEIGHT) as usize;
    let pixels: Vec<Rgba> = (0..count)
        .map(|i| {
            let x = i as f32 / count as f32;
            match i % 9 {
                0 => [-0.05, 0.2 * x, 0.4, 1.0],
                1 => [3.5 + x, 2.0, 7.0 * x, 1.0],
                2 => [1.0, 0.0, 0.0, 1.0],
                3 => [0.0, 0.0, 0.9 * x + 0.05, 1.0],
                4 => [1e-6 * x, 2e-6, 0.0, 1.0],
                5 => [0.0, 0.0, 0.0, 0.5],
                6 => [0.18, 0.18, 0.18, 1.0],
                7 => [-0.3, -0.1, 0.05, 1.0],
                _ => [x, x * x, 1.0 - x, 1.0],
            }
        })
        .collect();
    WorkingImage::from_pixels(WIDTH, HEIGHT, pixels).expect("valid test image")
}

/// A smooth test image (no hard steps between neighbors) of the given size.
/// Rotation tests use it, because a hard step makes the result sensitive to
/// float rounding of the sample position.
pub fn smooth_image(width: u32, height: u32) -> WorkingImage {
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let (u, v) = (x as f32 / width as f32, y as f32 / height as f32);
            pixels.push([
                0.05 + 0.8 * u * u,
                0.1 + 0.5 * (6.0 * u + 3.0 * v).sin().abs(),
                0.9 - 0.7 * v,
                1.0,
            ]);
        }
    }
    WorkingImage::from_pixels(width, height, pixels).expect("valid smooth image")
}

/// A test image with sharp edges for the sharpening stage: a checker of
/// 4 x 4 blocks with a color tint.
pub fn edge_image(width: u32, height: u32) -> WorkingImage {
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let on = ((x / 4) + (y / 4)) % 2 == 0;
            let v = if on { 0.6 } else { 0.08 };
            pixels.push([v, v * 0.8, v * 0.5 + 0.01, 1.0]);
        }
    }
    WorkingImage::from_pixels(width, height, pixels).expect("valid edge image")
}

/// A crop with the given rect, angle, and flips.
pub fn crop(rect: NormalizedRect, angle: f32, flip_h: bool, flip_v: bool) -> Crop {
    Crop {
        rect,
        angle,
        flip_horizontal: flip_h,
        flip_vertical: flip_v,
        ..Crop::default()
    }
}

/// A rect in the frame (normalized).
pub fn rect(left: f32, top: f32, width: f32, height: f32) -> NormalizedRect {
    NormalizedRect {
        left,
        top,
        width,
        height,
    }
}

/// Sharpening settings with the given amount and masking.
pub fn sharpening(amount: f32, masking: f32) -> Sharpening {
    Sharpening {
        amount,
        radius: 1.2,
        detail: 40.0,
        masking,
    }
}

/// Settings that use every stage group: a rotated crop with a flip, white
/// balance, tone, a tone curve, and sharpening.
pub fn all_stages_settings() -> DevelopSettings {
    let mut s = settings_set()[1].clone();
    s.tone_curve = settings_set()[2].tone_curve.clone();
    s.crop = crop(rect(0.1, 0.15, 0.75, 0.7), 4.0, true, false);
    s.sharpening = sharpening(120.0, 20.0);
    s
}

/// A set of non-trivial settings that use every stage.
pub fn settings_set() -> Vec<DevelopSettings> {
    let curve = ToneCurve {
        master: vec![
            CurvePoint::new(0.0, 0.02),
            CurvePoint::new(0.25, 0.2),
            CurvePoint::new(0.75, 0.82),
            CurvePoint::new(1.0, 1.0),
        ],
        red: vec![
            CurvePoint::new(0.0, 0.0),
            CurvePoint::new(0.5, 0.55),
            CurvePoint::new(1.0, 1.0),
        ],
        blue: vec![CurvePoint::new(0.1, 0.0), CurvePoint::new(0.9, 1.0)],
        ..ToneCurve::default()
    };
    vec![
        DevelopSettings {
            tone: BasicTone {
                exposure: 0.7,
                ..BasicTone::default()
            },
            ..DevelopSettings::default()
        },
        DevelopSettings {
            white_balance: WhiteBalance {
                mode: WhiteBalanceMode::Custom,
                temperature: 3200.0,
                tint: 15.0,
            },
            tone: BasicTone {
                exposure: -1.3,
                contrast: 40.0,
                highlights: -80.0,
                shadows: 60.0,
                whites: 25.0,
                blacks: -35.0,
            },
            ..DevelopSettings::default()
        },
        DevelopSettings {
            tone: BasicTone {
                contrast: -100.0,
                highlights: 100.0,
                shadows: -100.0,
                whites: -100.0,
                blacks: 100.0,
                exposure: 5.0,
            },
            tone_curve: curve.clone(),
            ..DevelopSettings::default()
        },
        DevelopSettings {
            white_balance: WhiteBalance {
                mode: WhiteBalanceMode::Custom,
                temperature: 9000.0,
                tint: -40.0,
            },
            tone_curve: curve,
            ..DevelopSettings::default()
        },
    ]
}
