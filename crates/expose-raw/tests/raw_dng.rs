//! RAW decode tests with synthetic DNG files (rawler backend).
#![cfg(feature = "rawler")]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "test data is small"
)]
#![allow(clippy::unwrap_used, reason = "test helpers")]

mod common;

use common::{DngSpec, for_each_mutation};
use expose_core::color::{ColorSpaceId, mul_vec_f32};
use expose_raw::raw::{
    AsShotWhite, CfaColor, DemosaicMethod, DemosaicOptions, SensorLayout, decode_raw, demosaic,
};
use expose_raw::{
    DecodeError, DecodeOptions, ImageFormat, Limits, Orientation, RawFormat, decode, decode_with,
    detect_format, read_metadata,
};

const RGGB: [u8; 4] = [0, 1, 1, 2];
const BGGR: [u8; 4] = [2, 1, 1, 0];
const GRBG: [u8; 4] = [1, 0, 2, 1];
const GBRG: [u8; 4] = [1, 2, 0, 1];

fn assert_close(a: f32, b: f32, tol: f32, what: &str) {
    assert!((a - b).abs() < tol, "{what}: {a} != {b}");
}

#[test]
fn detects_dng() {
    let dng = DngSpec::new(8, 8, RGGB, |_, _, _| 1000).build();
    assert_eq!(detect_format(&dng), Some(ImageFormat::Raw(RawFormat::Dng)));
}

#[test]
fn flat_gray_patch_decodes_to_neutral_gray() {
    // Camera neutral (0.5, 1.0, 0.8): a gray object gives these relative
    // raw values. Black 1024, white 15000; the gray is 40% of the range.
    let neutral = [0.5, 1.0, 0.8];
    let (black, white) = (1024.0_f64, 15000.0_f64);
    let mut spec = DngSpec::new(16, 12, RGGB, |c, _, _| {
        (black + 0.4 * neutral[usize::from(c)] * (white - black)).round() as u16
    });
    spec.black = 1024;
    spec.white = 15000;
    spec.neutral = neutral;
    let dng = spec.build();

    let out = decode(&dng).unwrap();
    assert_eq!((out.image.width(), out.image.height()), (16, 12));
    let info = out.info.raw.as_ref().unwrap();
    assert_eq!(info.backend, "rawler");
    assert!(!info.white_balance_assumed);
    for p in out.image.pixels() {
        for v in &p[..3] {
            assert_close(*v, 0.4, 2e-3, "gray");
        }
        assert_close(p[3], 1.0, 1e-6, "alpha");
    }
}

#[test]
fn cfa_patterns_map_to_correct_colors() {
    // A constant camera color with neutral (1,1,1). The camera sees linear
    // sRGB, so the output is the sRGB color in Rec. 2020.
    let col = [0.6_f32, 0.3, 0.1];
    let expected = mul_vec_f32(&ColorSpaceId::Srgb.to_working().unwrap().to_f32(), col);
    for (name, pattern) in [
        ("RGGB", RGGB),
        ("BGGR", BGGR),
        ("GRBG", GRBG),
        ("GBRG", GBRG),
    ] {
        let dng = DngSpec::new(10, 8, pattern, |c, _, _| {
            (col[usize::from(c)] * 65535.0).round() as u16
        })
        .build();
        let raw = decode_raw(&dng, &Limits::default()).unwrap();
        let SensorLayout::Cfa(p) = &raw.layout else {
            panic!("{name}: not CFA");
        };
        let first = match pattern[0] {
            0 => CfaColor::Red,
            1 => CfaColor::Green,
            _ => CfaColor::Blue,
        };
        assert_eq!(p.color_at(0, 0), first, "{name}");
        for method in [DemosaicMethod::Bilinear, DemosaicMethod::Mhc] {
            let opts = DemosaicOptions {
                method,
                ..DemosaicOptions::default()
            };
            let out = demosaic(&raw, &opts).unwrap();
            for px in out.image.pixels() {
                for c in 0..3 {
                    assert_close(px[c], expected[c], 2e-3, name);
                }
            }
        }
    }
}

#[test]
fn black_and_white_levels_apply() {
    // 12-bit data: black 256, white 4095. Raw value 4095 is 1.0, 256 is 0.
    let mut spec = DngSpec::new(8, 8, RGGB, |_, x, _| if x < 4 { 256 } else { 4095 });
    spec.black = 256;
    spec.white = 4095;
    spec.bits = 16;
    let raw = decode_raw(&spec.build(), &Limits::default()).unwrap();
    assert_close(raw.black.at(0, 0, 0), 256.0, 1e-3, "black");
    assert_close(raw.white.at(0, 0, 0), 4095.0, 1e-3, "white");
    assert_eq!(raw.as_shot, AsShotWhite::Neutral([1.0, 1.0, 1.0]));
    let out = demosaic(
        &raw,
        &DemosaicOptions {
            method: DemosaicMethod::Bilinear,
            ..DemosaicOptions::default()
        },
    )
    .unwrap();
    // Far from the edge between the halves, the values are exact.
    let dark = out.image.pixel(0, 4).unwrap();
    let bright = out.image.pixel(7, 4).unwrap();
    for c in 0..3 {
        assert_close(dark[c], 0.0, 1e-3, "dark");
        assert_close(bright[c], 1.0, 2e-3, "bright");
    }
}

