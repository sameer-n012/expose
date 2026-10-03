//! Decode of standard formats (JPEG, PNG, TIFF, WebP) with the `image`
//! crate, and color space identification.

use std::io::Cursor;

use expose_core::buffer::{ChannelLayout, Image8, Image16, Rgba, WorkingImage};
use expose_core::color::{ColorSpace, ColorSpaceId, OPRGB_GAMMA, TransferFunction, mul_vec_f32};
use image::{DynamicImage, ImageDecoder};
use serde::{Deserialize, Serialize};

use crate::error::{DecodeError, DecodeResult};
use crate::format::ImageFormat;
use crate::icc::{self, ProfileKind, ToneCurve};
use crate::limits::Limits;

/// The transfer function (tone curve) of the source data.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "gamma")]
pub enum SourceTransfer {
    /// Linear data.
    Linear,
    /// The sRGB curve.
    Srgb,
    /// A pure power curve with this gamma.
    Gamma(f32),
    /// The `ProPhoto` RGB curve.
    ProPhoto,
    /// The BT.2020 curve.
    Rec2020,
}

impl SourceTransfer {
    /// Returns the `expose-core` transfer function.
    pub const fn to_core(self) -> TransferFunction {
        match self {
            Self::Linear => TransferFunction::Linear,
            Self::Srgb => TransferFunction::Srgb,
            Self::Gamma(g) => TransferFunction::Gamma(g),
            Self::ProPhoto => TransferFunction::ProPhoto,
            Self::Rec2020 => TransferFunction::Rec2020,
        }
    }

    /// Returns the standard curve of a built-in space.
    pub const fn standard(id: ColorSpaceId) -> Self {
        match id {
            ColorSpaceId::LinearRec2020 => Self::Linear,
            ColorSpaceId::Srgb | ColorSpaceId::DisplayP3 => Self::Srgb,
            ColorSpaceId::OpRgb => Self::Gamma(OPRGB_GAMMA),
            ColorSpaceId::ProPhotoRgb => Self::ProPhoto,
            ColorSpaceId::Rec2020 => Self::Rec2020,
        }
    }
}

/// Where the source color space comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorOrigin {
    /// An embedded ICC profile that was identified.
    IccProfile,
    /// The EXIF `ColorSpace` tag or interoperability index.
    Exif,
    /// No usable information: a default was assumed.
    Assumed,
}

/// The color space of the source data.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceColor {
    /// The space whose primaries and white point the data uses.
    pub space: ColorSpaceId,
    /// The tone curve of the data.
    pub transfer: SourceTransfer,
    /// Where this information comes from.
    pub origin: ColorOrigin,
}

impl SourceColor {
    /// Returns the `expose-core` color space definition.
    pub fn to_color_space(self) -> ColorSpace {
        let mut cs = self.space.definition();
        cs.transfer = self.transfer.to_core();
        cs
    }

    /// Returns `true` if the color space was assumed (no profile and no
    /// EXIF color space, or a profile that was not identified).
    pub fn is_assumed(self) -> bool {
        self.origin == ColorOrigin::Assumed
    }
}

/// Information from the standard decoder.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StandardInfo {
    pub color: SourceColor,
    pub icc_description: Option<String>,
    pub has_icc_profile: bool,
    pub bits_per_sample: u32,
    pub has_alpha: bool,
}

/// EXIF color hints for [`identify_color`].
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ExifColorHint<'a> {
    pub color_space: Option<u16>,
    pub interop_index: Option<&'a str>,
}

/// Maps an ICC tone curve to a transfer for space `id`.
fn transfer_for(id: ColorSpaceId, curve: ToneCurve) -> Option<SourceTransfer> {
    match curve {
        ToneCurve::Linear => Some(SourceTransfer::Linear),
        ToneCurve::Srgb => Some(SourceTransfer::Srgb),
        ToneCurve::Rec2020 => Some(SourceTransfer::Rec2020),
        ToneCurve::ProPhoto => Some(SourceTransfer::ProPhoto),
        // ICC stores gamma as u8.8 fixed point: 2.2 is 563/256 (opRGB) and
        // 1.8 is 461/256. The ProPhoto curve has a short linear part near
        // black, and a gamma 1.8 profile is its usual approximation.
        ToneCurve::Gamma(g) if id == ColorSpaceId::ProPhotoRgb && (g - 1.8).abs() < 0.01 => {
            Some(SourceTransfer::ProPhoto)
        }
        ToneCurve::Gamma(g) => Some(SourceTransfer::Gamma(g)),
        ToneCurve::Other => None,
    }
}

