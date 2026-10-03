//! High-quality resampling of working images, and output size helpers.
//!
//! # Filter
//!
//! [`resize`] uses a separable Lanczos-3 filter (a sinc windowed by a wider
//! sinc, radius 3). It filters rows first, then columns.
//!
//! - The filter works on linear light, so mean brightness stays correct.
//! - It works on premultiplied alpha, so the color of transparent pixels
//!   does not bleed into visible pixels.
//! - Downscale: the filter support is scaled by the size ratio (each output
//!   pixel covers `6 * ratio` source pixels). This is necessary to prevent
//!   aliasing.
//! - Upscale: the filter interpolates with the unscaled kernel (6 taps).
//! - Edges: taps outside the image are removed, and the remaining weights
//!   are normalized to a sum of 1. A constant image therefore stays
//!   constant, also at the edges.
//! - A side that keeps its size is copied without a change.
//!
//! # Ringing
//!
//! A Lanczos filter has negative lobes. Near hard edges it gives values a
//! little below the darker side and a little above the lighter side. Color
//! values are **not** clamped: the working format permits values outside
//! `0.0..=1.0` (ADR 0002), and output encoding clips them. Alpha **is**
//! clamped to `0.0..=1.0`, because alpha outside this range has no meaning.
//!
//! # Determinism
//!
//! The code is single-threaded and uses no platform services. The same input
//! gives the same output on the same platform.

use std::f64::consts::PI;

use serde::{Deserialize, Serialize};

use crate::buffer::working::{Rgba, WorkingImage};
use crate::error::{CoreError, CoreResult};

/// The radius of the Lanczos-3 kernel, in source pixels at scale 1.
const LANCZOS_RADIUS: f64 = 3.0;

/// Alpha below this value is set to zero after a resize of an image with
/// straight alpha. The color of such a pixel is not stable after division
/// by alpha. The value is below the resolution of 16-bit output.
const MIN_STRAIGHT_ALPHA: f32 = 1.0 / 65536.0;

/// How the alpha channel of a [`WorkingImage`] is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AlphaMode {
    /// RGB is not multiplied by alpha. Decoders give this mode.
    #[default]
    Straight,
    /// RGB is multiplied by alpha. The editor compositor uses this mode.
    Premultiplied,
}

/// How to compute an output size from a source size (see [`fit_size`]).
///
/// All target values must be at least 1 (megapixels: finite and more than
/// 0). The aspect ratio is kept.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "mode", content = "value")]
pub enum ResizeMode {
    /// Keep the source size.
    None,
    /// The longer side gets this length.
    LongEdge(u32),
    /// The shorter side gets this length.
    ShortEdge(u32),
    /// The width gets this length.
    Width(u32),
    /// The height gets this length.
    Height(u32),
    /// The pixel count is at most this number of megapixels (1 megapixel =
    /// 1 000 000 pixels). Each side is rounded down.
    Megapixels(f64),
}

/// Returns the output size for a source of `width` x `height` pixels.
///
/// If the computed size is larger than the source and `allow_upscale` is
/// `false`, the function returns the source size. Each side of the result is
/// at least 1.
///
/// Errors:
/// - [`CoreError::InvalidDimensions`] if `width` or `height` is 0.
/// - [`CoreError::OutOfRange`] if a target value is 0 (or megapixels is not
///   more than 0), or if a computed side does not fit in `u32`.
/// - [`CoreError::NonFinite`] if megapixels is NaN or infinite.
pub fn fit_size(
    width: u32,
    height: u32,
    mode: ResizeMode,
    allow_upscale: bool,
) -> CoreResult<(u32, u32)> {
    if width == 0 || height == 0 {
        return Err(CoreError::InvalidDimensions { width, height });
    }
    let (w, h) = (u64::from(width), u64::from(height));
    let target = match mode {
        ResizeMode::None => return Ok((width, height)),
        ResizeMode::LongEdge(len) => {
            let len = u64::from(check_target("long edge", len)?);
            let long = w.max(h);
            (scale_side(w, len, long)?, scale_side(h, len, long)?)
        }
        ResizeMode::ShortEdge(len) => {
            let len = u64::from(check_target("short edge", len)?);
            let short = w.min(h);
            (scale_side(w, len, short)?, scale_side(h, len, short)?)
        }
        ResizeMode::Width(len) => {
            let len = u64::from(check_target("width", len)?);
            (scale_side(w, len, w)?, scale_side(h, len, w)?)
        }
        ResizeMode::Height(len) => {
            let len = u64::from(check_target("height", len)?);
            (scale_side(w, len, h)?, scale_side(h, len, h)?)
        }
        ResizeMode::Megapixels(mp) => megapixel_size(width, height, mp)?,
    };
    let upscales = u64::from(target.0) * u64::from(target.1) > w * h;
    if upscales && !allow_upscale {
        Ok((width, height))
    } else {
        Ok(target)
    }
}