#[test]
fn orientation_crop_and_metadata() {
    let mut spec = DngSpec::new(12, 8, RGGB, |_, _, _| 30000);
    spec.orientation = 6;
    spec.crop = Some(([2, 2], [8, 4]));
    spec.iso = Some(400);
    let dng = spec.build();

    let out = decode(&dng).unwrap();
    assert_eq!(out.info.orientation, Orientation::Rotate90);
    // The crop is 8x4; rotated it is 4x8.
    assert_eq!((out.image.width(), out.image.height()), (4, 8));

    let not_oriented = decode_with(
        &dng,
        &DecodeOptions {
            apply_orientation: false,
            ..DecodeOptions::default()
        },
    )
    .unwrap();
    assert_eq!(not_oriented.image.width(), 8);

    let m = read_metadata(&dng).unwrap();
    assert_eq!(m.format, ImageFormat::Raw(RawFormat::Dng));
    assert_eq!(m.make.as_deref(), Some("Synthetic"));
    assert_eq!(m.model.as_deref(), Some("Test Sensor"));
    assert_eq!(m.orientation, Some(Orientation::Rotate90));
    assert_eq!(m.iso, Some(400));
    assert_eq!((m.width, m.height), (Some(8), Some(4)));
    let t = m.capture_time.unwrap();
    assert_eq!(t.original, "2025:01:02 03:04:05");
    assert_eq!(t.offset_minutes, None);
}

#[test]
fn limits_reject_large_images_before_decode() {
    let dng = DngSpec::new(64, 64, RGGB, |_, _, _| 1).build();
    let small = Limits { max_pixels: 1000 };
    assert!(matches!(
        decode_raw(&dng, &small),
        Err(DecodeError::TooLarge { .. })
    ));
}

#[test]
fn corrupt_dng_returns_errors_without_panic() {
    let mut spec = DngSpec::new(16, 16, RGGB, |c, x, y| {
        1000 + u16::from(c) * 100 + (x * y) as u16
    });
    spec.iso = Some(100);
    let dng = spec.build();
    let opts = DecodeOptions {
        limits: Limits {
            max_pixels: 1 << 20,
        },
        ..DecodeOptions::default()
    };
    let mut ok = 0;
    let mut err = 0;
    for_each_mutation(&dng, 7, |data| {
        // Only "no panic" is checked: a mutation can give a valid file.
        match decode_with(data, &opts) {
            Ok(_) => ok += 1,
            Err(_) => err += 1,
        }
        let _ = read_metadata(data);
    });
    assert!(err > 0 && ok > 0, "ok {ok}, err {err}");
}

/// Decodes all files in the folder named by `EXPOSE_RAW_SAMPLES`. Run with
/// `cargo test -p expose-raw -- --ignored real_samples --nocapture`.
#[test]
#[ignore = "needs real camera files (set EXPOSE_RAW_SAMPLES)"]
fn real_samples() {
    let Ok(dir) = std::env::var("EXPOSE_RAW_SAMPLES") else {
        eprintln!("EXPOSE_RAW_SAMPLES is not set; nothing to do");
        return;
    };
    let mut failures = Vec::new();
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if !path.is_file() {
            continue;
        }
        count += 1;
        let data = std::fs::read(&path).unwrap();
        let started = std::time::Instant::now();
        match (read_metadata(&data), decode(&data)) {
            (Ok(m), Ok(img)) => {
                let raw = img.info.raw.as_ref();
                eprintln!(
                    "ok   {} {:?} {:?} {:?} {}x{} {:?} wb_assumed={:?} {:?}",
                    path.display(),
                    img.info.format,
                    m.make,
                    m.model,
                    img.image.width(),
                    img.image.height(),
                    img.info.orientation,
                    raw.map(|r| r.white_balance_assumed),
                    started.elapsed()
                );
            }
            (m, d) => {
                eprintln!(
                    "FAIL {}: metadata {:?}, decode {:?}",
                    path.display(),
                    m.err(),
                    d.err()
                );
                failures.push(path);
            }
        }
    }
    eprintln!("{count} files, {} failures", failures.len());
    assert!(failures.is_empty(), "failed: {failures:?}");
}
