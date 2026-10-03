//! Camera RAW decode.
//!
//! # Structure
//!
//! - [`RawImage`]: sensor data and the data that the develop step needs
//!   (CFA pattern, black and white levels, as-shot white balance, camera
//!   color matrices, active area, crop, orientation).
//! - [`RawDecoder`]: the boundary for decode backends. A backend reads a
//!   file and returns a [`RawImage`]. It does no color processing.
//! - [`demosaic()`]: converts a [`RawImage`] to a [`WorkingImage`]
//!   (linear Rec. 2020). This step is the same for all backends.
//!
//! # Backends
//!
//! - `rawler` (cargo feature `rawler`, on by default): most RAW formats,
//!   DNG included. LGPL-2.1, approved by the operator (see the crate
//!   documentation).
//! - Without the `rawler` feature, no backend is available, and
//!   [`decode_raw`] returns [`DecodeError::UnsupportedFormat`].
//!
//! [`WorkingImage`]: expose_core::buffer::WorkingImage

mod color;
mod demosaic;
#[cfg(feature = "rawler")]
mod rawler_backend;

use expose_core::buffer::Rect;
use expose_core::color::Mat3;
use serde::{Deserialize, Serialize};

pub use color::{ColorSolution, illuminant_kelvin};
pub use demosaic::{DemosaicMethod, DemosaicOptions, DevelopedRaw, WhiteBalance, demosaic};
#[cfg(feature = "rawler")]
pub use rawler_backend::RawlerDecoder;

use crate::error::{DecodeError, DecodeResult};
use crate::format::{ImageFormat, detect_format};
use crate::limits::Limits;
use crate::orientation::Orientation;

/// A color of a color filter array cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CfaColor {
    /// Red.
    Red,
    /// Green.
    Green,
    /// Blue.
    Blue,
}

impl CfaColor {
    /// Returns the channel index (0 = red, 1 = green, 2 = blue).
    pub const fn index(self) -> usize {
        match self {
            Self::Red => 0,
            Self::Green => 1,
            Self::Blue => 2,
        }
    }
}

/// Largest CFA pattern side that this crate accepts.
pub const MAX_CFA_SIDE: u32 = 16;

/// A repeating color filter array pattern.
///
/// The pattern origin is the top-left pixel of the full sensor data
/// (`RawImage::data`), not of the active area.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfaPattern {
    width: u32,
    height: u32,
    colors: Vec<CfaColor>,
}

impl CfaPattern {
    /// Makes a pattern from its size and its colors in row-major order.
    ///
    /// Returns an error if the size is zero or more than
    /// [`MAX_CFA_SIDE`], if the color count does not agree with the size,
    /// or if the pattern does not contain all three colors.
    pub fn new(width: u32, height: u32, colors: Vec<CfaColor>) -> DecodeResult<Self> {
        if width == 0 || height == 0 || width > MAX_CFA_SIDE || height > MAX_CFA_SIDE {
            return Err(DecodeError::unsupported(format!(
                "CFA pattern size {width}x{height}"
            )));
        }
        if colors.len() != (width * height) as usize {
            return Err(DecodeError::invalid(format!(
                "CFA pattern {width}x{height} has {} colors",
                colors.len()
            )));
        }
        for c in [CfaColor::Red, CfaColor::Green, CfaColor::Blue] {
            if !colors.contains(&c) {
                return Err(DecodeError::unsupported(format!(
                    "CFA pattern without {c:?} is not supported"
                )));
            }
        }
        Ok(Self {
            width,
            height,
            colors,
        })
    }

    /// Makes a 2x2 pattern from a name such as `"RGGB"`.
    pub fn bayer(name: &str) -> DecodeResult<Self> {
        let colors = name
            .chars()
            .map(|c| match c {
                'R' => Ok(CfaColor::Red),
                'G' => Ok(CfaColor::Green),
                'B' => Ok(CfaColor::Blue),
                _ => Err(DecodeError::invalid(format!("bad CFA color '{c}'"))),
            })
            .collect::<DecodeResult<Vec<_>>>()?;
        Self::new(2, 2, colors)
    }