fn check_target(name: &'static str, len: u32) -> CoreResult<u32> {
    if len == 0 {
        return Err(CoreError::OutOfRange {
            name,
            value: 0.0,
            min: 1.0,
            max: f64::from(u32::MAX),
        });
    }
    Ok(len)
}

/// Returns `round(side * num / den)`, at least 1. `den` is at least 1.
fn scale_side(side: u64, num: u64, den: u64) -> CoreResult<u32> {
    // side and num are at most u32::MAX, so the product fits in u64 (and
    // den / 2 adds less than u32::MAX).
    let v = (side * num + den / 2) / den;
    to_side(v.max(1))
}

fn to_side(v: u64) -> CoreResult<u32> {
    u32::try_from(v).map_err(|_| CoreError::OutOfRange {
        name: "output side",
        #[allow(
            clippy::cast_precision_loss,
            reason = "the value is only for the error message"
        )]
        value: v as f64,
        min: 1.0,
        max: f64::from(u32::MAX),
    })
}

fn megapixel_size(width: u32, height: u32, mp: f64) -> CoreResult<(u32, u32)> {
    if !mp.is_finite() {
        return Err(CoreError::NonFinite { name: "megapixels" });
    }
    if mp <= 0.0 {
        return Err(CoreError::OutOfRange {
            name: "megapixels",
            value: mp,
            min: f64::MIN_POSITIVE,
            max: f64::MAX,
        });
    }
    let (w, h) = (f64::from(width), f64::from(height));
    let scale = (mp * 1.0e6 / (w * h)).sqrt();
    // The small addition protects against results like 1999.9999999 for an
    // exact 2000; it is far below one pixel.
    let side = |s: f64| -> CoreResult<u32> {
        let v = (s * scale + 1e-6).floor().max(1.0);
        if v > f64::from(u32::MAX) {
            return Err(CoreError::OutOfRange {
                name: "output side",
                value: v,
                min: 1.0,
                max: f64::from(u32::MAX),
            });
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "v is an integer in 1..=u32::MAX (checked above)"
        )]
        Ok(v as u32)
    };
    Ok((side(w)?, side(h)?))
}

/// The filter taps of one output pixel: weights for the source pixels
/// `first..first + weights.len()`.
#[derive(Debug, Clone)]
struct Taps {
    first: usize,
    weights: Vec<f32>,
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        let p = PI * x;
        p.sin() / p
    }
}

fn lanczos3(x: f64) -> f64 {
    if x.abs() >= LANCZOS_RADIUS {
        0.0
    } else {
        sinc(x) * sinc(x / LANCZOS_RADIUS)
    }
}

/// Computes the taps for resampling a side of `src` pixels to `dst` pixels.
///
/// Pixel `j` has its center at `j + 0.5`. Output pixel `i` maps to the
/// source position `(i + 0.5) * src / dst`, so that the image corners stay
/// aligned.
fn compute_taps(src: u32, dst: u32) -> Vec<Taps> {
    if src == dst {
        // Exact copy: no rounding error from the kernel.
        return (0..src as usize)
            .map(|i| Taps {
                first: i,
                weights: vec![1.0],
            })
            .collect();
    }
    let ratio = f64::from(src) / f64::from(dst);
    let filter_scale = ratio.max(1.0);
    let support = LANCZOS_RADIUS * filter_scale;
    let src_f = f64::from(src);
    (0..dst)
        .map(|i| {
            let center = (f64::from(i) + 0.5) * ratio;
            // Both values are clamped to 0.0..=src, so the casts are exact
            // integers in range.
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "values are clamped to 0..=src (a u32)"
            )]
            let (lo, hi) = (
                (center - support).floor().clamp(0.0, src_f) as u32,
                (center + support).ceil().clamp(0.0, src_f) as u32,
            );
            let raw: Vec<f64> = (lo..hi)
                .map(|j| lanczos3((f64::from(j) + 0.5 - center) / filter_scale))
                .collect();
            let sum: f64 = raw.iter().sum();
            if raw.is_empty() || sum.abs() < 1e-12 {
                // This cannot happen for Lanczos-3: the window always
                // contains the source pixel nearest to `center`, whose
                // weight is in the positive main lobe, and the clipped
                // lobes are smaller. Use that nearest pixel, not a blank
                // output, if the assumption is ever wrong.
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "center is in 0..src, so the floor is a valid index"
                )]
                let nearest = (center.floor() as usize).min(src as usize - 1);
                return Taps {
                    first: nearest,
                    weights: vec![1.0],
                };
            }
            #[allow(
                clippy::cast_possible_truncation,
                reason = "normalized weights are small; f32 precision is sufficient"
            )]
            let weights = raw.iter().map(|w| (w / sum) as f32).collect();
            Taps {
                first: lo as usize,
                weights,
            }
        })
        .collect()
}

