//! CPU tests of the full develop: geometry, tone stages, and sharpening.

#![allow(clippy::expect_used, reason = "tests panic on failure")]

mod common;

use common::{all_stages_settings, crop, edge_image, rect, sharpening, smooth_image, test_image};
use expose_core::buffer::WorkingImage;
use expose_core::develop::{BasicTone, DevelopSettings};
use expose_develop::{DevelopError, Geometry, Pipeline, Sharpener, SourceInfo, Stage};

fn pipeline(settings: &DevelopSettings) -> Pipeline {
    Pipeline::new(settings, &SourceInfo::RENDERED).expect("pipeline")
}

fn bits(image: &WorkingImage) -> Vec<u32> {
    image.as_slice().iter().map(|v| v.to_bits()).collect()
}

fn assert_same(a: &WorkingImage, b: &WorkingImage) {
    assert_eq!((a.width(), a.height()), (b.width(), b.height()));
    assert_eq!(bits(a), bits(b));
}

fn crop_only() -> DevelopSettings {
    DevelopSettings {
        crop: crop(rect(0.2, 0.1, 0.5, 0.6), 0.0, true, true),
        ..DevelopSettings::default()
    }
}

#[test]
fn default_settings_give_the_input() {
    let p = pipeline(&DevelopSettings::default());
    assert!(p.is_identity() && p.tone_is_identity());
    assert!(!p.has_geometry() && !p.has_sharpening());
    let img = test_image();
    assert_same(&p.develop_cpu(&img, 1.0).expect("develop"), &img);
    assert_same(
        &p.develop_cpu_owned(img.clone(), 0.3).expect("develop"),
        &img,
    );
}

#[test]
fn stage_list_has_geometry_first_and_sharpening_last() {
    let mut p = pipeline(&all_stages_settings());
    let stages = p.stages();
    assert!(matches!(stages.first(), Some(Stage::Geometry { .. })));
    assert!(matches!(stages.last(), Some(Stage::Sharpen { .. })));
    assert!(stages.len() > 2);
    p.set_apply_geometry(false);
    assert!(!p.applies_geometry());
    assert!(
        p.has_geometry(),
        "the switch does not change the settings check"
    );
    assert!(!matches!(p.stages().first(), Some(Stage::Geometry { .. })));
}

#[test]
fn crop_only_equals_the_geometry_stage() {
    let img = test_image();
    for settings in [
        crop_only(),
        DevelopSettings {
            crop: crop(rect(0.1, 0.1, 0.8, 0.8), -3.0, false, true),
            ..DevelopSettings::default()
        },
    ] {
        let p = pipeline(&settings);
        assert!(p.has_geometry() && !p.is_identity() && p.tone_is_identity());
        let expected = Geometry::new(&settings.crop, img.width(), img.height())
            .expect("geometry")
            .apply_cpu(&img)
            .expect("apply");
        assert_same(&p.develop_cpu(&img, 1.0).expect("develop"), &expected);
        assert_same(
            &p.develop_cpu_owned(img.clone(), 1.0).expect("develop"),
            &expected,
        );
    }
}

#[test]
fn tone_only_paths_ignore_geometry_and_sharpening() {
    let img = test_image();
    let mut settings = crop_only();
    settings.sharpening = sharpening(100.0, 0.0);
    let p = pipeline(&settings);
    assert!(!p.is_identity());
    assert!(p.tone_is_identity());
    assert_same(&p.render_cpu(&img), &img);
    let mut copy = img.clone();
    p.apply_cpu(&mut copy);
    assert_same(&copy, &img);
}

#[test]
fn sharpen_only_equals_the_sharpener() {
    let img = edge_image(40, 24);
    let settings = DevelopSettings {
        sharpening: sharpening(130.0, 30.0),
        ..DevelopSettings::default()
    };
    let p = pipeline(&settings);
    assert!(p.has_sharpening() && !p.is_identity());
    for scale in [1.0, 0.5] {
        let s = Sharpener::new(&settings.sharpening, scale)
            .expect("sharpener")
            .expect("active at this scale");
        let mut expected = img.clone();
        s.apply_cpu(&mut expected).expect("sharpen");
        assert_same(&p.develop_cpu(&img, scale).expect("develop"), &expected);
    }
    // At a very small scale the stage is a no-op.
    assert!(p.sharpener(0.05).expect("sharpener").is_none());
    assert_same(&p.develop_cpu(&img, 0.05).expect("develop"), &img);
}

