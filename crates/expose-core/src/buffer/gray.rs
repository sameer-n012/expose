//! Single-channel `f32` image.

use super::{Rect, checked_len};
use crate::error::{CoreError, CoreResult};

/// A one-channel `f32` image. Row-major, no padding.
///
/// Use it for masks, alpha planes, and other scalar fields. The type does
/// not limit the value range.
#[derive(Debug, Clone, PartialEq)]
pub struct GrayImage {
    width: u32,
    height: u32,
    data: Vec<f32>,
}

impl GrayImage {
    /// Makes an image with all values set to `fill`.
    ///
    /// # Errors
    ///
    /// Returns an error if the width or height is zero, or if the size
    /// overflows the address space.
    pub fn new_filled(width: u32, height: u32, fill: f32) -> CoreResult<Self> {
        let count = checked_len(width, height, 1, size_of::<f32>())?;
        Ok(Self {
            width,
            height,
            data: vec![fill; count],
        })
    }

    /// Makes an image from values in row-major order.
    ///
    /// # Errors
    ///
    /// Returns an error if the size is not valid or if `data` has the wrong
    /// length.
    pub fn from_data(width: u32, height: u32, data: Vec<f32>) -> CoreResult<Self> {
        let expected = checked_len(width, height, 1, size_of::<f32>())?;
        if data.len() != expected {
            return Err(CoreError::BufferLengthMismatch {
                expected,
                actual: data.len(),
            });
        }
        Ok(Self {
            width,
            height,
            data,
        })
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

    /// Returns all values in row-major order.
    pub fn data(&self) -> &[f32] {
        &self.data
    }

    /// Returns all values in row-major order, for writing.
    pub fn data_mut(&mut self) -> &mut [f32] {
        &mut self.data
    }

    /// Consumes the image and returns the values.
    pub fn into_data(self) -> Vec<f32> {
        self.data
    }

    /// Returns the values of row `y`, or `None` if `y` is outside the image.
    pub fn row(&self, y: u32) -> Option<&[f32]> {
        if y >= self.height {
            return None;
        }
        // The constructor proved that width * height fits in usize.
        let w = self.width as usize;
        let start = y as usize * w;
        self.data.get(start..start + w)
    }

    /// Returns the values of row `y` for writing, or `None` if `y` is
    /// outside the image.
    pub fn row_mut(&mut self, y: u32) -> Option<&mut [f32]> {
        if y >= self.height {
            return None;
        }
        let w = self.width as usize;
        let start = y as usize * w;
        self.data.get_mut(start..start + w)
    }

    /// Returns the value at `(x, y)`, or `None` if it is outside the image.
    pub fn get(&self, x: u32, y: u32) -> Option<f32> {
        if x >= self.width {
            return None;
        }
        self.row(y).and_then(|r| r.get(x as usize)).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_and_get() {
        let g = GrayImage::from_data(2, 2, vec![1.0, 2.0, 3.0, 4.0]).unwrap();
        assert_eq!(g.row(1), Some(&[3.0, 4.0][..]));
        assert_eq!(g.row(2), None);
        assert_eq!(g.get(1, 0), Some(2.0));
        assert_eq!(g.get(2, 0), None);
        assert_eq!(g.bounds(), Rect::from_size(2, 2));
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(GrayImage::new_filled(0, 1, 0.0).is_err());
        assert!(GrayImage::from_data(2, 2, vec![0.0; 3]).is_err());
    }

    #[test]
    fn row_mut_writes() {
        let mut g = GrayImage::new_filled(2, 2, 0.0).unwrap();
        if let Some(r) = g.row_mut(1) {
            r[0] = 5.0;
        }
        assert_eq!(g.get(0, 1), Some(5.0));
    }
}