#[inline]
fn premultiplied(p: Rgba, alpha: AlphaMode) -> Rgba {
    match alpha {
        AlphaMode::Straight => [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]],
        AlphaMode::Premultiplied => p,
    }
}

/// Resizes `image` to exactly `width` x `height` pixels with a Lanczos-3
/// filter in linear light (see the module documentation).
///
/// `alpha` tells how the alpha of `image` is stored. The output uses the
/// same mode. For [`AlphaMode::Straight`], the function premultiplies before
/// the filter and divides after it; output alpha below 2^-16 becomes 0 with
/// zero color.
///
/// If the size does not change, the function returns a copy.
///
/// Returns [`CoreError::InvalidDimensions`] if `width` or `height` is 0, or
/// [`CoreError::SizeOverflow`] if the output does not fit in memory.
pub fn resize(
    image: &WorkingImage,
    width: u32,
    height: u32,
    alpha: AlphaMode,
) -> CoreResult<WorkingImage> {
    let (sw, sh) = (image.width(), image.height());
    if (sw, sh) == (width, height) {
        return Ok(image.clone());
    }
    // This also checks the output size before the filter work.
    let mut out = WorkingImage::new(width, height)?;
    let x_taps = compute_taps(sw, width);
    let y_taps = compute_taps(sh, height);
    let dw = width as usize;

    // Pass 1: filter each source row to the output width (premultiplied).
    // The intermediate image is `width` x `sh`; its size fits because the
    // output and the source both fit, and it is checked again here.
    let mid_len = dw.checked_mul(sh as usize).ok_or(CoreError::SizeOverflow {
        width,
        height: sh,
        channels: 4,
    })?;
    let mut mid: Vec<Rgba> = vec![[0.0; 4]; mid_len];
    for (src_row, mid_row) in image.rows().zip(mid.chunks_exact_mut(dw)) {
        for (o, t) in mid_row.iter_mut().zip(&x_taps) {
            let mut acc = [0.0_f32; 4];
            for (p, w) in src_row[t.first..].iter().zip(&t.weights) {
                let p = premultiplied(*p, alpha);
                for c in 0..4 {
                    acc[c] += w * p[c];
                }
            }
            *o = acc;
        }
    }

    // Pass 2: filter columns. Each output row is a weighted sum of whole
    // intermediate rows, which keeps memory access sequential.
    for (out_row, t) in out.rows_mut().zip(&y_taps) {
        for (k, w) in t.weights.iter().enumerate() {
            let start = (t.first + k) * dw;
            let mid_row = &mid[start..start + dw];
            for (o, p) in out_row.iter_mut().zip(mid_row) {
                for c in 0..4 {
                    o[c] += w * p[c];
                }
            }
        }
    }

    // Alpha outside 0..=1 from ringing has no meaning: clamp it.
    for p in out.pixels_mut() {
        p[3] = if p[3].is_nan() {
            0.0
        } else {
            p[3].clamp(0.0, 1.0)
        };
        if alpha == AlphaMode::Straight && p[3] < MIN_STRAIGHT_ALPHA {
            *p = [0.0; 4];
        }
    }
    if alpha == AlphaMode::Straight {
        out.unpremultiply_alpha();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]
    #![allow(clippy::cast_precision_loss, reason = "test sizes are small")]
    #![allow(clippy::float_cmp, reason = "the tests check exact copies")]

    use super::*;

    fn pattern(w: u32, h: u32) -> WorkingImage {
        let mut img = WorkingImage::new(w, h).unwrap();
        for y in 0..h {
            for x in 0..w {
                // A deterministic pseudo-random pattern in 0..1.
                let v = ((x * 7919 + y * 104_729) % 1000) as f32 / 1000.0;
                *img.pixel_mut(x, y).unwrap() = [v, 1.0 - v, 0.5 * v, 1.0];
            }
        }
        img
    }

    fn mean(img: &WorkingImage) -> [f64; 4] {
        let mut m = [0.0_f64; 4];
        for p in img.pixels() {
            for c in 0..4 {
                m[c] += f64::from(p[c]);
            }
        }
        let n = img.pixels().len() as f64;
        m.map(|v| v / n)
    }

    #[test]
    fn constant_image_stays_constant() {
        let fill = [0.3, 2.5, -0.1, 1.0];
        let img = WorkingImage::new_filled(37, 23, fill).unwrap();
        for (w, h) in [(10, 7), (100, 50), (37, 5), (1, 1), (80, 23)] {
            for mode in [AlphaMode::Straight, AlphaMode::Premultiplied] {
                let out = resize(&img, w, h, mode).unwrap();
                assert_eq!((out.width(), out.height()), (w, h));
                for p in out.pixels() {
                    for c in 0..4 {
                        assert!((p[c] - fill[c]).abs() < 1e-5, "{w}x{h} {mode:?}: {p:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn same_size_is_exact_copy() {
        let img = pattern(13, 9);
        assert_eq!(resize(&img, 13, 9, AlphaMode::Straight).unwrap(), img);
        // Only the height changes: rows are exact copies of the columns.
        let out = resize(&img, 13, 4, AlphaMode::Premultiplied).unwrap();
        assert_eq!(out.width(), 13);
    }

    #[test]
    fn downscale_preserves_mean() {
        let img = pattern(400, 300);
        let m0 = mean(&img);
        for (w, h) in [(100, 75), (37, 29), (200, 150)] {
            let out = resize(&img, w, h, AlphaMode::Straight).unwrap();
            let m1 = mean(&out);
            for c in 0..4 {
                assert!(
                    (m0[c] - m1[c]).abs() < 5e-3,
                    "{w}x{h} channel {c}: {} vs {}",
                    m0[c],
                    m1[c]
                );
            }
        }
    }

    #[test]
    fn downscale_removes_high_frequency() {
        // A one-pixel checkerboard has no visible structure after a 4x
        // downscale: all output pixels are close to the mean 0.5.
        let mut img = WorkingImage::new(64, 64).unwrap();
        for y in 0..64 {
            for x in 0..64 {
                let v = if (x + y) % 2 == 0 { 1.0 } else { 0.0 };
                *img.pixel_mut(x, y).unwrap() = [v, v, v, 1.0];
            }
        }
        let out = resize(&img, 16, 16, AlphaMode::Straight).unwrap();
        for p in out.pixels() {
            assert!((p[0] - 0.5).abs() < 0.02, "{p:?}");
        }
    }

    #[test]
    fn one_pixel_wide_and_tall_images() {
        let mut img = WorkingImage::new(1, 5).unwrap();
        for y in 0..5 {
            let v = y as f32 / 4.0;
            *img.pixel_mut(0, y).unwrap() = [v, v, v, 1.0];
        }
        let up = resize(&img, 3, 10, AlphaMode::Straight).unwrap();
        // The three columns are equal (one source column).
        for row in up.rows() {
            assert_eq!(row[0], row[1]);
            assert_eq!(row[1], row[2]);
        }
        let down = resize(&img, 1, 1, AlphaMode::Straight).unwrap();
        let p = down.pixel(0, 0).unwrap();
        assert!((p[0] - 0.5).abs() < 1e-3, "{p:?}");
        let wide = WorkingImage::new_filled(9, 1, [0.25, 0.5, 0.75, 1.0]).unwrap();
        let out = resize(&wide, 2, 4, AlphaMode::Straight).unwrap();
        for p in out.pixels() {
            assert!((p[1] - 0.5).abs() < 1e-5);
        }
    }

    #[test]
    fn transparent_color_does_not_bleed() {
        // Left half: opaque red. Right half: transparent green. With
        // premultiplied filtering, no output pixel gets green.
        let mut img = WorkingImage::new(16, 4).unwrap();
        for y in 0..4 {
            for x in 0..16 {
                let p = if x < 8 {
                    [1.0, 0.0, 0.0, 1.0]
                } else {
                    [0.0, 1.0, 0.0, 0.0]
                };
                *img.pixel_mut(x, y).unwrap() = p;
            }
        }
        let out = resize(&img, 6, 2, AlphaMode::Straight).unwrap();
        for p in out.pixels() {
            assert!((0.0..=1.0).contains(&p[3]), "{p:?}");
            if p[3] > 0.05 {
                assert!(p[1].abs() < 1e-4, "green leaked: {p:?}");
                assert!((p[0] - 1.0).abs() < 0.1, "{p:?}");
            }
        }
        // Premultiplied input gives premultiplied output: color <= alpha
        // except for small ringing.
        let mut pm = img.clone();
        pm.premultiply_alpha();
        let out = resize(&pm, 6, 2, AlphaMode::Premultiplied).unwrap();
        for p in out.pixels() {
            assert!(p[1].abs() < 1e-4, "{p:?}");
        }
    }

    #[test]
    fn invalid_target_size() {
        let img = pattern(4, 4);
        assert!(matches!(
            resize(&img, 0, 4, AlphaMode::Straight),
            Err(CoreError::InvalidDimensions { .. })
        ));
    }

    #[test]
    fn fit_size_modes() {
        let f = |m, up| fit_size(6000, 4000, m, up).unwrap();
        assert_eq!(f(ResizeMode::None, false), (6000, 4000));
        assert_eq!(f(ResizeMode::LongEdge(2048), false), (2048, 1365));
        assert_eq!(f(ResizeMode::ShortEdge(1000), false), (1500, 1000));
        assert_eq!(f(ResizeMode::Width(3000), false), (3000, 2000));
        assert_eq!(f(ResizeMode::Height(400), false), (600, 400));
        assert_eq!(f(ResizeMode::Megapixels(6.0), false), (3000, 2000));
        // Portrait.
        assert_eq!(
            fit_size(4000, 6000, ResizeMode::LongEdge(3000), false).unwrap(),
            (2000, 3000)
        );
        // Each side is at least 1.
        assert_eq!(
            fit_size(10_000, 10, ResizeMode::LongEdge(100), false).unwrap(),
            (100, 1)
        );
        assert_eq!(
            fit_size(1, 1, ResizeMode::Megapixels(0.000_000_1), false).unwrap(),
            (1, 1)
        );
    }

    #[test]
    fn fit_size_does_not_upscale_unless_asked() {
        assert_eq!(
            fit_size(800, 600, ResizeMode::LongEdge(2000), false).unwrap(),
            (800, 600)
        );
        assert_eq!(
            fit_size(800, 600, ResizeMode::LongEdge(2000), true).unwrap(),
            (2000, 1500)
        );
        assert_eq!(
            fit_size(800, 600, ResizeMode::Megapixels(24.0), false).unwrap(),
            (800, 600)
        );
        let (w, h) = fit_size(800, 600, ResizeMode::Megapixels(1.92), true).unwrap();
        assert_eq!((w, h), (1600, 1200));
    }

    #[test]
    fn fit_size_errors() {
        assert!(fit_size(0, 5, ResizeMode::None, false).is_err());
        assert!(matches!(
            fit_size(5, 5, ResizeMode::Width(0), false),
            Err(CoreError::OutOfRange { .. })
        ));
        assert!(matches!(
            fit_size(5, 5, ResizeMode::Megapixels(f64::NAN), false),
            Err(CoreError::NonFinite { .. })
        ));
        assert!(matches!(
            fit_size(5, 5, ResizeMode::Megapixels(-1.0), false),
            Err(CoreError::OutOfRange { .. })
        ));
        // The short side would need more than u32::MAX pixels.
        assert!(matches!(
            fit_size(u32::MAX, 1, ResizeMode::ShortEdge(2), true),
            Err(CoreError::OutOfRange { .. })
        ));
    }

    #[test]
    fn resize_mode_serde_names() {
        let s = serde_json::to_string(&ResizeMode::LongEdge(2048)).unwrap();
        assert_eq!(s, r#"{"mode":"longEdge","value":2048}"#);
        let n: ResizeMode = serde_json::from_str(r#"{"mode":"none"}"#).unwrap();
        assert_eq!(n, ResizeMode::None);
    }
}