/// Finds the color space of the source data.
///
/// Order: an identified ICC profile; then the EXIF color space (1 = sRGB)
/// or interoperability index `R03` (opRGB); then a default. The default is
/// sRGB for integer data and linear sRGB primaries for float data. A
/// default is marked [`ColorOrigin::Assumed`].
pub(crate) fn identify_color(
    icc: Option<&[u8]>,
    exif: ExifColorHint<'_>,
    float: bool,
) -> (SourceColor, Option<String>) {
    let assumed = SourceColor {
        space: ColorSpaceId::Srgb,
        transfer: if float {
            SourceTransfer::Linear
        } else {
            SourceTransfer::Srgb
        },
        origin: ColorOrigin::Assumed,
    };
    if let Some(info) = icc.and_then(icc::identify) {
        let description = info.description.clone();
        let space = match info.kind {
            ProfileKind::Rgb => info.primaries,
            // A gray value is neutral: only the white point (D65 for the
            // sRGB definition) and the curve matter.
            ProfileKind::Gray => Some(ColorSpaceId::Srgb),
            ProfileKind::Other => None,
        };
        let color = space
            .and_then(|id| {
                let transfer = transfer_for(id, info.curve?)?;
                Some(SourceColor {
                    space: id,
                    transfer,
                    origin: ColorOrigin::IccProfile,
                })
            })
            .unwrap_or(assumed);
        return (color, description);
    }
    let from_exif = |id: ColorSpaceId| SourceColor {
        space: id,
        transfer: SourceTransfer::standard(id),
        origin: ColorOrigin::Exif,
    };
    if exif.interop_index == Some("R03") {
        return (from_exif(ColorSpaceId::OpRgb), None);
    }
    if exif.color_space == Some(1) && !float {
        return (from_exif(ColorSpaceId::Srgb), None);
    }
    (assumed, None)
}

/// Decodes a standard format to the working image.
pub(crate) fn decode_standard(
    data: &[u8],
    format: ImageFormat,
    limits: Limits,
    exif: ExifColorHint<'_>,
) -> DecodeResult<(WorkingImage, StandardInfo)> {
    let f = match format {
        ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        ImageFormat::Png => image::ImageFormat::Png,
        ImageFormat::Tiff => image::ImageFormat::Tiff,
        ImageFormat::WebP => image::ImageFormat::WebP,
        ImageFormat::Heif | ImageFormat::Avif => {
            return Err(DecodeError::UnsupportedFormat {
                format,
                reason: "no mature pure-Rust HEIF/AVIF decoder is available yet",
            });
        }
        ImageFormat::Raw(_) => {
            return Err(DecodeError::invalid("RAW data in the standard decoder"));
        }
    };
    let mut reader = image::ImageReader::with_format(Cursor::new(data), f);
    let mut il = image::Limits::default();
    il.max_alloc = Some(limits.max_alloc_bytes());
    reader.limits(il);
    let mut decoder = reader.into_decoder()?;
    let (w, h) = decoder.dimensions();
    limits.check(u64::from(w), u64::from(h))?;
    let mut icc = decoder.icc_profile()?;
    if icc.is_none() && format == ImageFormat::Tiff {
        // The TIFF decoder of `image` limits tag data to a size derived
        // from the pixel buffer size, so it does not return the profile of
        // a small image (less than about 13 KB of pixels for a 0.5 KB
        // profile). Read the tag with the local bounds-checked reader.
        icc = crate::tiff::TiffReader::new(data, &[])
            .and_then(|r| r.icc_profile())
            .map(<[u8]>::to_vec);
    }
    let img = DynamicImage::from_decoder(decoder)?;

    let float = matches!(
        img,
        DynamicImage::ImageRgb32F(_) | DynamicImage::ImageRgba32F(_)
    );
    let (color, icc_description) = identify_color(icc.as_deref(), exif, float);
    let space = color.to_color_space();
    let has_alpha = img.color().has_alpha();
    let bits_per_sample =
        u32::from(img.color().bits_per_pixel()) / u32::from(img.color().channel_count()).max(1);

    let working = match img {
        DynamicImage::ImageLuma8(b) => {
            Image8::from_vec(w, h, ChannelLayout::Gray, b.into_raw())?.to_working(space)?
        }
        DynamicImage::ImageLumaA8(b) => {
            Image8::from_vec(w, h, ChannelLayout::GrayAlpha, b.into_raw())?.to_working(space)?
        }
        DynamicImage::ImageRgb8(b) => {
            Image8::from_vec(w, h, ChannelLayout::Rgb, b.into_raw())?.to_working(space)?
        }
        DynamicImage::ImageRgba8(b) => {
            Image8::from_vec(w, h, ChannelLayout::Rgba, b.into_raw())?.to_working(space)?
        }
        DynamicImage::ImageLuma16(b) => {
            Image16::from_vec(w, h, ChannelLayout::Gray, b.into_raw())?.to_working(space)?
        }
        DynamicImage::ImageLumaA16(b) => {
            Image16::from_vec(w, h, ChannelLayout::GrayAlpha, b.into_raw())?.to_working(space)?
        }
        DynamicImage::ImageRgb16(b) => {
            Image16::from_vec(w, h, ChannelLayout::Rgb, b.into_raw())?.to_working(space)?
        }
        DynamicImage::ImageRgba16(b) => {
            Image16::from_vec(w, h, ChannelLayout::Rgba, b.into_raw())?.to_working(space)?
        }
        DynamicImage::ImageRgb32F(b) => float_to_working(w, h, 3, &b.into_raw(), space)?,
        DynamicImage::ImageRgba32F(b) => float_to_working(w, h, 4, &b.into_raw(), space)?,
        other => {
            return Err(DecodeError::unsupported(format!(
                "pixel type {:?}",
                other.color()
            )));
        }
    };
    Ok((
        working,
        StandardInfo {
            color,
            icc_description,
            has_icc_profile: icc.is_some(),
            bits_per_sample,
            has_alpha,
        },
    ))
}

