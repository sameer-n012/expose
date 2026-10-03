//! Image encode for export: JPEG, PNG, and TIFF with an embedded ICC
//! profile.
//!
//! # Steps
//!
//! [`encode`] does these steps:
//!
//! 1. If the input has premultiplied alpha, divide RGB by alpha.
//! 2. Convert from the working space (linear Rec. 2020) to the output color
//!    space with [`WorkingImage::to_image`]. Values outside the output gamut
//!    or outside `0.0..=1.0` are clipped per channel. This is the P1 gamut
//!    handling (ADR 0002); gamut mapping is a later task.
//! 3. Quantize to 8 or 16 bits. For 8 bits, an optional ordered dither
//!    (8x8 Bayer matrix) can replace plain rounding.
//! 4. Encode the file, with the ICC profile of the output space (see
//!    [`icc_profile`]): JPEG APP2 `ICC_PROFILE`, PNG `iCCP`, TIFF tag 34675.
//!
//! # Not done here
//!
//! - Resize: call [`expose_core::buffer::resize`] before `encode`.
//! - Orientation: the pixels are written as they are. Give pixels that are
//!   already in display orientation. No EXIF block is written, so readers
//!   use the default orientation (normal).
//! - Metadata copy (EXIF, XMP, IPTC) is a later task.

mod icc_profile;
mod jpeg;

use std::borrow::Cow;
use std::io::Cursor;

use expose_core::buffer::{AlphaMode, ChannelLayout, Image, Image8, Sample, WorkingImage};
use expose_core::color::ColorSpaceId;
use image::ImageEncoder;
use serde::{Deserialize, Serialize};

use crate::error::{EncodeError, EncodeResult};

pub use icc_profile::{icc_description, icc_profile};

/// Chroma subsampling of JPEG output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ChromaSubsampling {
    /// No subsampling: full color resolution.
    #[serde(rename = "444")]
    Yuv444,
    /// Half horizontal color resolution.
    #[serde(rename = "422")]
    Yuv422,
    /// Half horizontal and half vertical color resolution. Smallest files.
    #[default]
    #[serde(rename = "420")]
    Yuv420,
}

/// Bits per sample of PNG and TIFF output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum BitDepth {
    /// 8 bits per sample.
    #[default]
    #[serde(rename = "8")]
    Eight,
    /// 16 bits per sample.
    #[serde(rename = "16")]
    Sixteen,
}

/// Compression of TIFF output. All methods are lossless.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TiffCompression {
    /// No compression.
    None,
    /// LZW with horizontal prediction.
    Lzw,
    /// Deflate (zlib) with horizontal prediction.
    #[default]
    Deflate,
}

/// The output file format and its format options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum OutputFormat {
    /// Baseline JPEG, 8 bits, no alpha.
    Jpeg {
        /// Quality, 1 (smallest file) to 100 (best quality).
        quality: u8,
        /// Chroma subsampling.
        subsampling: ChromaSubsampling,
    },
    /// PNG.
    Png {
        /// Bits per sample.
        bit_depth: BitDepth,
        /// If `true`, the file has an alpha channel.
        alpha: bool,
    },
    /// Baseline TIFF (RGB, one image).
    Tiff {
        /// Bits per sample.
        bit_depth: BitDepth,
        /// If `true`, the file has an alpha channel (unassociated).
        alpha: bool,
        /// Compression method.
        compression: TiffCompression,
    },
}

impl OutputFormat {
    /// Returns the usual file extension (without a dot).
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Jpeg { .. } => "jpg",
            Self::Png { .. } => "png",
            Self::Tiff { .. } => "tif",
        }
    }

    const fn bit_depth(self) -> BitDepth {
        match self {
            Self::Jpeg { .. } => BitDepth::Eight,
            Self::Png { bit_depth, .. } | Self::Tiff { bit_depth, .. } => bit_depth,
        }
    }

    const fn layout(self) -> ChannelLayout {
        match self {
            Self::Png { alpha: true, .. } | Self::Tiff { alpha: true, .. } => ChannelLayout::Rgba,
            _ => ChannelLayout::Rgb,
        }
    }
}

/// Options of [`encode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodeOptions {
    /// The file format and its options.
    pub format: OutputFormat,
    /// The output color space. [`ColorSpaceId::LinearRec2020`] needs 16
    /// bits (8-bit linear data shows visible banding).
    pub color_space: ColorSpaceId,
    /// How the alpha of the input image is stored.
    pub alpha_mode: AlphaMode,
    /// If `true`, 8-bit output uses an ordered dither (8x8 Bayer matrix)
    /// instead of rounding. This removes banding in smooth gradients. It
    /// has no effect on 16-bit output. Off by default.
    pub dither: bool,
}

