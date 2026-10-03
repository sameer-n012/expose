//! The working image: interleaved RGBA `f32` in linear Rec. 2020.

use crate::buffer::{Rect, checked_len};
use crate::error::{CoreError, CoreResult};

/// Number of `f32` samples per working pixel.
pub const WORKING_CHANNELS: usize = 4;

/// One working pixel: `[r, g, b, a]` in linear Rec. 2020 (D65).
pub type Rgba = [f32; 4];

/// Returns the pixel count, and checks that the byte size fits in one
/// allocation. Then `count * WORKING_CHANNELS` cannot overflow either.
fn pixel_count(width: u32, height: u32) -> CoreResult<usize> {
    let samples = checked_len(width, height, WORKING_CHANNELS, size_of::<f32>())?;
    Ok(samples / WORKING_CHANNELS)
}

/// An image in the working format (ADR 0002).
///
/// - Color: linear light, Rec. 2020 primaries, D65 white.
/// - Samples: `f32`. Values below 0.0 and above 1.0 are permitted.
/// - Layout: interleaved RGBA, row-major, no row padding. This maps directly
///   to the GPU format `rgba32float`.
/// - Alpha: the image does not record whether alpha is straight or
///   premultiplied. Conversions from I/O buffers give straight alpha. The
///   editor compositor must call [`WorkingImage::premultiply_alpha`].
///
/// Width and height are always at least 1, and the pixel count always fits
/// in memory addressing (`usize`).
#[derive(Debug, Clone, PartialEq)]
pub struct WorkingImage {
    width: u32,
    height: u32,
    pixels: Vec<Rgba>,
}

impl WorkingImage {
    /// Makes an image with all pixels set to `fill`.
    pub fn new_filled(width: u32, height: u32, fill: Rgba) -> CoreResult<Self> {
        let count = pixel_count(width, height)?;
        Ok(Self {
            width,
            height,
            pixels: vec![fill; count],
        })
    }

    /// Makes an image with all pixels set to transparent black (all zero).
    pub fn new(width: u32, height: u32) -> CoreResult<Self> {
        Self::new_filled(width, height, [0.0; 4])
    }