    /// Returns the pattern width.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the pattern height.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the color at sensor position `(x, y)`.
    #[inline]
    pub fn color_at(&self, x: usize, y: usize) -> CfaColor {
        let w = self.width as usize;
        let h = self.height as usize;
        // The constructor proved that colors.len() == w * h, so the index
        // is always in range.
        self.colors[(y % h) * w + (x % w)]
    }

    /// Returns `true` for a 2x2 pattern with two green cells on one
    /// diagonal and red and blue on the other diagonal.
    pub fn is_bayer(&self) -> bool {
        if self.width != 2 || self.height != 2 {
            return false;
        }
        let c = &self.colors;
        let red_blue = |a: CfaColor, b: CfaColor| {
            matches!(
                (a, b),
                (CfaColor::Red, CfaColor::Blue) | (CfaColor::Blue, CfaColor::Red)
            )
        };
        // Greens on the main diagonal (GRBG, GBRG) or on the other
        // diagonal (RGGB, BGGR).
        (c[0] == CfaColor::Green && c[3] == CfaColor::Green && red_blue(c[1], c[2]))
            || (c[1] == CfaColor::Green && c[2] == CfaColor::Green && red_blue(c[0], c[3]))
    }
}

/// How the sensor samples are arranged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SensorLayout {
    /// One sample per pixel behind a color filter array.
    Cfa(CfaPattern),
    /// Three samples (camera RGB) per pixel, already demosaiced.
    LinearRgb,
    /// One sample per pixel, no color filter (monochrome sensor).
    Monochrome,
}

impl SensorLayout {
    /// Returns the number of samples per pixel.
    pub const fn samples_per_pixel(&self) -> usize {
        match self {
            Self::Cfa(_) | Self::Monochrome => 1,
            Self::LinearRgb => 3,
        }
    }
}

/// The sensor samples.
#[derive(Debug, Clone, PartialEq)]
pub enum RawSamples {
    /// Integer samples (most formats).
    U16(Vec<u16>),
    /// Float samples (for example, floating-point DNG).
    F32(Vec<f32>),
}

impl RawSamples {
    /// Returns the number of samples.
    pub fn len(&self) -> usize {
        match self {
            Self::U16(v) => v.len(),
            Self::F32(v) => v.len(),
        }
    }

    /// Returns `true` if there are no samples.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns sample `i` as `f32`, or `None` if `i` is out of range.
    #[inline]
    pub fn get(&self, i: usize) -> Option<f32> {
        match self {
            Self::U16(v) => v.get(i).map(|&s| f32::from(s)),
            Self::F32(v) => v.get(i).copied(),
        }
    }
}

/// Black or white levels: a repeating pattern of values per sample.
///
/// The value for pixel `(x, y)` and sample `c` is
/// `values[((y % height) * width + (x % width)) * samples + c]`. The
/// pattern origin is the top-left pixel of the full sensor data.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelPattern {
    width: u32,
    height: u32,
    samples: u32,
    values: Vec<f32>,
}

impl LevelPattern {
    /// Makes a pattern. Returns an error if the size is zero, larger than
    /// [`MAX_CFA_SIDE`], or does not agree with `values.len()`, or if a
    /// value is not finite.
    pub fn new(width: u32, height: u32, samples: u32, values: Vec<f32>) -> DecodeResult<Self> {
        if width == 0
            || height == 0
            || samples == 0
            || samples > 4
            || width > MAX_CFA_SIDE
            || height > MAX_CFA_SIDE
        {
            return Err(DecodeError::invalid(format!(
                "level pattern {width}x{height}x{samples}"
            )));
        }
        if values.len() != (width * height * samples) as usize {
            return Err(DecodeError::invalid(format!(
                "level pattern {width}x{height}x{samples} has {} values",
                values.len()
            )));
        }
        if values.iter().any(|v| !v.is_finite()) {
            return Err(DecodeError::invalid("level value is not finite"));
        }
        Ok(Self {
            width,
            height,
            samples,
            values,
        })
    }

    /// Makes a pattern with one value for all pixels and `samples` samples.
    pub fn uniform(value: f32, samples: u32) -> DecodeResult<Self> {
        Self::new(1, 1, samples, vec![value; samples as usize])
    }

