//! Standard format decode and metadata tests.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "test data is small"
)]
#![allow(clippy::unwrap_used, reason = "test helpers")]

mod common;

use std::io::Cursor;

use common::{Ifd, Val, for_each_mutation, rat};
use expose_core::color::{ColorSpaceId, TransferFunction, mul_vec_f32};
use expose_raw::{
    ColorOrigin, DecodeError, DecodeOptions, ExifStatus, ImageFormat, Limits, Orientation,
    SourceTransfer, decode, decode_with, detect_format, read_metadata,
};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::tiff::TiffEncoder;
use image::codecs::webp::WebPEncoder;
use image::{ExtendedColorType, ImageEncoder};

fn png(w: u32, h: u32, data: &[u8], ct: ExtendedColorType, icc: Option<Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = PngEncoder::new(&mut out);
    if let Some(p) = icc {
        enc.set_icc_profile(p).unwrap();
    }
    enc.write_image(data, w, h, ct).unwrap();
    out
}

fn tiff(w: u32, h: u32, data: &[u8], ct: ExtendedColorType) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    TiffEncoder::new(&mut out)
        .write_image(data, w, h, ct)
        .unwrap();
    out.into_inner()
}

fn jpeg(w: u32, h: u32, data: &[u8], exif: Option<Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = JpegEncoder::new_with_quality(&mut out, 95);
    if let Some(e) = exif {
        enc.set_exif_metadata(e).unwrap();
    }
    enc.write_image(data, w, h, ExtendedColorType::Rgb8)
        .unwrap();
    out
}

fn srgb_to_working(rgb: [f32; 3]) -> [f32; 3] {
    let lin = rgb.map(|v| TransferFunction::Srgb.decode(v));
    mul_vec_f32(&ColorSpaceId::Srgb.to_working().unwrap().to_f32(), lin)
}

fn assert_px(p: [f32; 4], e: [f32; 3], tol: f32) {
    for c in 0..3 {
        assert!((p[c] - e[c]).abs() < tol, "{p:?} != {e:?}");
    }
}

#[test]
fn png_8bit_without_profile_is_assumed_srgb() {
    let data = [255, 0, 0, 0, 255, 0, 128, 128, 128, 0, 0, 0];
    let file = png(2, 2, &data, ExtendedColorType::Rgb8, None);
    assert_eq!(detect_format(&file), Some(ImageFormat::Png));
    let out = decode(&file).unwrap();
    assert!(out.info.assumed_color_space);
    assert_eq!(out.info.bits_per_sample, 8);
    assert_px(
        out.image.pixel(0, 0).unwrap(),
        srgb_to_working([1.0, 0.0, 0.0]),
        1e-5,
    );
    assert_px(
        out.image.pixel(1, 0).unwrap(),
        srgb_to_working([0.0, 1.0, 0.0]),
        1e-5,
    );
    let gray = TransferFunction::Srgb.decode(128.0 / 255.0);
    assert_px(out.image.pixel(0, 1).unwrap(), [gray; 3], 1e-5);
    assert_eq!(out.metadata.exif_status, ExifStatus::Absent);
    assert_eq!(
        (out.metadata.width, out.metadata.height),
        (Some(2), Some(2))
    );
}

#[test]
fn png_16bit_gray_alpha() {
    let samples: [u16; 4] = [65535, 32768, 0, 65535];
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_ne_bytes()).collect();
    let file = png(2, 1, &bytes, ExtendedColorType::La16, None);
    let out = decode(&file).unwrap();
    assert_eq!(out.info.bits_per_sample, 16);
    assert!(out.info.has_alpha);
    let p = out.image.pixel(0, 0).unwrap();
    assert_px(p, [1.0; 3], 1e-5);
    assert!((p[3] - 32768.0 / 65535.0).abs() < 1e-5);
}

#[test]
fn png_with_display_p3_profile() {
    let profile = p3_profile();
    let file = png(1, 1, &[255, 0, 0], ExtendedColorType::Rgb8, Some(profile));
    let out = decode(&file).unwrap();
    let c = out.info.source_color.unwrap();
    assert_eq!(c.space, ColorSpaceId::DisplayP3);
    assert_eq!(c.origin, ColorOrigin::IccProfile);
    assert_eq!(c.transfer, SourceTransfer::Srgb);
    assert!(!out.info.assumed_color_space);
    assert_eq!(out.info.icc_description.as_deref(), Some("Test P3"));
    let expected = mul_vec_f32(
        &ColorSpaceId::DisplayP3.to_working().unwrap().to_f32(),
        [1.0, 0.0, 0.0],
    );
    assert_px(out.image.pixel(0, 0).unwrap(), expected, 1e-4);
}

