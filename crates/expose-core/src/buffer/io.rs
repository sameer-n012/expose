//! Integer image buffers for decode and encode (8-bit and 16-bit).

use crate::buffer::{Rect, checked_len};
use crate::error::{CoreError, CoreResult};

mod sealed {
    pub trait Sealed {}
    impl Sealed for u8 {}
    impl Sealed for u16 {}
}

/// An integer sample type of an I/O image: `u8` or `u16`.
///
/// The full range `0..=MAX` maps to `0.0..=1.0`. This trait is sealed.
pub trait Sample:
    bytemuck::Pod + Default + Copy + PartialEq + Eq + std::fmt::Debug + sealed::Sealed
{
    /// The largest value.
    const MAX: Self;
    /// The number of distinct values (`MAX + 1`).
    const LEVELS: usize;
    /// `MAX` as `f32`.
    const MAX_F32: f32;

    /// Returns the value as an index in `0..LEVELS`.
    fn to_index(self) -> usize;

    /// Converts to a unit value in `0.0..=1.0`.
    fn to_unit(self) -> f32;

    /// Converts a unit value to a sample: clamp to `0.0..=1.0`, scale, and
    /// round to the nearest value. NaN gives 0.
    fn from_unit(v: f32) -> Self;
}

/// Clamps to `0.0..=1.0`, with NaN mapped to 0.0.
#[inline]
pub(crate) fn clamp_unit(v: f32) -> f32 {
    // `f32::clamp` keeps NaN, so map NaN explicitly.
    if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }
}

impl Sample for u8 {
    const MAX: Self = u8::MAX;
    const LEVELS: usize = 256;
    const MAX_F32: f32 = 255.0;

    #[inline]
    fn to_index(self) -> usize {
        usize::from(self)
    }

    #[inline]
    fn to_unit(self) -> f32 {
        f32::from(self) / Self::MAX_F32
    }

    #[inline]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the value is clamped to 0.0..=255.0 and rounded, so the cast is exact"
    )]
    fn from_unit(v: f32) -> Self {
        (clamp_unit(v) * Self::MAX_F32).round() as u8
    }
}

impl Sample for u16 {
    const MAX: Self = u16::MAX;
    const LEVELS: usize = 65536;
    const MAX_F32: f32 = 65535.0;

    #[inline]
    fn to_index(self) -> usize {
        usize::from(self)
    }

    #[inline]
    fn to_unit(self) -> f32 {
        f32::from(self) / Self::MAX_F32
    }

    #[inline]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the value is clamped to 0.0..=65535.0 and rounded, so the cast is exact"
    )]
    fn from_unit(v: f32) -> Self {
        (clamp_unit(v) * Self::MAX_F32).round() as u16
    }
}

/// The channels of an I/O image, in interleaved order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelLayout {
    /// One gray channel.
    Gray,
    /// Gray, then alpha.
    GrayAlpha,
    /// Red, green, blue.
    Rgb,
    /// Red, green, blue, alpha.
    Rgba,
}

impl ChannelLayout {
    /// Returns the number of channels.
    pub const fn channels(self) -> usize {
        match self {
            Self::Gray => 1,
            Self::GrayAlpha => 2,
            Self::Rgb => 3,
            Self::Rgba => 4,
        }
    }

    /// Returns `true` if the layout has an alpha channel (always the last
    /// channel).
    pub const fn has_alpha(self) -> bool {
        matches!(self, Self::GrayAlpha | Self::Rgba)
    }

    /// Returns `true` if the layout has one gray channel instead of RGB.
    pub const fn is_gray(self) -> bool {
        matches!(self, Self::Gray | Self::GrayAlpha)
    }
}

/// An interleaved, row-major integer image for decode and encode.
///
/// - Samples are encoded values (with the transfer function of their color
///   space applied).
/// - Alpha is straight (not premultiplied) and linear.
/// - There is no row padding.
///
/// Width and height are always at least 1. Use the aliases [`Image8`] and
/// [`Image16`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image<T: Sample> {
    width: u32,
    height: u32,
    layout: ChannelLayout,
    data: Vec<T>,
}

/// An 8-bit I/O image.
pub type Image8 = Image<u8>;
/// A 16-bit I/O image.
pub type Image16 = Image<u16>;

impl<T: Sample> Image<T> {
    /// Makes an image with all samples set to zero.
    pub fn new(width: u32, height: u32, layout: ChannelLayout) -> CoreResult<Self> {
        let len = checked_len(width, height, layout.channels(), size_of::<T>())?;
        Ok(Self {
            width,
            height,
            layout,
            data: vec![T::default(); len],
        })
    }

