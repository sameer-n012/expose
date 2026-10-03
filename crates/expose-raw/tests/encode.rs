//! Encode round-trip tests: encode with this crate, decode with this crate,
//! and compare pixels and the reported color space.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "test data is small"
)]
#![allow(clippy::unwrap_used, reason = "tests")]

use expose_core::buffer::{AlphaMode, ChannelLayout, Image16, Sample, WorkingImage};
use expose_core::color::{ColorSpaceId, OPRGB_GAMMA};
use expose_raw::{
    BitDepth, ChromaSubsampling, ColorOrigin, EncodeError, EncodeOptions, ImageFormat,
    OutputFormat, SourceTransfer, TiffCompression, decode, detect_format, encode, icc_description,
};

/// The output spaces of P1 export, plus ROMM RGB.
const SPACES: [ColorSpaceId; 5] = [
    ColorSpaceId::Srgb,
    ColorSpaceId::DisplayP3,
    ColorSpaceId::OpRgb,
    ColorSpaceId::Rec2020,
    ColorSpaceId::ProPhotoRgb,
];

/// A smooth test image with colors inside the sRGB gamut (so inside all
/// output gamuts) and varied alpha.
fn test_image(w: u32, h: u32) -> WorkingImage {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let fx = x as f32 / (w.max(2) - 1) as f32;
            let fy = y as f32 / (h.max(2) - 1) as f32;
            let px = [fx, fy, 0.5 * (fx + fy), 0.25 + 0.75 * fx];
            data.extend(px.map(|v| (v * 65535.0).round() as u16));
        }
    }
    Image16::from_vec(w, h, ChannelLayout::Rgba, data)
        .unwrap()
        .to_working(ColorSpaceId::Srgb)
        .unwrap()
}

/// Returns the largest difference in codes between the two images after
/// conversion to `T` samples in `space`.
fn max_code_diff<T: Sample + Into<u32>>(
    a: &WorkingImage,
    b: &WorkingImage,
    space: ColorSpaceId,
    layout: ChannelLayout,
) -> u32 {
    let ia = a.to_image::<T>(space, layout).unwrap();
    let ib = b.to_image::<T>(space, layout).unwrap();
    ia.as_slice()
        .iter()
        .zip(ib.as_slice())
        .map(|(x, y)| (*x).into().abs_diff((*y).into()))
        .max()
        .unwrap()
}

/// Checks a lossless file (PNG or TIFF):
///
/// 1. The file samples (read with the `image` crate) are exactly the
///    samples of [`WorkingImage::to_image`] for `space`.
/// 2. The image that this crate decodes is close to `img` in linear light.
///    (A comparison of re-encoded codes is not useful near black: the steep
///    power curves turn float rounding of about 1e-8 into many 16-bit codes.)
fn check_lossless(
    img: &WorkingImage,
    bytes: &[u8],
    dec: &WorkingImage,
    space: ColorSpaceId,
    depth: BitDepth,
    alpha: bool,
) {
    let layout = if alpha {
        ChannelLayout::Rgba
    } else {
        ChannelLayout::Rgb
    };
    let file = image::load_from_memory(bytes).unwrap();
    match depth {
        BitDepth::Eight => {
            let expected = img.to_image::<u8>(space, layout).unwrap();
            let got = if alpha {
                file.into_rgba8().into_raw()
            } else {
                file.into_rgb8().into_raw()
            };
            assert_eq!(got.as_slice(), expected.as_slice(), "{space:?} 8-bit");
        }
        BitDepth::Sixteen => {
            let expected = img.to_image::<u16>(space, layout).unwrap();
            let got = if alpha {
                file.into_rgba16().into_raw()
            } else {
                file.into_rgb16().into_raw()
            };
            assert_eq!(got.as_slice(), expected.as_slice(), "{space:?} 16-bit");
        }
    }
    // Half an 8-bit code at white is about 0.005 in linear light for the
    // steepest curve.
    let tol = match depth {
        BitDepth::Eight => 0.01,
        BitDepth::Sixteen => 1e-4,
    };
    let channels = if alpha { 4 } else { 3 };
    for (a, b) in img.pixels().iter().zip(dec.pixels()) {
        for c in 0..channels {
            assert!((a[c] - b[c]).abs() < tol, "{space:?}: {a:?} vs {b:?}");
        }
    }
}