    /// Returns the pattern width.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the pattern height.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the value for pixel `(x, y)` and sample `c`. The sample
    /// index is reduced modulo the sample count.
    #[inline]
    pub fn at(&self, x: usize, y: usize, c: usize) -> f32 {
        let (w, h, s) = (
            self.width as usize,
            self.height as usize,
            self.samples as usize,
        );
        // The constructor proved values.len() == w * h * s.
        self.values[((y % h) * w + (x % w)) * s + (c % s)]
    }
}

/// The as-shot white balance that the camera recorded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AsShotWhite {
    /// Camera-native RGB values of a neutral (gray) object. A larger value
    /// means a more sensitive channel. All values are finite and > 0.
    Neutral([f64; 3]),
    /// The file has no usable white balance data.
    Unknown,
}

/// One camera color calibration: a matrix from CIE XYZ to camera RGB that
/// is valid under one calibration illuminant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorCalibration {
    /// The illuminant as an EXIF light source code (for example 17 =
    /// standard light A, 21 = D65). See [`illuminant_kelvin`].
    pub illuminant: u16,
    /// Matrix from CIE XYZ to camera RGB, with the DNG `ColorMatrix`
    /// normalization (the illuminant white maps to a camera value with a
    /// largest component of about 1).
    pub xyz_to_camera: Mat3,
}

/// Sensor data and the data that the develop step needs.
///
/// All coordinates are in pixels of the full sensor data, with the origin
/// at the top-left of `samples`.
#[derive(Debug, Clone, PartialEq)]
pub struct RawImage {
    /// Width of the full sensor data, in pixels.
    pub width: u32,
    /// Height of the full sensor data, in pixels.
    pub height: u32,
    /// Sample arrangement.
    pub layout: SensorLayout,
    /// Sensor samples, row-major, `width * height * samples_per_pixel`.
    pub samples: RawSamples,
    /// Black levels (the sample value of no light).
    pub black: LevelPattern,
    /// White levels (the sample value of sensor saturation).
    pub white: LevelPattern,
    /// Bits per sample of the sensor data, as the file declares it.
    pub bits_per_sample: u32,
    /// As-shot white balance.
    pub as_shot: AsShotWhite,
    /// Color calibrations. One or more entries.
    pub calibrations: Vec<ColorCalibration>,
    /// The area of the sensor that received light.
    pub active_area: Rect,
    /// The recommended crop (inside the active area).
    pub crop: Rect,
    /// Orientation from the file metadata.
    pub orientation: Orientation,
    /// Camera maker name.
    pub make: String,
    /// Camera model name.
    pub model: String,
    /// Name of the backend that decoded the file.
    pub backend: &'static str,
}

impl RawImage {
    /// Checks that all fields agree with each other: the sample count, the
    /// active area and crop are inside the image, and the levels are
    /// valid. Backends must call this before they return an image.
    pub fn validate(&self) -> DecodeResult<()> {
        let spp = self.layout.samples_per_pixel();
        let expected = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|n| n.checked_mul(spp))
            .ok_or_else(|| DecodeError::invalid("raw size overflows"))?;
        if self.samples.len() != expected {
            return Err(DecodeError::invalid(format!(
                "raw data has {} samples, expected {expected}",
                self.samples.len()
            )));
        }
        let bounds = Rect::from_size(self.width, self.height);
        if self.active_area.is_empty() || !bounds.contains_rect(&self.active_area) {
            return Err(DecodeError::invalid("active area is outside the image"));
        }
        if self.crop.is_empty() || !self.active_area.contains_rect(&self.crop) {
            return Err(DecodeError::invalid("crop is outside the active area"));
        }
        if let SensorLayout::Cfa(p) = &self.layout {
            if self.active_area.width < p.width() || self.active_area.height < p.height() {
                return Err(DecodeError::invalid(
                    "active area is smaller than the CFA pattern",
                ));
            }
        }
        if self.calibrations.is_empty() && self.layout != SensorLayout::Monochrome {
            return Err(DecodeError::unsupported(
                "the file has no camera color matrix",
            ));
        }
        Ok(())
    }
}

