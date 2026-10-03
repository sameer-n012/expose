//! Conversion between I/O images and the working image.

use crate::buffer::encode_lut::{Encoder, LinearEncoder, LutEncoder};
use crate::buffer::io::{ChannelLayout, Image, Sample};
use crate::buffer::working::{Rgba, WorkingImage};
use crate::color::{ColorSpace, ColorSpaceId, TransferFunction, mul_vec_f32};
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
    ///
    /// Step 4 and 5 use a lookup table (see `encode_lut.rs`) in place of a
    /// `powf` call per sample. The table gives the same code as the direct
    /// formula, except for rare inputs within some ulps of a code boundary
    /// (at most one code of difference). The linear transfer function does
    /// not use a table.
    ///
    /// The matrix step uses separate `f32` multiplies and adds (no fused
    /// multiply-add), so the result does not change between targets.
    pub fn to_image<T: Sample>(
        &self,
        space: impl Into<ColorSpace>,
        layout: ChannelLayout,
    ) -> CoreResult<Image<T>> {
        let space: ColorSpace = space.into();
        let working = ColorSpaceId::LinearRec2020.definition();
        let matrix = working.conversion_to(&space)?.to_f32();
        let y_row = space.to_xyz()?.to_f32()[1];
        let ctx = EncodeContext { matrix, y_row };

        // `pixels().len() * 16` bytes are already allocated, so this
        // product (at most 4 channels) cannot overflow.
        let mut data = vec![T::default(); self.pixels().len() * layout.channels()];
        if space.transfer == TransferFunction::Linear {
            ctx.run(self.pixels(), &mut data, layout, &LinearEncoder);
        } else {
            // Match on the table form once, outside the pixel loop.
            match LutEncoder::<T>::new(space.transfer) {
                LutEncoder::Step(lut) => ctx.run(self.pixels(), &mut data, layout, &lut),
                LutEncoder::Interp(lut) => ctx.run(self.pixels(), &mut data, layout, &lut),
            }
        }
        Image::from_vec(self.width(), self.height(), layout, data)
    }
}

/// Clips to `+0.0..=1.0`. NaN, negative values, and `-0.0` give `+0.0`.
///
/// The encoders need a non-negative sign bit. For all other inputs this is
/// the same as `clamp_unit`.
#[inline]
fn clip_positive(v: f32) -> f32 {
    if v > 0.0 { v.min(1.0) } else { 0.0 }
}

/// The per-call constants of [`WorkingImage::to_image`].
struct EncodeContext {
    /// Linear Rec. 2020 to linear output RGB.
    matrix: [[f32; 3]; 3],
    /// The Y (luminance) row of the output RGB to XYZ matrix.
    y_row: [f32; 3],
}

impl EncodeContext {
    /// Selects the loop for `layout` once, outside the pixel loop.
    fn run<T: Sample, E: Encoder<T>>(
        &self,
        src: &[Rgba],
        out: &mut [T],
        layout: ChannelLayout,
        enc: &E,
    ) {
        match layout {
            ChannelLayout::Gray => self.run_layout::<T, E, 1, true, false>(src, out, enc),
            ChannelLayout::GrayAlpha => self.run_layout::<T, E, 2, true, true>(src, out, enc),
            ChannelLayout::Rgb => self.run_layout::<T, E, 3, false, false>(src, out, enc),
            ChannelLayout::Rgba => self.run_layout::<T, E, 4, false, true>(src, out, enc),
        }
    }

