//! Size limits for untrusted input.

use crate::error::{DecodeError, DecodeResult};

/// The default maximum number of pixels in one decoded image.
///
/// 200 megapixels is more than the largest current camera sensors (about
/// 150 megapixels). In the working format (16 bytes per pixel), an image at
/// this limit needs 3.2 GB. A file that declares a larger size is rejected
/// before the decoder allocates memory for it.
pub const DEFAULT_MAX_PIXELS: u64 = 200_000_000;

/// Limits that the decoders apply to untrusted input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The maximum number of pixels (`width * height`) in one image.
    pub max_pixels: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_pixels: DEFAULT_MAX_PIXELS,
        }
    }
}

impl Limits {
    /// Returns an error if `width * height` is zero or more than
    /// [`Limits::max_pixels`].
    pub fn check(&self, width: u64, height: u64) -> DecodeResult<()> {
        if width == 0 || height == 0 {
            return Err(DecodeError::invalid(format!(
                "image size {width}x{height} has a zero dimension"
            )));
        }
        // u64 * u64 can overflow; a checked multiply treats overflow as too large.
        match width.checked_mul(height) {
            Some(n) if n <= self.max_pixels => Ok(()),
            _ => Err(DecodeError::TooLarge {
                width,
                height,
                max_pixels: self.max_pixels,
            }),
        }
    }

    /// Returns the allocation limit in bytes for the standard format
    /// decoder: `max_pixels` at 16 bytes per pixel (RGBA, 32-bit float).
    pub(crate) fn max_alloc_bytes(self) -> u64 {
        self.max_pixels.saturating_mul(16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_rejects_zero_and_large() {
        let l = Limits { max_pixels: 100 };
        assert!(l.check(10, 10).is_ok());
        assert!(matches!(l.check(10, 11), Err(DecodeError::TooLarge { .. })));
        assert!(l.check(0, 10).is_err());
        assert!(matches!(
            l.check(u64::MAX, u64::MAX),
            Err(DecodeError::TooLarge { .. })
        ));
    }
}