/// A RAW decode backend.
///
/// A backend reads a file and returns the sensor data. It does no color
/// processing and no demosaic. Backends must return errors for bad input
/// and must not panic.
pub trait RawDecoder: Send + Sync {
    /// Returns the name of the backend.
    fn name(&self) -> &'static str;

    /// Returns `true` if the backend can try to decode files of `format`.
    fn supports(&self, format: ImageFormat) -> bool;

    /// Returns `true` if the backend recognizes `data` as a file that it
    /// can decode. [`decode_raw`] calls this only for data whose format
    /// [`detect_format`] does not know (rare RAW containers). The default
    /// is `false`.
    fn probe(&self, data: &[u8]) -> bool {
        let _ = data;
        false
    }

    /// Decodes `data`. The backend must apply `limits` before it allocates
    /// memory for the pixels, where the format makes this possible.
    fn decode(&self, data: &[u8], limits: &Limits) -> DecodeResult<RawImage>;
}

/// Returns the RAW backends of this build, in order of preference.
pub fn raw_decoders() -> Vec<Box<dyn RawDecoder>> {
    vec![
        #[cfg(feature = "rawler")]
        Box::new(RawlerDecoder),
    ]
}

/// Decodes a RAW file to sensor data with the first backend that supports
/// its format.
///
/// If [`detect_format`] does not know the format, the first backend whose
/// [`RawDecoder::probe`] accepts the data is used.
pub fn decode_raw(data: &[u8], limits: &Limits) -> DecodeResult<RawImage> {
    let Some(format) = detect_format(data) else {
        let backend = raw_decoders()
            .into_iter()
            .find(|d| d.probe(data))
            .ok_or(DecodeError::UnknownFormat)?;
        let raw = backend.decode(data, limits)?;
        raw.validate()?;
        return Ok(raw);
    };
    if !format.is_raw() {
        return Err(DecodeError::invalid(format!(
            "{format:?} is not a camera RAW format"
        )));
    }
    let backend = raw_decoders()
        .into_iter()
        .find(|d| d.supports(format))
        .ok_or(DecodeError::UnsupportedFormat {
            format,
            reason: "no RAW backend in this build supports this format",
        })?;
    let raw = backend.decode(data, limits)?;
    raw.validate()?;
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cfa_pattern_validation() {
        let p = CfaPattern::bayer("RGGB").unwrap();
        assert!(p.is_bayer());
        assert_eq!(p.color_at(0, 0), CfaColor::Red);
        assert_eq!(p.color_at(1, 0), CfaColor::Green);
        assert_eq!(p.color_at(1, 1), CfaColor::Blue);
        assert_eq!(p.color_at(3, 2), CfaColor::Green);
        assert!(CfaPattern::bayer("GBRG").unwrap().is_bayer());
        assert!(!CfaPattern::bayer("RGBG").unwrap().is_bayer());
        assert!(CfaPattern::bayer("RGGG").is_err());
        assert!(CfaPattern::bayer("RGGBX").is_err());
        assert!(CfaPattern::new(0, 2, vec![]).is_err());
        assert!(CfaPattern::new(17, 1, vec![CfaColor::Red; 17]).is_err());
    }

    #[test]
    fn level_pattern_lookup() {
        let l = LevelPattern::new(2, 2, 1, vec![1.0, 2.0, 3.0, 4.0]).unwrap();
        assert!((l.at(0, 0, 0) - 1.0).abs() < f32::EPSILON);
        assert!((l.at(3, 1, 0) - 4.0).abs() < f32::EPSILON);
        assert!(LevelPattern::new(2, 2, 1, vec![1.0]).is_err());
        assert!(LevelPattern::new(1, 1, 1, vec![f32::NAN]).is_err());
        let u = LevelPattern::uniform(5.0, 3).unwrap();
        assert!((u.at(9, 9, 2) - 5.0).abs() < f32::EPSILON);
    }

    #[test]
    fn decode_raw_rejects_non_raw() {
        let l = Limits::default();
        assert!(matches!(
            decode_raw(b"nothing", &l),
            Err(DecodeError::UnknownFormat)
        ));
        assert!(decode_raw(&[0xFF, 0xD8, 0xFF, 0xE0], &l).is_err());
    }
}