fn expected_transfer(space: ColorSpaceId) -> SourceTransfer {
    match space {
        ColorSpaceId::Srgb | ColorSpaceId::DisplayP3 => SourceTransfer::Srgb,
        ColorSpaceId::OpRgb => SourceTransfer::Gamma(OPRGB_GAMMA),
        ColorSpaceId::Rec2020 => SourceTransfer::Rec2020,
        ColorSpaceId::ProPhotoRgb => SourceTransfer::ProPhoto,
        ColorSpaceId::LinearRec2020 => SourceTransfer::Linear,
    }
}

/// Checks that the decoder found the embedded profile of `space`.
fn assert_profile(info: &expose_raw::DecodeInfo, space: ColorSpaceId) {
    assert!(info.has_icc_profile, "{space:?}");
    assert_eq!(
        info.icc_description.as_deref(),
        Some(icc_description(space))
    );
    let color = info.source_color.unwrap();
    assert_eq!(color.origin, ColorOrigin::IccProfile, "{space:?}");
    let primaries = match space {
        ColorSpaceId::LinearRec2020 => ColorSpaceId::Rec2020,
        other => other,
    };
    assert_eq!(color.space, primaries);
    match (color.transfer, expected_transfer(space)) {
        (SourceTransfer::Gamma(a), SourceTransfer::Gamma(b)) => assert!((a - b).abs() < 1e-3),
        (a, b) => assert_eq!(a, b, "{space:?}"),
    }
    assert!(!info.assumed_color_space);
}

#[test]
fn png_round_trip_all_spaces() {
    let img = test_image(32, 16);
    for space in SPACES {
        for (depth, alpha) in [
            (BitDepth::Eight, false),
            (BitDepth::Eight, true),
            (BitDepth::Sixteen, false),
            (BitDepth::Sixteen, true),
        ] {
            let opts = EncodeOptions::png(depth, alpha).with_color_space(space);
            let bytes = encode(&img, &opts).unwrap();
            assert_eq!(detect_format(&bytes), Some(ImageFormat::Png));
            let dec = decode(&bytes).unwrap();
            assert_profile(&dec.info, space);
            assert_eq!(dec.info.has_alpha, alpha);
            check_lossless(&img, &bytes, &dec.image, space, depth, alpha);
            if !alpha {
                assert!(dec.image.pixels().iter().all(|p| (p[3] - 1.0).abs() < 1e-6));
            }
        }
    }
}

#[test]
fn tiff_round_trip_all_spaces_and_compressions() {
    let img = test_image(24, 10);
    for space in SPACES {
        for compression in [
            TiffCompression::None,
            TiffCompression::Lzw,
            TiffCompression::Deflate,
        ] {
            for (depth, alpha) in [(BitDepth::Eight, true), (BitDepth::Sixteen, false)] {
                let opts = EncodeOptions::tiff(depth, alpha, compression).with_color_space(space);
                let bytes = encode(&img, &opts).unwrap();
                assert_eq!(detect_format(&bytes), Some(ImageFormat::Tiff));
                let dec = decode(&bytes).unwrap();
                assert_profile(&dec.info, space);
                assert_eq!(dec.info.has_alpha, alpha);
                check_lossless(&img, &bytes, &dec.image, space, depth, alpha);
            }
        }
    }
}

#[test]
fn linear_rec2020_needs_16_bits() {
    let img = test_image(8, 8);
    let space = ColorSpaceId::LinearRec2020;
    for (opts, alpha) in [
        (EncodeOptions::png(BitDepth::Sixteen, true), true),
        (
            EncodeOptions::tiff(BitDepth::Sixteen, false, TiffCompression::Deflate),
            false,
        ),
    ] {
        let opts = opts.with_color_space(space);
        let bytes = encode(&img, &opts).unwrap();
        let dec = decode(&bytes).unwrap();
        assert_profile(&dec.info, space);
        check_lossless(&img, &bytes, &dec.image, space, BitDepth::Sixteen, alpha);
    }
    for opts in [
        EncodeOptions::png(BitDepth::Eight, false),
        EncodeOptions::jpeg(90),
    ] {
        let err = encode(&img, &opts.with_color_space(space)).unwrap_err();
        assert!(matches!(err, EncodeError::Unsupported(_)), "{err}");
    }
}

