//! Conversion of sensor data to the working image.
//!
//! Steps of [`demosaic()`]:
//!
//! 1. Subtract the black level and divide by `white - black`. Values are not
//!    clipped: noise below black gives negative values, and values above
//!    the white level stay above 1.0 (for highlight recovery).
//! 2. Multiply by the white balance multipliers.
//! 3. Demosaic the active area (CFA sensors only).
//! 4. Keep the crop area (or the active area).
//! 5. Convert white-balanced camera RGB to linear Rec. 2020 with one matrix
//!    (see [`ColorSolution`]).
//!
//! Orientation is not applied. Use [`crate::Orientation::apply`].

use expose_core::buffer::{Rect, Rgba, WorkingImage};
use expose_core::color::{Chromaticity, WORKING_WHITE, mul_vec_f32};

use super::color::ColorSolution;
use super::{AsShotWhite, CfaPattern, RawImage, SensorLayout};
use crate::error::{DecodeError, DecodeResult};

/// The demosaic algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DemosaicMethod {
    /// [`DemosaicMethod::Mhc`] for 2x2 Bayer patterns with an active area
    /// of at least 3x3 pixels; [`DemosaicMethod::Bilinear`] otherwise.
    #[default]
    Auto,
    /// Bilinear interpolation: each missing color is the mean of the
    /// nearest cells of that color. Works for all CFA patterns (for
    /// example 6x6 patterns). This is the reference algorithm.
    Bilinear,
    /// Malvar-He-Cutler (2004) gradient-corrected linear interpolation with
    /// 5x5 kernels. 2x2 Bayer patterns only. Edges use mirrored pixels.
    Mhc,
}

/// The white balance to use.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum WhiteBalance {
    /// The as-shot white balance of the file. If the file has none, D65 is
    /// used and [`DevelopedRaw::white_balance_assumed`] is `true`.
    #[default]
    AsShot,
    /// A camera-native neutral (RGB values of a gray object).
    Neutral([f64; 3]),
    /// A scene white point.
    White(Chromaticity),
}

/// Options of [`demosaic()`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DemosaicOptions {
    /// The demosaic algorithm.
    pub method: DemosaicMethod,
    /// The white balance.
    pub white_balance: WhiteBalance,
    /// If `true`, the output is the crop area of the raw image. If
    /// `false`, the output is the full active area.
    pub crop: bool,
}

impl Default for DemosaicOptions {
    fn default() -> Self {
        Self {
            method: DemosaicMethod::Auto,
            white_balance: WhiteBalance::AsShot,
            crop: true,
        }
    }
}

/// The result of [`demosaic()`].
#[derive(Debug, Clone)]
pub struct DevelopedRaw {
    /// The image in the working format (linear Rec. 2020, alpha 1).
    pub image: WorkingImage,
    /// The color solution. `None` for monochrome sensors.
    pub solution: Option<ColorSolution>,
    /// The algorithm that was used. `None` if no demosaic was necessary.
    pub method: Option<DemosaicMethod>,
    /// `true` if the as-shot white balance was requested but the file had
    /// none, so that D65 was used.
    pub white_balance_assumed: bool,
}