#[test]
fn geometry_comes_before_the_tone_stages() {
    let img = test_image();
    let mut settings = crop_only();
    settings.tone = BasicTone {
        exposure: 0.8,
        contrast: 30.0,
        ..BasicTone::default()
    };
    let p = pipeline(&settings);
    let cropped = Geometry::new(&settings.crop, img.width(), img.height())
        .expect("geometry")
        .apply_cpu(&img)
        .expect("apply");
    let expected = p.render_cpu(&cropped);
    assert_same(&p.develop_cpu(&img, 1.0).expect("develop"), &expected);
}

#[test]
fn sharpening_comes_after_the_tone_stages() {
    let img = edge_image(32, 20);
    let settings = DevelopSettings {
        tone: BasicTone {
            exposure: -0.5,
            contrast: 40.0,
            ..BasicTone::default()
        },
        sharpening: sharpening(150.0, 0.0),
        ..DevelopSettings::default()
    };
    let p = pipeline(&settings);
    let mut expected = p.render_cpu(&img);
    p.sharpener(1.0)
        .expect("sharpener")
        .expect("active")
        .apply_cpu(&mut expected)
        .expect("sharpen");
    assert_same(&p.develop_cpu(&img, 1.0).expect("develop"), &expected);
}

#[test]
fn geometry_switch_skips_the_crop() {
    let img = smooth_image(30, 20);
    let mut settings = all_stages_settings();
    let mut p = pipeline(&settings);
    p.set_apply_geometry(false);
    assert_eq!(p.output_size(30, 20).expect("size"), (30, 20));
    let out = p.develop_cpu(&img, 1.0).expect("develop");
    // Same as the settings without a crop.
    settings.crop = expose_core::develop::Crop::default();
    let expected = pipeline(&settings).develop_cpu(&img, 1.0).expect("develop");
    assert_same(&out, &expected);

    // Crop only, switch off: nothing is active.
    let mut p = pipeline(&crop_only());
    p.set_apply_geometry(false);
    assert!(p.is_identity());
    assert_same(&p.develop_cpu(&img, 1.0).expect("develop"), &img);
}

#[test]
fn output_size_follows_the_crop() {
    let p = pipeline(&crop_only());
    // 0.5 * 37 = 18.5 rounds to 19; 0.6 * 23 = 13.8 rounds to 14.
    assert_eq!(p.output_size(37, 23).expect("size"), (19, 14));
    let out = p.develop_cpu(&test_image(), 1.0).expect("develop");
    assert_eq!((out.width(), out.height()), (19, 14));
    let none = pipeline(&DevelopSettings::default());
    assert_eq!(none.output_size(37, 23).expect("size"), (37, 23));
    assert!(none.output_size(0, 23).is_err());
    assert!(p.output_size(37, 0).is_err());
}

#[test]
fn bad_pixel_scale_is_an_error() {
    let p = pipeline(&DevelopSettings::default());
    let img = test_image();
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(matches!(
            p.develop_cpu(&img, scale),
            Err(DevelopError::Core(_))
        ));
    }
}

#[test]
fn full_develop_is_deterministic() {
    let img = smooth_image(41, 27);
    let p = pipeline(&all_stages_settings());
    let a = p.develop_cpu(&img, 0.8).expect("develop");
    let b = p.develop_cpu(&img, 0.8).expect("develop");
    assert_same(&a, &b);
    assert_eq!(
        (a.width(), a.height()),
        p.output_size(41, 27).expect("size")
    );
    assert_same(&p.develop_cpu_owned(img, 0.8).expect("develop"), &a);
}