    /// Makes an image from interleaved samples.
    ///
    /// Returns [`CoreError::BufferLengthMismatch`] if `data.len()` is not
    /// `width * height * layout.channels()`.
    pub fn from_vec(
        width: u32,
        height: u32,
        layout: ChannelLayout,
        data: Vec<T>,
    ) -> CoreResult<Self> {
        let expected = checked_len(width, height, layout.channels(), size_of::<T>())?;
        if data.len() != expected {
            return Err(CoreError::BufferLengthMismatch {
                expected,
                actual: data.len(),
            });
        }
        Ok(Self {
            width,
            height,
            layout,
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

    /// Returns the channel layout.
    pub const fn layout(&self) -> ChannelLayout {
        self.layout
    }

    /// Returns the rectangle `(0, 0, width, height)`.
    pub const fn bounds(&self) -> Rect {
        Rect::from_size(self.width, self.height)
    }

    /// Returns the number of samples in one row.
    pub fn row_len(&self) -> usize {
        // The constructor proved that the full sample count fits in usize.
        self.width as usize * self.layout.channels()
    }

    /// Returns all samples.
    pub fn as_slice(&self) -> &[T] {
        &self.data
    }

    /// Returns all samples, mutable.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.data
    }

    /// Returns the raw bytes. 16-bit samples are in native byte order.
    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.data)
    }

    /// Consumes the image and returns its samples.
    pub fn into_vec(self) -> Vec<T> {
        self.data
    }

    fn offset(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let c = self.layout.channels();
        Some((y as usize * self.width as usize + x as usize) * c)
    }

    /// Returns the samples of pixel `(x, y)`, or `None` if it is outside the
    /// image.
    pub fn pixel(&self, x: u32, y: u32) -> Option<&[T]> {
        let o = self.offset(x, y)?;
        Some(&self.data[o..o + self.layout.channels()])
    }

    /// Returns the mutable samples of pixel `(x, y)`, or `None` if it is
    /// outside the image.
    pub fn pixel_mut(&mut self, x: u32, y: u32) -> Option<&mut [T]> {
        let o = self.offset(x, y)?;
        let c = self.layout.channels();
        Some(&mut self.data[o..o + c])
    }

    /// Returns the samples of row `y`, or `None` if it is outside the image.
    pub fn row(&self, y: u32) -> Option<&[T]> {
        let o = self.offset(0, y)?;
        Some(&self.data[o..o + self.row_len()])
    }

    /// Returns the mutable samples of row `y`, or `None` if it is outside
    /// the image.
    pub fn row_mut(&mut self, y: u32) -> Option<&mut [T]> {
        let o = self.offset(0, y)?;
        let len = self.row_len();
        Some(&mut self.data[o..o + len])
    }

    /// Returns an iterator over the rows, from top to bottom.
    pub fn rows(&self) -> impl ExactSizeIterator<Item = &[T]> {
        // Width and channel count are at least 1, so the row length is not 0.
        self.data.chunks_exact(self.row_len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_unit_conversion() {
        assert!((255_u8.to_unit() - 1.0).abs() < f32::EPSILON);
        assert!((65535_u16.to_unit() - 1.0).abs() < f32::EPSILON);
        assert_eq!(u8::from_unit(0.5), 128);
        assert_eq!(u8::from_unit(-1.0), 0);
        assert_eq!(u8::from_unit(2.0), 255);
        assert_eq!(u8::from_unit(f32::NAN), 0);
        assert_eq!(u16::from_unit(1.0), 65535);
        for v in 0..=255_u8 {
            assert_eq!(u8::from_unit(v.to_unit()), v);
        }
        for v in (0..=65535_u16).step_by(97) {
            assert_eq!(u16::from_unit(v.to_unit()), v);
        }
    }

    #[test]
    fn layout_properties() {
        assert_eq!(ChannelLayout::GrayAlpha.channels(), 2);
        assert!(ChannelLayout::GrayAlpha.has_alpha());
        assert!(!ChannelLayout::Rgb.has_alpha());
        assert!(ChannelLayout::Gray.is_gray());
        assert!(!ChannelLayout::Rgba.is_gray());
    }

    #[test]
    fn construction_and_access() {
        let data: Vec<u8> = (0..18).collect();
        let img = Image8::from_vec(3, 2, ChannelLayout::Rgb, data).unwrap();
        assert_eq!(img.pixel(1, 1), Some(&[12_u8, 13, 14][..]));
        assert_eq!(img.row(1).map(<[u8]>::len), Some(9));
        assert_eq!(img.rows().len(), 2);
        assert!(img.pixel(3, 0).is_none());
        assert!(matches!(
            Image8::from_vec(3, 2, ChannelLayout::Rgba, vec![0; 18]),
            Err(CoreError::BufferLengthMismatch {
                expected: 24,
                actual: 18
            })
        ));
        assert!(Image16::new(0, 1, ChannelLayout::Gray).is_err());
        let img16 = Image16::new(2, 2, ChannelLayout::GrayAlpha).unwrap();
        assert_eq!(img16.as_bytes().len(), 16);
    }
}