/// Mean and maximum code difference of 8-bit RGB.
fn rgb8_error(a: &WorkingImage, b: &WorkingImage, space: ColorSpaceId) -> (f64, u32) {
    let ia = a.to_image::<u8>(space, ChannelLayout::Rgb).unwrap();
    let ib = b.to_image::<u8>(space, ChannelLayout::Rgb).unwrap();
    let diffs: Vec<u32> = ia
        .as_slice()
        .iter()
        .zip(ib.as_slice())
        .map(|(x, y)| u32::from(x.abs_diff(*y)))
        .collect();
    let mean = diffs.iter().map(|d| f64::from(*d)).sum::<f64>() / diffs.len() as f64;
    (mean, diffs.into_iter().max().unwrap())
}

#[test]
fn jpeg_round_trip_all_spaces_and_subsampling() {
    // An opaque image: JPEG has no alpha.
    let mut img = test_image(37, 21);
    for p in img.pixels_mut() {
        p[3] = 1.0;
    }
    for space in SPACES {
        for subsampling in [
            ChromaSubsampling::Yuv444,
            ChromaSubsampling::Yuv422,
            ChromaSubsampling::Yuv420,
        ] {
            let opts = EncodeOptions {
                format: OutputFormat::Jpeg {
                    quality: 95,
                    subsampling,
                },
                ..EncodeOptions::jpeg(95)
            }
            .with_color_space(space);
            let bytes = encode(&img, &opts).unwrap();
            assert_eq!(detect_format(&bytes), Some(ImageFormat::Jpeg));
            let dec = decode(&bytes).unwrap();
            assert_profile(&dec.info, space);
            assert_eq!((dec.image.width(), dec.image.height()), (37, 21));
            let (mean, max) = rgb8_error(&img, &dec.image, space);
            assert!(
                mean < 1.5 && max <= 8,
                "{space:?} {subsampling:?}: {mean} {max}"
            );
        }
    }
}

#[test]
fn jpeg_quality_100_is_close() {
    let mut img = test_image(16, 16);
    for p in img.pixels_mut() {
        p[3] = 1.0;
    }
    let opts = EncodeOptions {
        format: OutputFormat::Jpeg {
            quality: 100,
            subsampling: ChromaSubsampling::Yuv444,
        },
        ..EncodeOptions::jpeg(100)
    };
    let dec = decode(&encode(&img, &opts).unwrap()).unwrap();
    let (mean, max) = rgb8_error(&img, &dec.image, ColorSpaceId::Srgb);
    assert!(mean < 0.6 && max <= 2, "{mean} {max}");
    // Lower quality gives a smaller file.
    let small = encode(&img, &EncodeOptions::jpeg(30)).unwrap();
    assert!(small.len() < encode(&img, &opts).unwrap().len());
}

#[test]
fn one_by_one_images() {
    let img = WorkingImage::new_filled(1, 1, [0.2, 0.4, 0.6, 1.0]).unwrap();
    let all = [
        EncodeOptions::jpeg(90),
        EncodeOptions::png(BitDepth::Eight, true),
        EncodeOptions::png(BitDepth::Sixteen, false),
        EncodeOptions::tiff(BitDepth::Eight, false, TiffCompression::Lzw),
        EncodeOptions::tiff(BitDepth::Sixteen, true, TiffCompression::None),
    ];
    for opts in all {
        let dec = decode(&encode(&img, &opts).unwrap()).unwrap();
        assert_eq!((dec.image.width(), dec.image.height()), (1, 1));
        let (_, max) = rgb8_error(&img, &dec.image, ColorSpaceId::Srgb);
        assert!(max <= 2, "{opts:?}: {max}");
    }
}

#[test]
fn invalid_quality_is_an_error() {
    let img = test_image(4, 4);
    for q in [0, 101, 255] {
        let err = encode(&img, &EncodeOptions::jpeg(q)).unwrap_err();
        assert!(
            matches!(
                err,
                EncodeError::InvalidOption {
                    name: "quality",
                    ..
                }
            ),
            "{err}"
        );
    }
}