    /// The pixel loop for one layout. `C` is the channel count: 1 or 3
    /// color channels (`GRAY`), plus 1 if `ALPHA`.
    #[inline]
    fn run_layout<T: Sample, E: Encoder<T>, const C: usize, const GRAY: bool, const ALPHA: bool>(
        &self,
        src: &[Rgba],
        out: &mut [T],
        enc: &E,
    ) {
        debug_assert_eq!(C, usize::from(!GRAY) * 2 + 1 + usize::from(ALPHA));
        debug_assert_eq!(out.len(), src.len() * C);
        for (p, o) in src.iter().zip(out.chunks_exact_mut(C)) {
            let rgb = mul_vec_f32(&self.matrix, [p[0], p[1], p[2]]).map(clip_positive);
            if GRAY {
                let y = &self.y_row;
                o[0] = enc.encode(clip_positive(y[0] * rgb[0] + y[1] * rgb[1] + y[2] * rgb[2]));
            } else {
                o[0] = enc.encode(rgb[0]);
                o[1] = enc.encode(rgb[1]);
                o[2] = enc.encode(rgb[2]);
            }
            if ALPHA {
                o[C - 1] = T::from_unit(p[3]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::buffer::{ChannelLayout, Image8, Image16, Sample, WorkingImage};
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

    /// The direct per-sample formula (the code before the lookup table).
    fn reference<T: Sample>(img: &WorkingImage, id: ColorSpaceId, layout: ChannelLayout) -> Vec<T> {
        use crate::buffer::io::clamp_unit;
        use crate::color::mul_vec_f32;
        let space = id.definition();
        let working = ColorSpaceId::LinearRec2020.definition();
        let m = working.conversion_to(&space).unwrap().to_f32();
        let y = space.to_xyz().unwrap().to_f32()[1];
        let enc = |v: f32| T::from_unit(space.transfer.encode(v));
        let mut out = Vec::new();
        for p in img.pixels() {
            let rgb = mul_vec_f32(&m, [p[0], p[1], p[2]]).map(clamp_unit);
            match layout {
                ChannelLayout::Gray | ChannelLayout::GrayAlpha => {
                    out.push(enc(clamp_unit(
                        y[0] * rgb[0] + y[1] * rgb[1] + y[2] * rgb[2],
                    )));
                }
                ChannelLayout::Rgb | ChannelLayout::Rgba => out.extend(rgb.map(enc)),
            }
            if layout.has_alpha() {
                out.push(T::from_unit(p[3]));
            }
        }
        out
    }

    #[test]
    fn matches_direct_formula_for_all_layouts_spaces_and_edge_values() {
        let specials = [
            0.0,
            -0.0,
            1.0,
            0.5,
            0.18,
            0.003_130_8,
            f32::NAN,
            -1.0,
            -1e-30,
            2.0,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::from_bits(1),
            f32::MIN_POSITIVE,
            1e-7,
            0.999_999_9,
            1.000_000_1,
        ];
        let mut pixels = Vec::new();
        for (i, &a) in specials.iter().enumerate() {
            for (j, &b) in specials.iter().enumerate() {
                let alpha = specials[(i + j) % specials.len()];
                pixels.push([a, b, specials[(i * 7 + j * 3) % specials.len()], alpha]);
            }
        }
        // A dense ramp of neutral and colored values.
        for i in 0..=4096_u16 {
            let t = f32::from(i) / 4096.0;
            pixels.push([t, t, t, t]);
            pixels.push([t, 1.0 - t, t * t, 1.0]);
        }
        let w = u32::try_from(pixels.len()).unwrap();
        let img = WorkingImage::from_pixels(w, 1, pixels).unwrap();
        let layouts = [
            ChannelLayout::Gray,
            ChannelLayout::GrayAlpha,
            ChannelLayout::Rgb,
            ChannelLayout::Rgba,
        ];
        for id in ColorSpaceId::ALL {
            for layout in layouts {
                let fast = img.to_image::<u8>(id, layout).unwrap();
                assert_eq!(fast.layout(), layout);
                assert_eq!(
                    fast.as_slice(),
                    reference::<u8>(&img, id, layout),
                    "{id:?} {layout:?}"
                );
                let fast = img.to_image::<u16>(id, layout).unwrap();
                assert_eq!(
                    fast.as_slice(),
                    reference::<u16>(&img, id, layout),
                    "{id:?} {layout:?}"
                );
            }
        }
    }

    /// Timing of `to_image` on a 24 MP image. Run in release mode:
    /// `cargo test -p expose-core --release -- --ignored to_image_24mp_timing --nocapture`.
    #[test]
    #[ignore = "timing benchmark; run manually in release mode"]
    fn to_image_24mp_timing() {
        let (w, h) = (6000_u32, 4000_u32);
        let pixels: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                // A deterministic pattern that covers the full 0..1 range.
                #[allow(clippy::cast_precision_loss)]
                let t = (i % 9973) as f32 / 9972.0;
                [t, (1.0 - t) * 0.8, t * t, 1.0]
            })
            .collect();
        let ramp = WorkingImage::from_pixels(w, h, pixels).unwrap();
        // Uniform noise: the worst case for branch prediction.
        let mut state = 0x9E37_79B9_u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            #[allow(clippy::cast_precision_loss)]
            let v = (state >> 8) as f32 / 16_777_216.0;
            v
        };
        let noise_px: Vec<[f32; 4]> = (0..w * h).map(|_| [next(), next(), next(), 1.0]).collect();
        let noise = WorkingImage::from_pixels(w, h, noise_px).unwrap();
        for id in ColorSpaceId::ALL {
            let t0 = std::time::Instant::now();
            let _l8 = super::LutEncoder::<u8>::new(id.transfer());
            let t8 = t0.elapsed();
            let t0 = std::time::Instant::now();
            let _l16 = super::LutEncoder::<u16>::new(id.transfer());
            let t16 = t0.elapsed();
            println!(
                "LUT build {id:?}: u8 {:.3} ms, u16 {:.3} ms",
                t8.as_secs_f64() * 1e3,
                t16.as_secs_f64() * 1e3,
            );
        }
        for (name, id, img) in [
            ("ramp srgb", ColorSpaceId::Srgb, &ramp),
            ("ramp oprgb", ColorSpaceId::OpRgb, &ramp),
            ("ramp linear", ColorSpaceId::LinearRec2020, &ramp),
            ("noise srgb", ColorSpaceId::Srgb, &noise),
            ("noise linear", ColorSpaceId::LinearRec2020, &noise),
        ] {
            // The minimum of some runs is less sensitive to other load on
            // the machine.
            let mut t8 = std::time::Duration::MAX;
            let mut t16 = std::time::Duration::MAX;
            let mut out8 = None;
            let mut out16 = None;
            for _ in 0..7 {
                let t0 = std::time::Instant::now();
                out8 = Some(img.to_image::<u8>(id, ChannelLayout::Rgb).unwrap());
                t8 = t8.min(t0.elapsed());
                let t0 = std::time::Instant::now();
                out16 = Some(img.to_image::<u16>(id, ChannelLayout::Rgb).unwrap());
                t16 = t16.min(t0.elapsed());
            }
            let (out8, out16) = (out8.unwrap(), out16.unwrap());
            println!(
                "to_image 6000x4000 {name}: u8 {:.1} ms, u16 {:.1} ms (checksum {} {})",
                t8.as_secs_f64() * 1e3,
                t16.as_secs_f64() * 1e3,
                out8.as_slice().iter().map(|&v| u64::from(v)).sum::<u64>(),
                out16.as_slice().iter().map(|&v| u64::from(v)).sum::<u64>(),
            );
        }
    }
}