/// Converts sensor data to the working image. See the module documentation.
pub fn demosaic(raw: &RawImage, options: &DemosaicOptions) -> DecodeResult<DevelopedRaw> {
    raw.validate()?;
    let area = raw.active_area;
    let out_rect = if options.crop { raw.crop } else { area };

    let mut white_balance_assumed = false;
    let solution = if raw.layout == SensorLayout::Monochrome {
        None
    } else {
        let cals = &raw.calibrations;
        Some(match (options.white_balance, raw.as_shot) {
            (WhiteBalance::AsShot, AsShotWhite::Neutral(n)) | (WhiteBalance::Neutral(n), _) => {
                ColorSolution::from_neutral(cals, n)?
            }
            (WhiteBalance::AsShot, AsShotWhite::Unknown) => {
                white_balance_assumed = true;
                ColorSolution::from_white(cals, WORKING_WHITE)?
            }
            (WhiteBalance::White(xy), _) => ColorSolution::from_white(cals, xy)?,
        })
    };
    let mult = solution.map_or([1.0; 3], |s| s.multipliers);

    let (rgb, method) = match &raw.layout {
        SensorLayout::Cfa(pattern) => {
            let plane = cfa_plane(raw, pattern, mult)?;
            let method = match options.method {
                DemosaicMethod::Auto
                    if pattern.is_bayer() && area.width >= 3 && area.height >= 3 =>
                {
                    DemosaicMethod::Mhc
                }
                DemosaicMethod::Auto => DemosaicMethod::Bilinear,
                m => m,
            };
            let local = Rect::new(
                out_rect.x - area.x,
                out_rect.y - area.y,
                out_rect.width,
                out_rect.height,
            );
            let grid = Grid {
                plane: &plane,
                w: area.width as usize,
                h: area.height as usize,
                pattern,
                ox: area.x as usize,
                oy: area.y as usize,
            };
            let rgb = match method {
                DemosaicMethod::Mhc => grid.mhc(local)?,
                _ => grid.bilinear(local),
            };
            (rgb, Some(method))
        }
        SensorLayout::LinearRgb | SensorLayout::Monochrome => (direct(raw, out_rect, mult)?, None),
    };

    let pixels: Vec<Rgba> = match solution {
        Some(s) => {
            let m = s.camera_to_working.to_f32();
            rgb.into_iter()
                .map(|p| {
                    let [r, g, b] = mul_vec_f32(&m, p);
                    [r, g, b, 1.0]
                })
                .collect()
        }
        None => rgb.into_iter().map(|[r, g, b]| [r, g, b, 1.0]).collect(),
    };
    let image = WorkingImage::from_pixels(out_rect.width, out_rect.height, pixels)?;
    Ok(DevelopedRaw {
        image,
        solution,
        method,
        white_balance_assumed,
    })
}

/// Returns the normalized value of sample `c` of pixel `(x, y)` (full
/// sensor coordinates) with value `s`.
#[inline]
fn normalize(raw: &RawImage, x: usize, y: usize, c: usize, s: f32) -> DecodeResult<f32> {
    let b = raw.black.at(x, y, c);
    let range = raw.white.at(x, y, c) - b;
    if range <= 0.0 || !range.is_finite() {
        return Err(DecodeError::invalid(format!(
            "white level is not above black level at ({x}, {y})"
        )));
    }
    Ok((s - b) / range)
}

/// Returns the normalized, white-balanced CFA values of the active area.
fn cfa_plane(raw: &RawImage, pattern: &CfaPattern, mult: [f32; 3]) -> DecodeResult<Vec<f32>> {
    let a = raw.active_area;
    let full_w = raw.width as usize;
    let mut plane = Vec::with_capacity(a.width as usize * a.height as usize);
    for y in a.y as usize..(a.y + a.height) as usize {
        for x in a.x as usize..(a.x + a.width) as usize {
            // validate() proved that the active area is inside the image
            // and that the sample count is width * height.
            let s = raw
                .samples
                .get(y * full_w + x)
                .ok_or_else(|| DecodeError::invalid("sample index out of range"))?;
            let color = pattern.color_at(x, y).index();
            plane.push(normalize(raw, x, y, 0, s)? * mult[color]);
        }
    }
    Ok(plane)
}

/// Normalizes data that needs no demosaic (linear RGB or monochrome).
fn direct(raw: &RawImage, rect: Rect, mult: [f32; 3]) -> DecodeResult<Vec<[f32; 3]>> {
    let spp = raw.layout.samples_per_pixel();
    let full_w = raw.width as usize;
    let mut out = Vec::with_capacity(rect.width as usize * rect.height as usize);
    for y in rect.y as usize..(rect.y + rect.height) as usize {
        for x in rect.x as usize..(rect.x + rect.width) as usize {
            let base = (y * full_w + x) * spp;
            let mut px = [0.0_f32; 3];
            for (c, v) in px.iter_mut().enumerate() {
                let sc = c.min(spp - 1);
                let s = raw
                    .samples
                    .get(base + sc)
                    .ok_or_else(|| DecodeError::invalid("sample index out of range"))?;
                *v = normalize(raw, x, y, sc, s)? * mult[c];
            }
            out.push(px);
        }
    }
    Ok(out)
}

/// A normalized CFA plane of the active area.
struct Grid<'a> {
    plane: &'a [f32],
    w: usize,
    h: usize,
    pattern: &'a CfaPattern,
    /// Offset of the plane origin in sensor coordinates (pattern phase).
    ox: usize,
    oy: usize,
}

