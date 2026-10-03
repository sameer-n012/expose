//! Conversion between I/O images and the working image.

use crate::buffer::io::{ChannelLayout, Image, Sample, clamp_unit};
use crate::buffer::working::{Rgba, WorkingImage};
use crate::color::{ColorSpace, ColorSpaceId, mul_vec_f32};
use crate::error::CoreResult;

impl<T: Sample> Image<T> {
    /// Converts this image to the working format.
    ///
    /// `space` is the color space of the encoded samples. The steps are:
    ///
    /// 1. Decode the transfer function of `space` (with a lookup table of
    ///    `T::LEVELS` entries).
    /// 2. Convert linear RGB to linear Rec. 2020 (D65) with a matrix. The
    ///    matrix includes a Bradford adaptation if the white point of
    ///    `space` is not D65.
    /// 3. Copy alpha as a linear unit value. Layouts with no alpha get 1.0.
    ///
    /// Gray samples are treated as R = G = B in `space`. The result has
    /// straight (not premultiplied) alpha.
    pub fn to_working(&self, space: impl Into<ColorSpace>) -> CoreResult<WorkingImage> {
        let space: ColorSpace = space.into();
        let working = ColorSpaceId::LinearRec2020.definition();
        let matrix = space.conversion_to(&working)?.to_f32();

        let lut: Vec<f32> = (0..T::LEVELS)
            .map(|i| {
                #[allow(
                    clippy::cast_precision_loss,
                    reason = "i is at most 65535, which f32 represents exactly"
                )]
                let unit = i as f32 / T::MAX_F32;
                space.transfer.decode(unit)
            })
            .collect();
        let decode = |s: T| lut[s.to_index()];

        let layout = self.layout();
        let pixels: Vec<Rgba> = self
            .as_slice()
            .chunks_exact(layout.channels())
            .map(|px| {
                let (rgb, alpha) = match layout {
                    ChannelLayout::Gray => ([decode(px[0]); 3], 1.0),
                    ChannelLayout::GrayAlpha => ([decode(px[0]); 3], px[1].to_unit()),
                    ChannelLayout::Rgb => ([decode(px[0]), decode(px[1]), decode(px[2])], 1.0),
                    ChannelLayout::Rgba => (
                        [decode(px[0]), decode(px[1]), decode(px[2])],
                        px[3].to_unit(),
                    ),
                };
                let [r, g, b] = mul_vec_f32(&matrix, rgb);
                [r, g, b, alpha]
            })
            .collect();
        WorkingImage::from_pixels(self.width(), self.height(), pixels)
    }
}

impl WorkingImage {
    /// Converts this image to an integer I/O image.
    ///
    /// `space` is the output color space. The input must have straight
    /// alpha. The steps are:
    ///
    /// 1. Convert linear Rec. 2020 to linear RGB in `space` with a matrix.
    /// 2. Clip each channel to `0.0..=1.0` (simple gamut clip). NaN gives 0.
    /// 3. For gray layouts, compute luminance with the Y row of the RGB to
    ///    XYZ matrix of `space`.
    /// 4. Encode the transfer function of `space`.
    /// 5. Scale to `0..=T::MAX` and round to the nearest value.
    ///
    /// Alpha is clipped to `0.0..=1.0` and quantized linearly. Layouts with
    /// no alpha drop it.
    pub fn to_image<T: Sample>(
        &self,
        space: impl Into<ColorSpace>,
        layout: ChannelLayout,
    ) -> CoreResult<Image<T>> {
        let space: ColorSpace = space.into();
        let working = ColorSpaceId::LinearRec2020.definition();
        let matrix = working.conversion_to(&space)?.to_f32();
        let y_row = space.to_xyz()?.to_f32()[1];
        let tf = space.transfer;
        let clip = clamp_unit;
        let encode = |v: f32| T::from_unit(tf.encode(v));

        let channels = layout.channels();
        let mut data = Vec::with_capacity(self.pixels().len() * channels);
        for p in self.pixels() {
            let rgb = mul_vec_f32(&matrix, [p[0], p[1], p[2]]).map(clip);
            let alpha = T::from_unit(p[3]);
            match layout {
                ChannelLayout::Gray | ChannelLayout::GrayAlpha => {
                    let y = clip(y_row[0] * rgb[0] + y_row[1] * rgb[1] + y_row[2] * rgb[2]);
                    data.push(encode(y));
                }
                ChannelLayout::Rgb | ChannelLayout::Rgba => {
                    data.extend(rgb.map(encode));
                }
            }
            if layout.has_alpha() {
                data.push(alpha);
            }
        }
        Image::from_vec(self.width(), self.height(), layout, data)
    }
}

#[cfg(test)]
mod tests {
    use crate::buffer::{ChannelLayout, Image8, Image16, WorkingImage};
    use crate::color::{ColorSpaceId, TransferFunction};