#[test]
fn premultiplied_input_gives_the_same_file() {
    let img = test_image(12, 7);
    let mut pm = img.clone();
    pm.premultiply_alpha();
    let opts = EncodeOptions::png(BitDepth::Sixteen, true);
    let a = decode(&encode(&img, &opts).unwrap()).unwrap();
    let pm_opts = EncodeOptions {
        alpha_mode: AlphaMode::Premultiplied,
        ..opts
    };
    let b = decode(&encode(&pm, &pm_opts).unwrap()).unwrap();
    assert!(max_code_diff::<u16>(&a.image, &b.image, ColorSpaceId::Srgb, ChannelLayout::Rgba) <= 2);
}

#[test]
fn dithered_output_stays_close() {
    let img = test_image(64, 8);
    let opts = EncodeOptions {
        dither: true,
        ..EncodeOptions::png(BitDepth::Eight, true)
    };
    let dec = decode(&encode(&img, &opts).unwrap()).unwrap();
    assert!(max_code_diff::<u8>(&img, &dec.image, ColorSpaceId::Srgb, ChannelLayout::Rgba) <= 1);
    // The dither changes some pixels compared with plain rounding.
    let plain = encode(&img, &EncodeOptions::png(BitDepth::Eight, true)).unwrap();
    assert_ne!(plain, encode(&img, &opts).unwrap());
}

#[test]
fn out_of_gamut_values_are_clipped() {
    // Rec. 2020 green is outside sRGB; values above 1 and NaN are clipped.
    let img = WorkingImage::from_pixels(
        3,
        1,
        vec![
            [0.0, 1.0, 0.0, 1.0],
            [5.0, 5.0, 5.0, 1.0],
            [f32::NAN, -1.0, 0.5, 1.0],
        ],
    )
    .unwrap();
    let dec = decode(&encode(&img, &EncodeOptions::png(BitDepth::Eight, false)).unwrap()).unwrap();
    let out = dec
        .image
        .to_image::<u8>(ColorSpaceId::Srgb, ChannelLayout::Rgb)
        .unwrap();
    let s = out.as_slice();
    assert_eq!(&s[0..3], &[0, 255, 0]);
    assert_eq!(&s[3..6], &[255, 255, 255]);
    assert_eq!(&s[6..8], &[0, 0]);
}

#[cfg(feature = "native")]
#[test]
fn encode_path_writes_atomically() {
    let dir = std::env::temp_dir().join(format!("expose-encode-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("out.png");
    let img = test_image(8, 8);
    let opts = EncodeOptions::png(BitDepth::Eight, false);
    expose_raw::encode_path(&img, &opts, &path).unwrap();
    // A second write replaces the file.
    expose_raw::encode_path(&img, &opts.with_color_space(ColorSpaceId::DisplayP3), &path).unwrap();
    let dec = expose_raw::decode_path(&path).unwrap();
    assert_profile(&dec.info, ColorSpaceId::DisplayP3);
    // No temporary file stays in the folder.
    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("out.png")]);
    // A path in a folder that does not exist is an I/O error.
    let bad = dir.join("missing").join("x.png");
    assert!(matches!(
        expose_raw::encode_path(&img, &opts, &bad),
        Err(EncodeError::Io(_))
    ));
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&dir).unwrap();
}

/// Encode time of a 24-megapixel image to JPEG quality 90 (4:2:0).
///
/// Run with `cargo test -p expose-raw --release --test encode -- --ignored
/// --nocapture`.
#[test]
#[ignore = "benchmark; run in release mode"]
fn benchmark_jpeg_24mp() {
    let (w, h) = (6000_u32, 4000_u32);
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = ((x ^ y) % 256) as f32 / 255.0;
            pixels.push([
                v * v,
                (x as f32 / w as f32).powi(2),
                (y as f32 / h as f32).powi(2),
                1.0,
            ]);
        }
    }
    let img = WorkingImage::from_pixels(w, h, pixels).unwrap();
    let opts = EncodeOptions::jpeg(90);
    let start = std::time::Instant::now();
    let bytes = encode(&img, &opts).unwrap();
    let total = start.elapsed();
    let start = std::time::Instant::now();
    let samples = img
        .to_image::<u8>(ColorSpaceId::Srgb, ChannelLayout::Rgb)
        .unwrap();
    let convert = start.elapsed();
    println!(
        "24 MP JPEG q90 4:2:0: total {total:?} (color conversion {convert:?}), {} bytes, {} samples",
        bytes.len(),
        samples.as_slice().len()
    );
}