impl EncodeOptions {
    /// JPEG options: sRGB, 4:2:0, no dither.
    pub const fn jpeg(quality: u8) -> Self {
        Self {
            format: OutputFormat::Jpeg {
                quality,
                subsampling: ChromaSubsampling::Yuv420,
            },
            color_space: ColorSpaceId::Srgb,
            alpha_mode: AlphaMode::Straight,
            dither: false,
        }
    }

    /// PNG options: sRGB, no dither.
    pub const fn png(bit_depth: BitDepth, alpha: bool) -> Self {
        Self {
            format: OutputFormat::Png { bit_depth, alpha },
            color_space: ColorSpaceId::Srgb,
            alpha_mode: AlphaMode::Straight,
            dither: false,
        }
    }

    /// TIFF options: sRGB, no dither.
    pub const fn tiff(bit_depth: BitDepth, alpha: bool, compression: TiffCompression) -> Self {
        Self {
            format: OutputFormat::Tiff {
                bit_depth,
                alpha,
                compression,
            },
            color_space: ColorSpaceId::Srgb,
            alpha_mode: AlphaMode::Straight,
            dither: false,
        }
    }

    /// Returns a copy with another output color space.
    #[must_use]
    pub const fn with_color_space(mut self, color_space: ColorSpaceId) -> Self {
        self.color_space = color_space;
        self
    }

    /// Checks the options before any pixel work.
    fn validate(self) -> EncodeResult<()> {
        if let OutputFormat::Jpeg { quality, .. } = self.format {
            if !(1..=100).contains(&quality) {
                return Err(EncodeError::InvalidOption {
                    name: "quality",
                    reason: format!("{quality} is outside the range 1..=100"),
                });
            }
        }
        if self.color_space == ColorSpaceId::LinearRec2020
            && self.format.bit_depth() == BitDepth::Eight
        {
            return Err(EncodeError::Unsupported(
                "linear Rec. 2020 output needs 16 bits per sample (PNG or TIFF)",
            ));
        }
        Ok(())
    }
}

/// The 8x8 Bayer matrix (values 0..64).
const BAYER8: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

/// Quantizes 16-bit samples to 8 bits with an ordered dither. Alpha is
/// rounded, not dithered (dithered alpha gives visible edge noise).
fn dither_to_8(src: &Image<u16>) -> EncodeResult<Image8> {
    let channels = src.layout().channels();
    let alpha_index = src.layout().has_alpha().then_some(channels - 1);
    let mut data = Vec::with_capacity(src.as_slice().len());
    for (y, row) in src.rows().enumerate() {
        let bayer = &BAYER8[y % 8];
        for (x, px) in row.chunks_exact(channels).enumerate() {
            // Threshold in 0..1 (exclusive), centered in each Bayer cell,
            // as a fraction of 65535: (2b + 1) / 128.
            let threshold = (u32::from(bayer[x % 8]) * 2 + 1) * 65_535 / 128;
            for (c, &v) in px.iter().enumerate() {
                let scaled = u32::from(v) * 255;
                let q = if Some(c) == alpha_index {
                    (scaled + 65_535 / 2) / 65_535
                } else {
                    (scaled + threshold) / 65_535
                };
                // scaled + threshold < 256 * 65535, so q <= 255.
                data.push(u8::try_from(q).unwrap_or(u8::MAX));
            }
        }
    }
    Ok(Image8::from_vec(
        src.width(),
        src.height(),
        src.layout(),
        data,
    )?)
}

/// Converts the working image to output samples.
fn to_samples<T: Sample>(
    image: &WorkingImage,
    space: ColorSpaceId,
    layout: ChannelLayout,
) -> EncodeResult<Image<T>> {
    Ok(image.to_image::<T>(space, layout)?)
}