    #[test]
    fn round_trip_8bit_is_exact_in_every_space() {
        // All 8-bit gray levels plus some colors inside every output gamut.
        let mut data = Vec::new();
        for v in 0..=255_u8 {
            data.extend([v, v, v, 255 - v]);
        }
        data.extend([
            200, 100, 50, 255, 30, 160, 90, 128, 120, 120, 200, 0, 0, 0, 0, 7,
        ]);
        let w = u32::try_from(data.len() / 4).unwrap();
        let src = Image8::from_vec(w, 1, ChannelLayout::Rgba, data).unwrap();
        for id in ColorSpaceId::ALL {
            let working = src.to_working(id).unwrap();
            let back = working.to_image::<u8>(id, ChannelLayout::Rgba).unwrap();
            assert_eq!(back.as_slice(), src.as_slice(), "{id:?}");
        }
    }

    #[test]
    fn round_trip_16bit_within_one_code() {
        let data: Vec<u16> = (0..=65535_u16).step_by(37).collect();
        let n = data.len() - data.len() % 3;
        let data = data[..n].to_vec();
        let w = u32::try_from(n / 3).unwrap();
        let src = Image16::from_vec(w, 1, ChannelLayout::Rgb, data).unwrap();
        for id in ColorSpaceId::ALL {
            let back = src
                .to_working(id)
                .unwrap()
                .to_image::<u16>(id, ChannelLayout::Rgb)
                .unwrap();
            for (a, b) in back.as_slice().iter().zip(src.as_slice()) {
                assert!(a.abs_diff(*b) <= 1, "{id:?}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn white_and_gray_map_to_neutral_working_values() {
        let src = Image8::from_vec(2, 1, ChannelLayout::Gray, vec![255, 128]).unwrap();
        for id in ColorSpaceId::ALL {
            let img = src.to_working(id).unwrap();
            let white = img.pixel(0, 0).unwrap();
            for c in &white[..3] {
                assert!((c - 1.0).abs() < 1e-5, "{id:?}: {white:?}");
            }
            assert!((white[3] - 1.0).abs() < f32::EPSILON);
            let gray = img.pixel(1, 0).unwrap();
            let expected = id.transfer().decode(128.0 / 255.0);
            for c in &gray[..3] {
                assert!((c - expected).abs() < 1e-5, "{id:?}: {gray:?}");
            }
        }
    }

    #[test]
    fn srgb_red_converts_to_known_rec2020_value() {
        let src = Image8::from_vec(1, 1, ChannelLayout::Rgb, vec![255, 0, 0]).unwrap();
        let p = src
            .to_working(ColorSpaceId::Srgb)
            .unwrap()
            .pixel(0, 0)
            .unwrap();
        // First column of the BT.709 to BT.2020 matrix.
        for (a, b) in p[..3].iter().zip([0.6274, 0.0691, 0.0164]) {
            assert!((a - b).abs() < 1e-3, "{p:?}");
        }
    }

    #[test]
    fn output_clips_out_of_gamut_and_out_of_range() {
        let img = WorkingImage::from_pixels(
            3,
            1,
            vec![
                [0.0, 1.0, 0.0, 1.0],        // Rec. 2020 green is outside sRGB.
                [4.0, 4.0, 4.0, 2.0],        // Above white.
                [f32::NAN, -1.0, 0.5, -0.5], // NaN and negative values.
            ],
        )
        .unwrap();
        let out = img
            .to_image::<u8>(ColorSpaceId::Srgb, ChannelLayout::Rgba)
            .unwrap();
        let s = out.as_slice();
        assert_eq!(&s[0..4], &[0, 255, 0, 255]);
        assert_eq!(&s[4..8], &[255, 255, 255, 255]);
        assert_eq!(s[11], 0);
    }

    #[test]
    fn gray_output_uses_luminance() {
        let img =
            WorkingImage::from_pixels(2, 1, vec![[0.5, 0.5, 0.5, 1.0], [1.0, 0.0, 0.0, 0.25]])
                .unwrap();
        let out = img
            .to_image::<u16>(ColorSpaceId::LinearRec2020, ChannelLayout::GrayAlpha)
            .unwrap();
        let s = out.as_slice();
        assert_eq!(s[0], 32768);
        assert_eq!(s[1], 65535);
        // Rec. 2020 red has luminance 0.2627.
        let expected = (0.2627_f32 * 65535.0).round();
        assert!((f32::from(s[2]) - expected).abs() <= 2.0);
        assert_eq!(s[3], 16384);
    }

    #[test]
    fn linear_output_has_no_curve() {
        let img = WorkingImage::new_filled(1, 1, [0.25, 0.25, 0.25, 1.0]).unwrap();
        let out = img
            .to_image::<u8>(ColorSpaceId::LinearRec2020, ChannelLayout::Rgb)
            .unwrap();
        assert_eq!(out.as_slice(), &[64, 64, 64]);
        let srgb = img
            .to_image::<u8>(ColorSpaceId::Srgb, ChannelLayout::Rgb)
            .unwrap();
        let expected = (TransferFunction::Srgb.encode(0.25) * 255.0).round();
        assert!((f32::from(srgb.as_slice()[0]) - expected).abs() < f32::EPSILON);
    }
}