/// A Display P3 matrix/curve profile with a parametric sRGB curve, built
/// here so that the test does not depend on the crate internals.
fn p3_profile() -> Vec<u8> {
    let s15 = |v: f64| ((v * 65536.0).round() as i32).to_be_bytes();
    let def = ColorSpaceId::DisplayP3.definition();
    let adapt =
        expose_core::color::bradford_adaptation(def.white, expose_core::color::white::D50).unwrap();
    let m = adapt * def.to_xyz().unwrap();
    let mut tags: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    for (i, sig) in [*b"rXYZ", *b"gXYZ", *b"bXYZ"].into_iter().enumerate() {
        let mut t = b"XYZ \0\0\0\0".to_vec();
        for r in 0..3 {
            t.extend_from_slice(&s15(m.rows[r][i]));
        }
        tags.push((sig, t));
    }
    let mut para = b"para\0\0\0\0".to_vec();
    para.extend_from_slice(&3_u16.to_be_bytes());
    para.extend_from_slice(&[0, 0]);
    for v in [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.040_45] {
        para.extend_from_slice(&s15(v));
    }
    tags.push((*b"rTRC", para));
    let mut desc = b"desc\0\0\0\0".to_vec();
    desc.extend_from_slice(&8_u32.to_be_bytes());
    desc.extend_from_slice(b"Test P3\0");
    tags.push((*b"desc", desc));

    let mut out = vec![0_u8; 128];
    out[16..20].copy_from_slice(b"RGB ");
    out[20..24].copy_from_slice(b"XYZ ");
    out[36..40].copy_from_slice(b"acsp");
    out.extend_from_slice(&(tags.len() as u32).to_be_bytes());
    let table_end = 132 + tags.len() * 12;
    let mut data = Vec::new();
    for (sig, t) in &tags {
        out.extend_from_slice(sig);
        out.extend_from_slice(&((table_end + data.len()) as u32).to_be_bytes());
        out.extend_from_slice(&(t.len() as u32).to_be_bytes());
        data.extend_from_slice(t);
        while data.len() % 4 != 0 {
            data.push(0);
        }
    }
    out.extend_from_slice(&data);
    let len = out.len() as u32;
    out[0..4].copy_from_slice(&len.to_be_bytes());
    out
}

#[test]
fn tiff_16bit_and_float() {
    let s16: [u16; 3] = [65535, 0, 32768];
    let b16: Vec<u8> = s16.iter().flat_map(|s| s.to_ne_bytes()).collect();
    let out = decode(&tiff(1, 1, &b16, ExtendedColorType::Rgb16)).unwrap();
    assert_eq!(out.info.format, ImageFormat::Tiff);
    assert_eq!(out.info.bits_per_sample, 16);
    assert_px(
        out.image.pixel(0, 0).unwrap(),
        srgb_to_working([1.0, 0.0, 32768.0 / 65535.0]),
        1e-4,
    );

    // Float data without a profile: linear sRGB primaries, assumed. Values
    // above 1.0 are kept.
    let f: [f32; 3] = [2.0, 0.5, 0.25];
    let bf: Vec<u8> = f.iter().flat_map(|s| s.to_ne_bytes()).collect();
    let out = decode(&tiff(1, 1, &bf, ExtendedColorType::Rgb32F)).unwrap();
    assert_eq!(out.info.bits_per_sample, 32);
    assert!(out.info.assumed_color_space);
    assert_eq!(
        out.info.source_color.unwrap().transfer,
        SourceTransfer::Linear
    );
    let expected = mul_vec_f32(&ColorSpaceId::Srgb.to_working().unwrap().to_f32(), f);
    assert_px(out.image.pixel(0, 0).unwrap(), expected, 1e-5);
}