/// Encodes `image` to a file in memory.
///
/// See the module documentation for the steps. The output has the ICC
/// profile of `options.color_space`.
///
/// Errors:
/// - [`EncodeError::InvalidOption`]: JPEG quality outside 1..=100.
/// - [`EncodeError::Unsupported`]: linear output with 8 bits.
/// - [`EncodeError::TooLarge`]: a JPEG side larger than 65 535.
/// - [`EncodeError::Image`] / [`EncodeError::Tiff`]: encoder errors.
pub fn encode(image: &WorkingImage, options: &EncodeOptions) -> EncodeResult<Vec<u8>> {
    options.validate()?;
    if let OutputFormat::Jpeg { .. } = options.format {
        // Check before the color conversion of a large image.
        if image.width() > jpeg::MAX_SIDE || image.height() > jpeg::MAX_SIDE {
            return Err(EncodeError::TooLarge {
                width: image.width(),
                height: image.height(),
                format: "JPEG",
                max: jpeg::MAX_SIDE,
            });
        }
    }
    let straight: Cow<'_, WorkingImage> = match options.alpha_mode {
        AlphaMode::Straight => Cow::Borrowed(image),
        AlphaMode::Premultiplied => {
            let mut copy = image.clone();
            copy.unpremultiply_alpha();
            Cow::Owned(copy)
        }
    };
    let icc = icc_profile(options.color_space)?;
    let space = options.color_space;
    let layout = options.format.layout();

    match options.format.bit_depth() {
        BitDepth::Eight => {
            let samples = if options.dither {
                dither_to_8(&to_samples::<u16>(&straight, space, layout)?)?
            } else {
                to_samples::<u8>(&straight, space, layout)?
            };
            match options.format {
                OutputFormat::Jpeg {
                    quality,
                    subsampling,
                } => jpeg::encode_jpeg(&samples, quality, subsampling, &icc),
                OutputFormat::Png { .. } => encode_png(&samples, icc),
                OutputFormat::Tiff { compression, .. } => {
                    encode_tiff_8(&samples, compression, &icc)
                }
            }
        }
        BitDepth::Sixteen => {
            let samples = to_samples::<u16>(&straight, space, layout)?;
            match options.format {
                OutputFormat::Png { .. } => encode_png(&samples, icc),
                OutputFormat::Tiff { compression, .. } => {
                    encode_tiff_16(&samples, compression, &icc)
                }
                // validate() and bit_depth() make JPEG always 8-bit.
                OutputFormat::Jpeg { .. } => Err(EncodeError::Unsupported("16-bit JPEG")),
            }
        }
    }
}

fn encode_png<T: Sample>(samples: &Image<T>, icc: Vec<u8>) -> EncodeResult<Vec<u8>> {
    use image::ExtendedColorType as C;
    let color = match (samples.layout(), size_of::<T>()) {
        (ChannelLayout::Rgb, 1) => C::Rgb8,
        (ChannelLayout::Rgba, 1) => C::Rgba8,
        (ChannelLayout::Rgb, 2) => C::Rgb16,
        (ChannelLayout::Rgba, 2) => C::Rgba16,
        _ => return Err(EncodeError::Unsupported("PNG layout")),
    };
    let mut out = Vec::new();
    let mut encoder = image::codecs::png::PngEncoder::new_with_quality(
        &mut out,
        image::codecs::png::CompressionType::Default,
        image::codecs::png::FilterType::Adaptive,
    );
    encoder
        .set_icc_profile(icc)
        .map_err(|e| EncodeError::Image(image::ImageError::Unsupported(e)))?;
    // `as_bytes` gives native byte order, which the PNG encoder expects for
    // 16-bit data. The buffer length agrees with the size by construction
    // of `Image`, so `write_image` does not panic.
    encoder.write_image(samples.as_bytes(), samples.width(), samples.height(), color)?;
    Ok(out)
}

fn tiff_encoder(
    out: &mut Cursor<Vec<u8>>,
    compression: TiffCompression,
) -> EncodeResult<tiff::encoder::TiffEncoder<&mut Cursor<Vec<u8>>>> {
    use tiff::encoder::{Compression, DeflateLevel, Predictor};
    let encoder = tiff::encoder::TiffEncoder::new(out)?;
    Ok(match compression {
        TiffCompression::None => encoder,
        TiffCompression::Lzw => encoder
            .with_compression(Compression::Lzw)
            .with_predictor(Predictor::Horizontal),
        TiffCompression::Deflate => encoder
            .with_compression(Compression::Deflate(DeflateLevel::Balanced))
            .with_predictor(Predictor::Horizontal),
    })
}

/// Writes one TIFF image with color type `C`, the ICC tag, and the
/// `ExtraSamples` tag for alpha.
fn write_tiff_image<C>(
    compression: TiffCompression,
    width: u32,
    height: u32,
    alpha: bool,
    data: &[C::Inner],
    icc: &[u8],
) -> EncodeResult<Vec<u8>>
where
    C: tiff::encoder::colortype::ColorType,
    [C::Inner]: tiff::encoder::TiffValue,
{
    use tiff::tags::Tag;
    let mut out = Cursor::new(Vec::new());
    {
        let mut encoder = tiff_encoder(&mut out, compression)?;
        let mut img = encoder.new_image::<C>(width, height)?;
        img.encoder().write_tag(Tag::IccProfile, icc)?;
        if alpha {
            // ExtraSamples = 2: unassociated (straight) alpha.
            img.encoder().write_tag(Tag::ExtraSamples, &[2_u16][..])?;
        }
        img.write_data(data)?;
    }
    Ok(out.into_inner())
}

