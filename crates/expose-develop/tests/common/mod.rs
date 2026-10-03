//! Shared test helpers: test images and settings.

// Test helpers panic on failure. Each test file uses only some helpers.
#![allow(dead_code, clippy::cast_precision_loss, clippy::expect_used)]

use expose_core::buffer::{Rgba, WorkingImage};
use expose_core::develop::{
    BasicTone, CurvePoint, DevelopSettings, ToneCurve, WhiteBalance, WhiteBalanceMode,
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