#[test]
fn webp_lossless() {
    let mut out = Vec::new();
    WebPEncoder::new_lossless(&mut out)
        .write_image(&[10, 20, 30, 255], 1, 1, ExtendedColorType::Rgba8)
        .unwrap();
    assert_eq!(detect_format(&out), Some(ImageFormat::WebP));
    let img = decode(&out).unwrap();
    assert_px(
        img.image.pixel(0, 0).unwrap(),
        srgb_to_working([10.0 / 255.0, 20.0 / 255.0, 30.0 / 255.0]),
        1e-5,
    );
}

/// An EXIF block (TIFF structure) with orientation 6, camera data, a
/// capture time with offset, and GPS.
fn exif_block() -> Vec<u8> {
    let exif = Ifd::default()
        .with(0x829A, Val::Rational(vec![(1, 125)]))
        .with(0x829D, Val::Rational(vec![(56, 10)]))
        .with(0x8827, Val::Short(vec![200]))
        .with(0x9003, Val::Ascii("2024:06:30 18:45:10".into()))
        .with(0x9011, Val::Ascii("+02:00".into()))
        .with(0x920A, Val::Rational(vec![(50, 1)]))
        .with(0xA001, Val::Short(vec![1]))
        .with(0xA434, Val::Ascii("Test Lens 50mm".into()));
    let gps = Ifd::default()
        .with(1, Val::Ascii("N".into()))
        .with(2, Val::Rational(vec![(48, 1), (51, 1), rat(29.6)]))
        .with(3, Val::Ascii("W".into()))
        .with(4, Val::Rational(vec![(2, 1), (17, 1), (40, 1)]))
        .with(5, Val::Byte(vec![1]))
        .with(6, Val::Rational(vec![(10, 1)]));
    let ifd0 = Ifd::default()
        .with(0x010F, Val::Ascii("TestMaker".into()))
        .with(0x0110, Val::Ascii("Model X".into()))
        .with(0x0112, Val::Short(vec![6]))
        .with(0x8769, Val::Ifd(Box::new(exif)))
        .with(0x8825, Val::Ifd(Box::new(gps)));
    common::tiff(&ifd0)
}

#[test]
fn jpeg_exif_metadata_and_orientation() {
    // 4x2 image; left column red.
    let mut data = vec![0_u8; 4 * 2 * 3];
    for y in 0..2 {
        data[y * 12] = 255;
    }
    let file = jpeg(4, 2, &data, Some(exif_block()));
    let m = read_metadata(&file).unwrap();
    assert_eq!(m.exif_status, ExifStatus::Present);
    assert_eq!(m.make.as_deref(), Some("TestMaker"));
    assert_eq!(m.model.as_deref(), Some("Model X"));
    assert_eq!(m.lens.as_deref(), Some("Test Lens 50mm"));
    assert_eq!(m.orientation, Some(Orientation::Rotate90));
    assert_eq!(m.iso, Some(200));
    assert!((m.f_number.unwrap() - 5.6).abs() < 1e-9);
    assert!((m.focal_length_mm.unwrap() - 50.0).abs() < 1e-9);
    let e = m.exposure_time.unwrap();
    assert_eq!((e.numerator, e.denominator), (1, 125));
    let t = m.capture_time.as_ref().unwrap();
    assert_eq!(t.original, "2024:06:30 18:45:10");
    assert_eq!(t.offset_minutes, Some(120));
    let g = m.gps.unwrap();
    assert!((g.latitude - (48.0 + 51.0 / 60.0 + 29.6 / 3600.0)).abs() < 1e-6);
    assert!((g.longitude + (2.0 + 17.0 / 60.0 + 40.0 / 3600.0)).abs() < 1e-6);
    assert_eq!(g.altitude_meters, Some(-10.0));
    assert_eq!((m.width, m.height), (Some(4), Some(2)));

    let out = decode(&file).unwrap();
    assert_eq!(out.info.orientation, Orientation::Rotate90);
    assert_eq!(out.info.source_color.unwrap().origin, ColorOrigin::Exif);
    assert!(!out.info.assumed_color_space);
    // Rotated 90 degrees clockwise: the left column is now the top row.
    assert_eq!((out.image.width(), out.image.height()), (2, 4));
    let top = out.image.pixel(1, 0).unwrap();
    let bottom = out.image.pixel(1, 3).unwrap();
    assert!(top[0] > 0.5, "{top:?}");
    assert!(bottom[0] < 0.1, "{bottom:?}");

    let json = serde_json::to_value(&m).unwrap();
    assert_eq!(json["captureTime"]["offsetMinutes"], 120);
    assert_eq!(json["orientation"], 6);
}

