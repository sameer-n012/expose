//! Image buffers.
//!
//! - [`WorkingImage`]: interleaved RGBA `f32` in linear Rec. 2020 (D65).
//!   All processing uses this format (ADR 0002).
//! - [`Image`] (with the aliases [`Image8`] and [`Image16`]): integer
//!   buffers for decode and encode, with a [`ChannelLayout`].
//! - [`Rect`]: a pixel rectangle for tiles and crops.
//! - [`resize`] and [`fit_size`]: high-quality resampling (Lanczos-3 in
//!   linear light) and output size computation for export.
//! - [`trace_contours`]: outline polygons ([`Contour`]) of a [`GrayImage`]
//!   (marching squares), for example for the selection outline.
//!
//! Convert I/O images to the working format with [`Image::to_working`], and
//! back with [`WorkingImage::to_image`].

mod contour;
mod convert;
mod encode_lut;
mod gray;
mod io;
mod orientation;
mod rect;
mod resample;
mod working;

pub use contour::{Contour, trace_contours};
pub use gray::GrayImage;
pub use io::{ChannelLayout, Image, Image8, Image16, Sample};
pub use orientation::Orientation;
pub use rect::Rect;
pub use resample::{AlphaMode, ResizeMode, fit_size, resize};
pub use working::{Rgba, WORKING_CHANNELS, WorkingImage};

use crate::error::{CoreError, CoreResult};

/// Returns `width * height * channels` with checked arithmetic.
///
/// - Returns [`CoreError::InvalidDimensions`] if `width` or `height` is 0.
/// - Returns [`CoreError::SizeOverflow`] if the sample count does not fit in
///   `usize`, or the byte size (`count * sample_bytes`) is more than
///   `isize::MAX` (the limit for one allocation).
fn checked_len(width: u32, height: u32, channels: usize, sample_bytes: usize) -> CoreResult<usize> {
    if width == 0 || height == 0 {
        return Err(CoreError::InvalidDimensions { width, height });
    }
    let overflow = CoreError::SizeOverflow {
        width,
        height,
        channels,
    };
    let w = usize::try_from(width).map_err(|_| overflow.clone())?;
    let h = usize::try_from(height).map_err(|_| overflow.clone())?;
    let len = w
        .checked_mul(h)
        .and_then(|n| n.checked_mul(channels))
        .ok_or_else(|| overflow.clone())?;
    let bytes = len
        .checked_mul(sample_bytes)
        .ok_or_else(|| overflow.clone())?;
    if isize::try_from(bytes).is_err() {
        return Err(overflow);
    }
    Ok(len)
}