/// Converts float RGB or RGBA samples to the working image.
///
/// `expose-core` has no float I/O image type, so this conversion is local:
/// decode the transfer function, then apply the matrix to linear Rec. 2020.
/// Values outside `0.0..=1.0` are kept.
fn float_to_working(
    w: u32,
    h: u32,
    channels: usize,
    samples: &[f32],
    space: ColorSpace,
) -> DecodeResult<WorkingImage> {
    let m = space
        .conversion_to(&ColorSpaceId::LinearRec2020.definition())?
        .to_f32();
    let tf = space.transfer;
    let pixels: Vec<Rgba> = samples
        .chunks_exact(channels)
        .map(|p| {
            let [r, g, b] = mul_vec_f32(&m, [tf.decode(p[0]), tf.decode(p[1]), tf.decode(p[2])]);
            let a = if channels == 4 { p[3] } else { 1.0 };
            [r, g, b, a]
        })
        .collect();
    Ok(WorkingImage::from_pixels(w, h, pixels)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icc::testing::rgb_profile;

    #[test]
    fn color_identification_order() {
        let p3 = rgb_profile(ColorSpaceId::DisplayP3, 2.2, "Display P3 test");
        let (c, d) = identify_color(Some(&p3), ExifColorHint::default(), false);
        assert_eq!(c.space, ColorSpaceId::DisplayP3);
        assert_eq!(c.origin, ColorOrigin::IccProfile);
        assert_eq!(d.as_deref(), Some("Display P3 test"));

        let op = rgb_profile(ColorSpaceId::OpRgb, OPRGB_GAMMA, "op");
        let (c, _) = identify_color(Some(&op), ExifColorHint::default(), false);
        assert_eq!(c.space, ColorSpaceId::OpRgb);
        assert!(matches!(c.transfer, SourceTransfer::Gamma(g) if (g - OPRGB_GAMMA).abs() < 0.01));

        let pp = rgb_profile(ColorSpaceId::ProPhotoRgb, 1.8, "pp");
        let (c, _) = identify_color(Some(&pp), ExifColorHint::default(), false);
        assert_eq!(
            (c.space, c.transfer),
            (ColorSpaceId::ProPhotoRgb, SourceTransfer::ProPhoto)
        );

        let exif = ExifColorHint {
            color_space: Some(1),
            interop_index: None,
        };
        let (c, _) = identify_color(None, exif, false);
        assert_eq!((c.space, c.origin), (ColorSpaceId::Srgb, ColorOrigin::Exif));
        let r03 = ExifColorHint {
            color_space: Some(65535),
            interop_index: Some("R03"),
        };
        assert_eq!(
            identify_color(None, r03, false).0.space,
            ColorSpaceId::OpRgb
        );

        let (c, _) = identify_color(None, ExifColorHint::default(), false);
        assert!(c.is_assumed());
        let (c, _) = identify_color(None, ExifColorHint::default(), true);
        assert!(c.is_assumed());
        assert_eq!(c.transfer, SourceTransfer::Linear);

        // An unknown profile is not silently trusted.
        let (c, d) = identify_color(Some(b"garbage"), ExifColorHint::default(), false);
        assert!(c.is_assumed());
        assert!(d.is_none());
    }
}