#[test]
fn pillow_fixture_metadata() {
    let file = include_bytes!("fixtures/pillow_exif.jpg");
    let m = read_metadata(file).unwrap();
    assert_eq!(m.make.as_deref(), Some("FixtureMaker"));
    assert_eq!(m.model.as_deref(), Some("Fixture One"));
    assert_eq!(m.lens.as_deref(), Some("Fixture 35mm F2.8"));
    assert_eq!(m.orientation, Some(Orientation::Rotate180));
    assert_eq!(m.iso, Some(800));
    assert!((m.f_number.unwrap() - 2.8).abs() < 1e-9);
    let t = m.capture_time.as_ref().unwrap();
    assert_eq!(t.offset_minutes, Some(-300));
    let l = t.local.unwrap();
    assert_eq!(
        (l.year, l.month, l.day, l.hour, l.minute, l.second),
        (2023, 7, 14, 21, 5, 33)
    );
    let g = m.gps.unwrap();
    assert!((g.latitude + 33.0 + 52.0 / 60.0).abs() < 1e-6);
    assert!((g.longitude - (151.0 + 12.0 / 60.0 + 36.0 / 3600.0)).abs() < 1e-6);
    assert_eq!(g.altitude_meters, Some(58.0));

    // Orientation 3: the red left half becomes the right half.
    let out = decode(file).unwrap();
    assert_eq!((out.image.width(), out.image.height()), (32, 16));
    assert!(out.image.pixel(28, 8).unwrap()[0] > 0.5);
    assert!(out.image.pixel(2, 8).unwrap()[2] > 0.5);
}

#[test]
fn heif_is_detected_but_unsupported() {
    let mut heic = 24_u32.to_be_bytes().to_vec();
    heic.extend_from_slice(b"ftypheic\0\0\0\0mif1heic");
    assert_eq!(detect_format(&heic), Some(ImageFormat::Heif));
    assert!(matches!(
        decode(&heic),
        Err(DecodeError::UnsupportedFormat {
            format: ImageFormat::Heif,
            ..
        })
    ));
}

/// Without a RAW backend, RAW decode is a clear error, but metadata works.
#[cfg(not(feature = "rawler"))]
#[test]
fn raw_without_backend_is_unsupported() {
    let mut spec = common::DngSpec::new(8, 8, [0, 1, 1, 2], |_, _, _| 100);
    spec.crop = Some(([1, 1], [6, 5]));
    let dng = spec.build();
    assert!(matches!(
        decode(&dng),
        Err(DecodeError::UnsupportedFormat { .. })
    ));
    let m = read_metadata(&dng).unwrap();
    assert_eq!(m.make.as_deref(), Some("Synthetic"));
    assert_eq!((m.width, m.height), (Some(6), Some(5)));
}

#[test]
fn limits_apply_to_standard_formats() {
    let file = png(40, 40, &[0; 40 * 40 * 3], ExtendedColorType::Rgb8, None);
    let opts = DecodeOptions {
        limits: Limits { max_pixels: 100 },
        ..DecodeOptions::default()
    };
    assert!(matches!(
        decode_with(&file, &opts),
        Err(DecodeError::TooLarge { .. })
    ));
}

#[test]
fn corrupt_standard_files_do_not_panic() {
    let mut data = vec![0_u8; 8 * 8 * 3];
    for (i, v) in data.iter_mut().enumerate() {
        *v = (i * 7) as u8;
    }
    let files = [
        jpeg(8, 8, &data, Some(exif_block())),
        png(8, 8, &data, ExtendedColorType::Rgb8, Some(p3_profile())),
        tiff(8, 8, &data, ExtendedColorType::Rgb8),
        include_bytes!("fixtures/pillow_exif.jpg").to_vec(),
    ];
    let opts = DecodeOptions {
        limits: Limits {
            max_pixels: 1 << 16,
        },
        ..DecodeOptions::default()
    };
    for f in &files {
        for_each_mutation(f, 3, |d| {
            let _ = decode_with(d, &opts);
            let _ = read_metadata(d);
        });
    }
}
