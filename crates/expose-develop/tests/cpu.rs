//! CPU reference tests: golden values and determinism.

#![allow(clippy::float_cmp, reason = "golden tests expect exact values")]

mod common;

use common::{settings_set, test_image};
use expose_core::color::{luminance, white};
use expose_core::develop::{BasicTone, DevelopSettings, WhiteBalanceMode};
use expose_develop::{
    DevelopError, LuminanceHistogram, Pipeline, SourceInfo, SourceWhite, Stage, auto_tone,
};

#[test]
fn identity_settings_are_exact_identity() {
    let p = Pipeline::new(&DevelopSettings::default(), &SourceInfo::RENDERED).unwrap();
    assert!(p.is_identity());
    let img = test_image();
    let out = p.render_cpu(&img);
    for (a, b) in img.as_slice().iter().zip(out.as_slice()) {
        assert_eq!(a.to_bits(), b.to_bits());
    }
}

#[test]
fn exposure_plus_one_doubles() {
    let settings = DevelopSettings {
        tone: BasicTone {
            exposure: 1.0,
            ..BasicTone::default()
        },
        ..DevelopSettings::default()
    };
    let p = Pipeline::new(&settings, &SourceInfo::RENDERED).unwrap();
    assert_eq!(p.stages(), &[Stage::Exposure { ev: 1.0 }]);
    let img = test_image();
    let out = p.render_cpu(&img);
    for (a, b) in img.pixels().iter().zip(out.pixels()) {
        for c in 0..3 {
            assert_eq!(b[c], a[c] * 2.0);
        }
        assert_eq!(b[3], a[3], "alpha is not changed");
    }
}

#[test]
fn as_shot_d65_is_identity() {
    let raw = SourceInfo {
        as_shot: SourceWhite::Chromaticity(white::D65),
    };
    let p = Pipeline::new(&DevelopSettings::default(), &raw).unwrap();
    assert!(p.is_identity(), "{:?}", p.stages());
    let neutral = SourceInfo {
        as_shot: SourceWhite::NeutralRgb([0.4, 0.4, 0.4]),
    };
    let p = Pipeline::new(&DevelopSettings::default(), &neutral).unwrap();
    // The neutral gives D65 within f64 rounding, so the stage is skipped.
    assert!(p.is_identity(), "{:?}", p.stages());
}

#[test]
fn as_shot_warm_light_neutralizes_gray() {
    let warm = expose_core::color::temperature_tint_to_chromaticity(3000.0, 0.0).unwrap();
    let to_xyz = expose_core::color::ColorSpaceId::LinearRec2020
        .definition()
        .to_xyz()
        .unwrap();
    let n = to_xyz.inverse().unwrap().mul_vec(warm.to_xyz().unwrap());
    let raw = SourceInfo {
        as_shot: SourceWhite::NeutralRgb(n),
    };
    let p = Pipeline::new(&DevelopSettings::default(), &raw).unwrap();
    #[allow(clippy::cast_possible_truncation)]
    let px = [
        (n[0] * 0.3) as f32,
        (n[1] * 0.3) as f32,
        (n[2] * 0.3) as f32,
        1.0,
    ];
    let out = p.apply_pixel(px);
    for v in &out[..3] {
        assert!((v - 0.3).abs() < 1e-5, "{out:?}");
    }
}

#[test]
fn output_is_deterministic() {
    let img = test_image();
    for s in settings_set() {
        let p = Pipeline::new(&s, &SourceInfo::RENDERED).unwrap();
        let a = p.render_cpu(&img);
        let b = Pipeline::new(&s, &SourceInfo::RENDERED)
            .unwrap()
            .render_cpu(&img);
        for (x, y) in a.as_slice().iter().zip(b.as_slice()) {
            assert_eq!(x.to_bits(), y.to_bits());
        }
    }
}

#[test]
fn stage_order_is_fixed_and_identity_stages_are_skipped() {
    let s = &settings_set()[1];
    let p = Pipeline::new(s, &SourceInfo::RENDERED).unwrap();
    let kinds: Vec<&str> = p
        .stages()
        .iter()
        .map(|s| match s {
            Stage::WhiteBalance { .. } => "wb",
            Stage::Exposure { .. } => "exposure",
            Stage::ToneZones { .. } => "zones",
            Stage::Contrast { .. } => "contrast",
            Stage::ToneCurve { .. } => "curve",
            Stage::Geometry { .. } => "geometry",
            Stage::Sharpen { .. } => "sharpen",
        })
        .collect();
    assert_eq!(kinds, ["wb", "exposure", "zones", "contrast"]);
}

#[test]
fn tone_stages_keep_channel_ratios_and_mid_gray() {
    let settings = DevelopSettings {
        tone: BasicTone {
            contrast: 70.0,
            ..BasicTone::default()
        },
        ..DevelopSettings::default()
    };
    let p = Pipeline::new(&settings, &SourceInfo::RENDERED).unwrap();
    let gray = p.apply_pixel([0.18, 0.18, 0.18, 1.0]);
    assert!((gray[0] - 0.18).abs() < 1e-5, "{gray:?}");
    let c = [0.5, 0.2, 0.05, 1.0];
    let out = p.apply_pixel(c);
    assert!((out[0] / out[1] - c[0] / c[1]).abs() < 1e-5);
    assert!(luminance([out[0], out[1], out[2]]) > luminance([c[0], c[1], c[2]]));
}

#[test]
fn invalid_settings_and_source_are_errors() {
    let mut s = DevelopSettings::default();
    s.tone.exposure = f32::NAN;
    assert!(matches!(
        Pipeline::new(&s, &SourceInfo::RENDERED),
        Err(DevelopError::Settings(_))
    ));
    let bad = SourceInfo {
        as_shot: SourceWhite::NeutralRgb([0.0; 3]),
    };
    let s = DevelopSettings::default();
    assert_eq!(s.white_balance.mode, WhiteBalanceMode::AsShot);
    assert!(matches!(
        Pipeline::new(&s, &bad),
        Err(DevelopError::Source(_))
    ));
}

#[test]
fn auto_tone_moves_dark_image_toward_mid_gray() {
    // A dark, flat image.
    let img = expose_core::buffer::WorkingImage::from_pixels(
        64,
        1,
        (0..64_u8)
            .map(|i| {
                let v = 0.005 + 0.0005 * f32::from(i);
                [v, v, v, 1.0]
            })
            .collect(),
    )
    .unwrap();
    let before = LuminanceHistogram::from_image(&img)
        .percentile(0.5)
        .unwrap();
    let tone = auto_tone(&LuminanceHistogram::from_image(&img)).unwrap();
    let settings = DevelopSettings {
        tone,
        ..DevelopSettings::default()
    };
    let out = Pipeline::new(&settings, &SourceInfo::RENDERED)
        .unwrap()
        .render_cpu(&img);
    let after = LuminanceHistogram::from_image(&out)
        .percentile(0.5)
        .unwrap();
    assert!(after.abs() < before.abs() * 0.5, "{before} -> {after}");
}