impl Grid<'_> {
    #[inline]
    fn color(&self, x: usize, y: usize) -> usize {
        self.pattern.color_at(x + self.ox, y + self.oy).index()
    }

    #[inline]
    fn at(&self, x: usize, y: usize) -> f32 {
        self.plane[y * self.w + x]
    }

    /// Bilinear demosaic of `rect` (plane coordinates).
    fn bilinear(&self, rect: Rect) -> Vec<[f32; 3]> {
        let (pw, ph) = (
            self.pattern.width() as usize,
            self.pattern.height() as usize,
        );
        let max_r = pw.max(ph);
        // For each pattern phase and color: the neighbor offsets at the
        // smallest radius that contains that color (on an unbounded plane).
        let mut table: Vec<[Vec<(isize, isize)>; 3]> = Vec::with_capacity(pw * ph);
        let mut reach = 1_usize;
        for py in 0..ph {
            for px in 0..pw {
                let mut lists: [Vec<(isize, isize)>; 3] = Default::default();
                for (c, list) in lists.iter_mut().enumerate() {
                    for r in 1..=max_r {
                        let ri = r.cast_signed();
                        for dy in -ri..=ri {
                            for dx in -ri..=ri {
                                if (dx, dy) == (0, 0) {
                                    continue;
                                }
                                // (px, py) is a phase in sensor coordinates.
                                // Add whole periods so that the sum with a
                                // negative offset stays positive.
                                let sx = (px + pw * max_r).wrapping_add_signed(dx);
                                let sy = (py + ph * max_r).wrapping_add_signed(dy);
                                if self.pattern.color_at(sx, sy).index() == c {
                                    list.push((dx, dy));
                                }
                            }
                        }
                        if !list.is_empty() {
                            reach = reach.max(r);
                            break;
                        }
                    }
                }
                table.push(lists);
            }
        }

        let mut out = Vec::with_capacity(rect.width as usize * rect.height as usize);
        for y in rect.y as usize..(rect.y + rect.height) as usize {
            for x in rect.x as usize..(rect.x + rect.width) as usize {
                let own = self.color(x, y);
                let mut px = [0.0_f32; 3];
                px[own] = self.at(x, y);
                let interior = x >= reach && y >= reach && x + reach < self.w && y + reach < self.h;
                for (c, slot) in px.iter_mut().enumerate() {
                    if c == own {
                        continue;
                    }
                    *slot = if interior {
                        // Phase of (x, y) in the pattern (sensor coordinates).
                        let phase = ((y + self.oy) % ph) * pw + (x + self.ox) % pw;
                        let list = &table[phase][c];
                        let sum: f32 = list
                            .iter()
                            .map(|&(dx, dy)| {
                                self.at(x.wrapping_add_signed(dx), y.wrapping_add_signed(dy))
                            })
                            .sum();
                        #[allow(clippy::cast_precision_loss, reason = "list length is small")]
                        let n = list.len() as f32;
                        sum / n
                    } else {
                        self.edge_mean(x, y, c, max_r)
                    };
                }
                out.push(px);
            }
        }
        out
    }

    /// Mean of the nearest in-bounds cells of color `c` around `(x, y)`.
    ///
    /// The plane is at least as large as the pattern (checked by
    /// `RawImage::validate`), and the pattern contains all colors, so a
    /// radius of `max_r` always finds a cell. The 0.0 result for no cell
    /// cannot occur.
    fn edge_mean(&self, x: usize, y: usize, c: usize, max_r: usize) -> f32 {
        for r in 1..=max_r {
            let (x0, x1) = (x.saturating_sub(r), (x + r).min(self.w - 1));
            let (y0, y1) = (y.saturating_sub(r), (y + r).min(self.h - 1));
            let mut sum = 0.0;
            let mut n = 0_u32;
            for yy in y0..=y1 {
                for xx in x0..=x1 {
                    if (xx, yy) != (x, y) && self.color(xx, yy) == c {
                        sum += self.at(xx, yy);
                        n += 1;
                    }
                }
            }
            if n > 0 {
                #[allow(clippy::cast_precision_loss, reason = "n is small")]
                return sum / n as f32;
            }
        }
        0.0
    }

    /// Malvar-He-Cutler demosaic of `rect` (plane coordinates).
    fn mhc(&self, rect: Rect) -> DecodeResult<Vec<[f32; 3]>> {
        if !self.pattern.is_bayer() {
            return Err(DecodeError::unsupported(
                "MHC demosaic needs a 2x2 Bayer pattern",
            ));
        }
        if self.w < 3 || self.h < 3 {
            return Err(DecodeError::unsupported(
                "MHC demosaic needs an active area of at least 3x3 pixels",
            ));
        }
        let (w, h) = (self.w.cast_signed(), self.h.cast_signed());
        // Mirror without edge repeat: -1 -> 1, w -> w - 2. This keeps the
        // CFA phase. It is valid for offsets of at most 2 when w, h >= 3.
        let mirror = |i: isize, n: isize| -> usize {
            let i = if i < 0 { -i } else { i };
            let i = if i >= n { 2 * (n - 1) - i } else { i };
            i.clamp(0, n - 1).cast_unsigned()
        };
        let mut out = Vec::with_capacity(rect.width as usize * rect.height as usize);
        for y in rect.y as usize..(rect.y + rect.height) as usize {
            for x in rect.x as usize..(rect.x + rect.width) as usize {
                let (xi, yi) = (x.cast_signed(), y.cast_signed());
                let p = |dx: isize, dy: isize| self.at(mirror(xi + dx, w), mirror(yi + dy, h));
                let c = p(0, 0);
                let cross1 = p(-1, 0) + p(1, 0) + p(0, -1) + p(0, 1);
                let cross2 = p(-2, 0) + p(2, 0) + p(0, -2) + p(0, 2);
                let diag = p(-1, -1) + p(1, -1) + p(-1, 1) + p(1, 1);
                let h2 = p(-2, 0) + p(2, 0);
                let v2 = p(0, -2) + p(0, 2);
                let h1 = p(-1, 0) + p(1, 0);
                let v1 = p(0, -1) + p(0, 1);
                // Green at a red or blue cell.
                let g_rb = (4.0 * c + 2.0 * cross1 - cross2) / 8.0;
                // Red or blue at a blue or red cell.
                let rb_br = (6.0 * c + 2.0 * diag - 1.5 * cross2) / 8.0;
                // A color whose cells are left and right of this green cell.
                let horiz = (5.0 * c + 4.0 * h1 - diag - h2 + 0.5 * v2) / 8.0;
                // A color whose cells are above and below this green cell.
                let vert = (5.0 * c + 4.0 * v1 - diag - v2 + 0.5 * h2) / 8.0;

                let own = self.color(x, y);
                let px = match own {
                    1 => {
                        // Green cell: find the color of the horizontal
                        // neighbors (mirroring keeps the phase).
                        let hx = mirror(xi + 1, w);
                        if self.color(hx, y) == 0 {
                            [horiz, c, vert]
                        } else {
                            [vert, c, horiz]
                        }
                    }
                    0 => [c, g_rb, rb_br],
                    _ => [rb_br, g_rb, c],
                };
                out.push(px);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss, reason = "test coordinates are small")]
mod tests {
    use super::super::{CfaColor, ColorCalibration, LevelPattern, RawSamples};
    use super::*;
    use crate::orientation::Orientation;
    use expose_core::color::{ColorSpaceId, Mat3};

    /// A camera that sees linear sRGB.
    fn srgb_cal() -> ColorCalibration {
        ColorCalibration {
            illuminant: 21,
            xyz_to_camera: ColorSpaceId::Srgb
                .definition()
                .to_xyz()
                .unwrap()
                .inverse()
                .unwrap(),
        }
    }

    /// Builds a CFA raw image where each cell gets `f(color, x, y)`.
    fn cfa_raw(
        pattern: &str,
        w: u32,
        h: u32,
        black: f32,
        white: f32,
        f: impl Fn(CfaColor, u32, u32) -> f32,
    ) -> RawImage {
        let p = CfaPattern::bayer(pattern).unwrap();
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let c = p.color_at(x as usize, y as usize);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                data.push(f(c, x, y).round() as u16);
            }
        }
        RawImage {
            width: w,
            height: h,
            layout: SensorLayout::Cfa(p),
            samples: RawSamples::U16(data),
            black: LevelPattern::uniform(black, 1).unwrap(),
            white: LevelPattern::uniform(white, 1).unwrap(),
            bits_per_sample: 16,
            as_shot: AsShotWhite::Neutral([1.0, 1.0, 1.0]),
            calibrations: vec![srgb_cal()],
            active_area: Rect::from_size(w, h),
            crop: Rect::from_size(w, h),
            orientation: Orientation::Normal,
            make: String::new(),
            model: String::new(),
            backend: "test",
        }
    }

    fn opts(method: DemosaicMethod) -> DemosaicOptions {
        DemosaicOptions {
            method,
            ..DemosaicOptions::default()
        }
    }

    #[test]
    fn flat_gray_is_neutral_with_levels() {
        // Black 1000, white 9000: raw 5000 is 0.5.
        for pattern in ["RGGB", "BGGR", "GRBG", "GBRG"] {
            let raw = cfa_raw(pattern, 8, 6, 1000.0, 9000.0, |_, _, _| 5000.0);
            for m in [DemosaicMethod::Bilinear, DemosaicMethod::Mhc] {
                let out = demosaic(&raw, &opts(m)).unwrap();
                for p in out.image.pixels() {
                    for v in &p[..3] {
                        assert!((v - 0.5).abs() < 1e-5, "{pattern} {m:?} {p:?}");
                    }
                    assert!((p[3] - 1.0).abs() < f32::EPSILON);
                }
            }
        }
    }

    #[test]
    fn as_shot_neutral_balances_tinted_gray() {
        // A gray object under a light that gives camera (0.5, 1.0, 0.8).
        let n = [0.5, 1.0, 0.8];
        let mut raw = cfa_raw("RGGB", 6, 6, 0.0, 10000.0, |c, _, _| {
            #[allow(clippy::cast_possible_truncation)]
            let v = (4000.0 * n[c.index()]) as f32;
            v
        });
        raw.as_shot = AsShotWhite::Neutral(n);
        let out = demosaic(&raw, &DemosaicOptions::default()).unwrap();
        assert_eq!(out.method, Some(DemosaicMethod::Mhc));
        for p in out.image.pixels() {
            for v in &p[..3] {
                assert!((v - 0.4).abs() < 1e-4, "{p:?}");
            }
        }
        // Unknown white balance: D65 is used, and the result says so.
        raw.as_shot = AsShotWhite::Unknown;
        let out = demosaic(&raw, &DemosaicOptions::default()).unwrap();
        assert!(out.white_balance_assumed);
    }

    #[test]
    fn constant_color_is_reconstructed_and_converted() {
        // Linear sRGB camera; constant camera color (0.6, 0.3, 0.1).
        let col = [0.6_f32, 0.3, 0.1];
        let expected = mul_vec_f32(&ColorSpaceId::Srgb.to_working().unwrap().to_f32(), col);
        for pattern in ["RGGB", "GBRG"] {
            let raw = cfa_raw(pattern, 7, 5, 0.0, 10000.0, |c, _, _| {
                col[c.index()] * 10000.0
            });
            for m in [DemosaicMethod::Bilinear, DemosaicMethod::Mhc] {
                let out = demosaic(&raw, &opts(m)).unwrap();
                for p in out.image.pixels() {
                    for i in 0..3 {
                        assert!((p[i] - expected[i]).abs() < 1e-4, "{pattern} {m:?} {p:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn bilinear_is_exact_on_linear_ramp_interior() {
        // A horizontal ramp in all colors: interior interpolation is exact.
        let raw = cfa_raw("RGGB", 10, 10, 0.0, 10000.0, |_, x, _| {
            100.0 * x as f32 + 100.0
        });
        let out = demosaic(&raw, &opts(DemosaicMethod::Bilinear)).unwrap();
        for y in 1..9 {
            for x in 1..9 {
                let p = out.image.pixel(x, y).unwrap();
                let v = (100.0 * x as f32 + 100.0) / 10000.0;
                for c in &p[..3] {
                    assert!((c - v).abs() < 1e-4, "({x},{y}) {p:?}");
                }
            }
        }
    }

    #[test]
    fn highlights_are_not_clipped_and_crop_applies() {
        let mut raw = cfa_raw("RGGB", 8, 8, 0.0, 1000.0, |_, _, _| 3000.0);
        raw.crop = Rect::new(2, 2, 4, 3);
        let out = demosaic(&raw, &DemosaicOptions::default()).unwrap();
        assert_eq!((out.image.width(), out.image.height()), (4, 3));
        assert!(out.image.pixels().iter().all(|p| (p[1] - 3.0).abs() < 1e-4));
        let full = demosaic(
            &raw,
            &DemosaicOptions {
                crop: false,
                ..DemosaicOptions::default()
            },
        )
        .unwrap();
        assert_eq!(full.image.width(), 8);
    }

    #[test]
    fn active_area_offset_keeps_cfa_phase() {
        // Pattern origin is the full sensor; the active area starts at an
        // odd position. Constant color must still decode exactly.
        let col = [0.2_f32, 0.5, 0.7];
        let mut raw = cfa_raw("RGGB", 9, 9, 0.0, 1000.0, |c, _, _| col[c.index()] * 1000.0);
        raw.active_area = Rect::new(1, 1, 7, 7);
        raw.crop = Rect::new(2, 3, 4, 4);
        raw.calibrations = vec![ColorCalibration {
            illuminant: 21,
            xyz_to_camera: ColorSpaceId::LinearRec2020
                .definition()
                .to_xyz()
                .unwrap()
                .inverse()
                .unwrap(),
        }];
        for m in [DemosaicMethod::Bilinear, DemosaicMethod::Mhc] {
            let out = demosaic(&raw, &opts(m)).unwrap();
            for p in out.image.pixels() {
                for i in 0..3 {
                    assert!((p[i] - col[i]).abs() < 1e-3, "{m:?} {p:?}");
                }
            }
        }
    }

    #[test]
    fn non_bayer_pattern_uses_bilinear() {
        // A 2x2 pattern with greens in one row is not Bayer.
        let mut raw = cfa_raw("RGGB", 6, 6, 0.0, 100.0, |_, _, _| 50.0);
        raw.layout = SensorLayout::Cfa(CfaPattern::bayer("RGBG").unwrap());
        let out = demosaic(&raw, &DemosaicOptions::default()).unwrap();
        assert_eq!(out.method, Some(DemosaicMethod::Bilinear));
        assert!(demosaic(&raw, &opts(DemosaicMethod::Mhc)).is_err());
        assert!(out.image.pixels().iter().all(|p| (p[0] - 0.5).abs() < 1e-5));
    }

    #[test]
    fn bad_levels_are_errors() {
        let raw = cfa_raw("RGGB", 4, 4, 500.0, 500.0, |_, _, _| 1.0);
        assert!(demosaic(&raw, &DemosaicOptions::default()).is_err());
        let mut raw = cfa_raw("RGGB", 4, 4, 0.0, 500.0, |_, _, _| 1.0);
        raw.calibrations[0].xyz_to_camera = Mat3::new([[0.0; 3]; 3]);
        assert!(demosaic(&raw, &DemosaicOptions::default()).is_err());
    }

    #[test]
    fn monochrome_and_linear_rgb() {
        let mut raw = cfa_raw("RGGB", 3, 2, 0.0, 100.0, |_, _, _| 25.0);
        raw.layout = SensorLayout::Monochrome;
        raw.calibrations.clear();
        let out = demosaic(&raw, &DemosaicOptions::default()).unwrap();
        assert!(out.solution.is_none());
        assert!(
            out.image
                .pixels()
                .iter()
                .all(|p| (p[2] - 0.25).abs() < 1e-6)
        );

        let mut rgb = cfa_raw("RGGB", 2, 2, 0.0, 100.0, |_, _, _| 0.0);
        rgb.layout = SensorLayout::LinearRgb;
        rgb.samples = RawSamples::U16(vec![50; 12]);
        rgb.calibrations = vec![ColorCalibration {
            illuminant: 21,
            xyz_to_camera: ColorSpaceId::LinearRec2020
                .definition()
                .to_xyz()
                .unwrap()
                .inverse()
                .unwrap(),
        }];
        let out = demosaic(&rgb, &DemosaicOptions::default()).unwrap();
        assert_eq!(out.method, None);
        assert!(out.image.pixels().iter().all(|p| (p[0] - 0.5).abs() < 1e-5));
    }
}