    /// Makes an image from a vector of pixels in row-major order.
    ///
    /// Returns [`CoreError::BufferLengthMismatch`] if `pixels.len()` is not
    /// `width * height`.
    pub fn from_pixels(width: u32, height: u32, pixels: Vec<Rgba>) -> CoreResult<Self> {
        let expected = pixel_count(width, height)?;
        if pixels.len() != expected {
            return Err(CoreError::BufferLengthMismatch {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// Makes an image from interleaved RGBA samples. The function copies
    /// the data.
    ///
    /// Returns [`CoreError::BufferLengthMismatch`] if `samples.len()` is not
    /// `width * height * 4`.
    pub fn from_interleaved(width: u32, height: u32, samples: &[f32]) -> CoreResult<Self> {
        let expected = pixel_count(width, height)? * WORKING_CHANNELS;
        if samples.len() != expected {
            return Err(CoreError::BufferLengthMismatch {
                expected,
                actual: samples.len(),
            });
        }
        let pixels = samples
            .chunks_exact(WORKING_CHANNELS)
            .map(|c| [c[0], c[1], c[2], c[3]])
            .collect();
        Self::from_pixels(width, height, pixels)
    }

    /// Returns the width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the rectangle `(0, 0, width, height)`.
    pub const fn bounds(&self) -> Rect {
        Rect::from_size(self.width, self.height)
    }

    /// Returns all pixels in row-major order.
    pub fn pixels(&self) -> &[Rgba] {
        &self.pixels
    }

    /// Returns all pixels in row-major order, mutable.
    pub fn pixels_mut(&mut self) -> &mut [Rgba] {
        &mut self.pixels
    }

    /// Returns all samples as one flat `f32` slice (`width * height * 4`).
    pub fn as_slice(&self) -> &[f32] {
        bytemuck::cast_slice(&self.pixels)
    }

    /// Returns all samples as one flat mutable `f32` slice.
    pub fn as_mut_slice(&mut self) -> &mut [f32] {
        bytemuck::cast_slice_mut(&mut self.pixels)
    }

    /// Returns the raw bytes (native byte order) for GPU upload.
    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.pixels)
    }

    /// Consumes the image and returns its pixels.
    pub fn into_pixels(self) -> Vec<Rgba> {
        self.pixels
    }

    fn index(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        // The constructor proved that width * height fits in usize.
        Some(y as usize * self.width as usize + x as usize)
    }

    /// Returns the pixel at `(x, y)`, or `None` if it is outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> Option<Rgba> {
        self.index(x, y).map(|i| self.pixels[i])
    }

    /// Returns a mutable reference to the pixel at `(x, y)`, or `None` if it
    /// is outside the image.
    pub fn pixel_mut(&mut self, x: u32, y: u32) -> Option<&mut Rgba> {
        self.index(x, y).map(|i| &mut self.pixels[i])
    }

    /// Returns row `y`, or `None` if it is outside the image.
    pub fn row(&self, y: u32) -> Option<&[Rgba]> {
        let start = self.index(0, y)?;
        Some(&self.pixels[start..start + self.width as usize])
    }

    /// Returns row `y` mutable, or `None` if it is outside the image.
    pub fn row_mut(&mut self, y: u32) -> Option<&mut [Rgba]> {
        let start = self.index(0, y)?;
        let w = self.width as usize;
        Some(&mut self.pixels[start..start + w])
    }

    /// Returns an iterator over the rows, from top to bottom.
    pub fn rows(&self) -> impl ExactSizeIterator<Item = &[Rgba]> {
        // Width is at least 1, so `chunks_exact` does not panic.
        self.pixels.chunks_exact(self.width as usize)
    }

    /// Returns an iterator over the mutable rows, from top to bottom.
    pub fn rows_mut(&mut self) -> impl ExactSizeIterator<Item = &mut [Rgba]> {
        // Width is at least 1, so `chunks_exact_mut` does not panic.
        self.pixels.chunks_exact_mut(self.width as usize)
    }

    /// Copies the area `rect` into a new image.
    ///
    /// Returns [`CoreError::InvalidRect`] if `rect` is empty or not fully
    /// inside the image.
    pub fn crop(&self, rect: Rect) -> CoreResult<Self> {
        if rect.is_empty() || !self.bounds().contains_rect(&rect) {
            return Err(CoreError::InvalidRect {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
            });
        }
        let (x0, w) = (rect.x as usize, rect.width as usize);
        let mut pixels = Vec::with_capacity(w * rect.height as usize);
        for row in self.rows().skip(rect.y as usize).take(rect.height as usize) {
            pixels.extend_from_slice(&row[x0..x0 + w]);
        }
        Self::from_pixels(rect.width, rect.height, pixels)
    }

    /// Multiplies RGB by alpha in each pixel (straight to premultiplied).
    pub fn premultiply_alpha(&mut self) {
        for p in &mut self.pixels {
            let a = p[3];
            p[0] *= a;
            p[1] *= a;
            p[2] *= a;
        }
    }

    /// Divides RGB by alpha in each pixel (premultiplied to straight).
    ///
    /// If alpha is zero or less, the color is undefined. The function then
    /// sets RGB to zero and keeps alpha.
    pub fn unpremultiply_alpha(&mut self) {
        for p in &mut self.pixels {
            let a = p[3];
            if a > 0.0 {
                let inv = 1.0 / a;
                p[0] *= inv;
                p[1] *= inv;
                p[2] *= inv;
            } else {
                p[0] = 0.0;
                p[1] = 0.0;
                p[2] = 0.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> WorkingImage {
        let mut img = WorkingImage::new(w, h).unwrap();
        for y in 0..h {
            for x in 0..w {
                #[allow(clippy::cast_precision_loss)]
                let v = (y * w + x) as f32;
                *img.pixel_mut(x, y).unwrap() = [v, v, v, 1.0];
            }
        }
        img
    }

    #[test]
    fn constructors_validate_size() {
        assert!(matches!(
            WorkingImage::new(0, 5),
            Err(CoreError::InvalidDimensions { .. })
        ));
        assert!(matches!(
            WorkingImage::from_interleaved(2, 2, &[0.0; 15]),
            Err(CoreError::BufferLengthMismatch {
                expected: 16,
                actual: 15
            })
        ));
        assert!(matches!(
            WorkingImage::from_pixels(2, 2, vec![[0.0; 4]; 3]),
            Err(CoreError::BufferLengthMismatch { .. })
        ));
        let img = WorkingImage::from_interleaved(2, 1, &[1., 2., 3., 4., 5., 6., 7., 8.]).unwrap();
        assert_eq!(img.pixel(1, 0), Some([5., 6., 7., 8.]));
        assert_eq!(img.as_slice().len(), 8);
        assert_eq!(img.as_bytes().len(), 32);
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn huge_size_is_overflow_not_panic() {
        // 2^32-1 squared pixels times 16 bytes does not fit in 64 bits.
        assert!(matches!(
            WorkingImage::new(u32::MAX, u32::MAX),
            Err(CoreError::SizeOverflow { .. })
        ));
    }

    #[test]
    fn pixel_and_row_access() {
        let img = gradient(3, 2);
        assert_eq!(img.pixel(2, 1), Some([5.0, 5.0, 5.0, 1.0]));
        assert_eq!(img.pixel(3, 0), None);
        assert_eq!(img.pixel(0, 2), None);
        assert_eq!(img.row(1).map(<[Rgba]>::len), Some(3));
        assert!(img.row(2).is_none());
        assert_eq!(img.rows().len(), 2);
    }

    #[test]
    fn crop_copies_area() {
        let img = gradient(4, 4);
        let c = img.crop(Rect::new(1, 2, 2, 2)).unwrap();
        assert_eq!((c.width(), c.height()), (2, 2));
        assert_eq!(c.pixel(0, 0).map(|p| p[0]), Some(9.0));
        assert_eq!(c.pixel(1, 1).map(|p| p[0]), Some(14.0));
        assert!(img.crop(Rect::new(3, 3, 2, 1)).is_err());
        assert!(img.crop(Rect::new(0, 0, 0, 1)).is_err());
    }

    #[test]
    fn premultiply_round_trip() {
        let mut img = WorkingImage::from_pixels(
            3,
            1,
            vec![
                [0.8, 0.4, 0.2, 0.5],
                [1.5, -0.5, 0.0, 1.0],
                [0.3, 0.3, 0.3, 0.0],
            ],
        )
        .unwrap();
        img.premultiply_alpha();
        assert_eq!(img.pixel(0, 0), Some([0.4, 0.2, 0.1, 0.5]));
        img.unpremultiply_alpha();
        let p = img.pixel(0, 0).unwrap();
        for (a, b) in p.iter().zip([0.8, 0.4, 0.2, 0.5]) {
            assert!((a - b).abs() < 1e-6);
        }
        assert_eq!(img.pixel(1, 0), Some([1.5, -0.5, 0.0, 1.0]));
        // Zero alpha gives zero color.
        assert_eq!(img.pixel(2, 0), Some([0.0, 0.0, 0.0, 0.0]));
    }
}