fn encode_tiff_8(
    samples: &Image8,
    compression: TiffCompression,
    icc: &[u8],
) -> EncodeResult<Vec<u8>> {
    use tiff::encoder::colortype::{RGB8, RGBA8};
    let (w, h, d) = (samples.width(), samples.height(), samples.as_slice());
    match samples.layout() {
        ChannelLayout::Rgb => write_tiff_image::<RGB8>(compression, w, h, false, d, icc),
        ChannelLayout::Rgba => write_tiff_image::<RGBA8>(compression, w, h, true, d, icc),
        _ => Err(EncodeError::Unsupported("TIFF layout")),
    }
}

fn encode_tiff_16(
    samples: &Image<u16>,
    compression: TiffCompression,
    icc: &[u8],
) -> EncodeResult<Vec<u8>> {
    use tiff::encoder::colortype::{RGB16, RGBA16};
    let (w, h, d) = (samples.width(), samples.height(), samples.as_slice());
    match samples.layout() {
        ChannelLayout::Rgb => write_tiff_image::<RGB16>(compression, w, h, false, d, icc),
        ChannelLayout::Rgba => write_tiff_image::<RGBA16>(compression, w, h, true, d, icc),
        _ => Err(EncodeError::Unsupported("TIFF layout")),
    }
}

/// Encodes `image` and writes it to `path` (feature `native`).
///
/// The write is atomic: the function writes a temporary file in the same
/// folder, flushes it to the disk, and then renames it to `path`. An
/// existing file at `path` is replaced. If a step fails, the function
/// removes the temporary file and `path` does not change.
#[cfg(feature = "native")]
pub fn encode_path(
    image: &WorkingImage,
    options: &EncodeOptions,
    path: impl AsRef<std::path::Path>,
) -> EncodeResult<()> {
    let bytes = encode(image, options)?;
    write_atomic(path.as_ref(), &bytes)
}

#[cfg(feature = "native")]
fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> EncodeResult<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let invalid = || {
        EncodeError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("path has no file name: {}", path.display()),
        ))
    };
    let name = path.file_name().ok_or_else(invalid)?;
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => std::path::Path::new("."),
    };
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(name);
    temp_name.push(format!(".{}.{n}.tmp", std::process::id()));
    let temp = dir.join(temp_name);

    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)
    })();
    if let Err(e) = result {
        // Remove the partial file. If this also fails, report the first
        // error: it is the cause.
        let _ = std::fs::remove_file(&temp);
        return Err(EncodeError::Io(e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn dither_keeps_mean_and_range() {
        // A flat 16-bit value between two 8-bit codes: the dithered output
        // mixes the two codes with the right mean.
        // 33024 / 65535 * 255 = 128.498: between the codes 128 and 129.
        let v16 = 33_024_u16;
        let data = vec![v16; 64 * 64 * 3];
        let src = Image::<u16>::from_vec(64, 64, ChannelLayout::Rgb, data).unwrap();
        let out = dither_to_8(&src).unwrap();
        let s = out.as_slice();
        assert!(s.iter().all(|v| *v == 128 || *v == 129), "two codes only");
        #[allow(clippy::cast_precision_loss, reason = "small test sizes")]
        let mean = s.iter().map(|v| f64::from(*v)).sum::<f64>() / s.len() as f64;
        let expected = f64::from(v16) * 255.0 / 65535.0;
        assert!((mean - expected).abs() < 0.02, "{mean} vs {expected}");
        // The extremes stay exact.
        let ends =
            Image::<u16>::from_vec(2, 1, ChannelLayout::Rgb, vec![0, 0, 0, 65535, 65535, 65535])
                .unwrap();
        assert_eq!(
            dither_to_8(&ends).unwrap().as_slice(),
            &[0, 0, 0, 255, 255, 255]
        );
    }

    #[test]
    fn option_validation() {
        assert!(EncodeOptions::jpeg(0).validate().is_err());
        assert!(EncodeOptions::jpeg(101).validate().is_err());
        assert!(EncodeOptions::jpeg(1).validate().is_ok());
        assert!(EncodeOptions::jpeg(100).validate().is_ok());
        let lin8 = EncodeOptions::png(BitDepth::Eight, false)
            .with_color_space(ColorSpaceId::LinearRec2020);
        assert!(matches!(lin8.validate(), Err(EncodeError::Unsupported(_))));
        let lin16 = EncodeOptions::png(BitDepth::Sixteen, false)
            .with_color_space(ColorSpaceId::LinearRec2020);
        assert!(lin16.validate().is_ok());
    }

    #[test]
    fn options_serde_names() {
        let o = EncodeOptions::jpeg(90).with_color_space(ColorSpaceId::DisplayP3);
        let s = serde_json::to_string(&o).unwrap();
        assert_eq!(
            s,
            r#"{"format":{"kind":"jpeg","quality":90,"subsampling":"420"},"colorSpace":"display_p3","alphaMode":"straight","dither":false}"#
        );
        let back: EncodeOptions = serde_json::from_str(&s).unwrap();
        assert_eq!(back, o);
    }
}
