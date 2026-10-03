//! EXIF orientation and its application to working images.

use serde::{Deserialize, Serialize};

use super::{Rgba, WorkingImage};
use crate::error::CoreResult;

/// The EXIF / TIFF orientation (tag 0x0112).
///
/// The value tells how to transform the stored pixels for correct display.
/// The serialized form is the EXIF number (1 to 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(into = "u16", try_from = "u16")]
pub enum Orientation {
    /// 1: no change.
    #[default]
    Normal,
    /// 2: mirror left to right.
    FlipHorizontal,
    /// 3: rotate 180 degrees.
    Rotate180,
    /// 4: mirror top to bottom.
    FlipVertical,
    /// 5: mirror along the top-left to bottom-right diagonal.
    Transpose,
    /// 6: rotate 90 degrees clockwise.
    Rotate90,
    /// 7: mirror along the top-right to bottom-left diagonal.
    Transverse,
    /// 8: rotate 270 degrees clockwise (90 degrees counter-clockwise).
    Rotate270,
}

impl Orientation {
    /// Converts the EXIF value (1 to 8). Returns `None` for other values.
    pub const fn from_exif(v: u16) -> Option<Self> {
        Some(match v {
            1 => Self::Normal,
            2 => Self::FlipHorizontal,
            3 => Self::Rotate180,
            4 => Self::FlipVertical,
            5 => Self::Transpose,
            6 => Self::Rotate90,
            7 => Self::Transverse,
            8 => Self::Rotate270,
            _ => return None,
        })
    }

    /// Returns the EXIF value (1 to 8).
    pub const fn to_exif(self) -> u16 {
        match self {
            Self::Normal => 1,
            Self::FlipHorizontal => 2,
            Self::Rotate180 => 3,
            Self::FlipVertical => 4,
            Self::Transpose => 5,
            Self::Rotate90 => 6,
            Self::Transverse => 7,
            Self::Rotate270 => 8,
        }
    }

    /// Returns `true` if the transform swaps width and height.
    pub const fn swaps_dimensions(self) -> bool {
        matches!(
            self,
            Self::Transpose | Self::Rotate90 | Self::Transverse | Self::Rotate270
        )
    }

    /// Applies the transform to `img` and returns the display-oriented
    /// image.
    ///
    /// # Errors
    ///
    /// Returns an error only if the output buffer cannot be made. This
    /// cannot happen for a valid input image.
    pub fn apply(self, img: WorkingImage) -> CoreResult<WorkingImage> {
        if self == Self::Normal {
            return Ok(img);
        }
        let (w, h) = (img.width() as usize, img.height() as usize);
        let (ow, oh) = if self.swaps_dimensions() {
            (h, w)
        } else {
            (w, h)
        };
        let src = img.pixels();
        let mut out: Vec<Rgba> = Vec::with_capacity(src.len());
        // For each output pixel (x, y), compute the source pixel. All
        // indices are inside the image because x < ow and y < oh.
        for y in 0..oh {
            for x in 0..ow {
                let (sx, sy) = match self {
                    Self::Normal => (x, y),
                    Self::FlipHorizontal => (w - 1 - x, y),
                    Self::Rotate180 => (w - 1 - x, h - 1 - y),
                    Self::FlipVertical => (x, h - 1 - y),
                    Self::Transpose => (y, x),
                    Self::Rotate90 => (y, h - 1 - x),
                    Self::Transverse => (w - 1 - y, h - 1 - x),
                    Self::Rotate270 => (w - 1 - y, x),
                };
                out.push(src[sy * w + sx]);
            }
        }
        // ow and oh are the input dimensions (u32), maybe swapped.
        let (ow, oh) = if self.swaps_dimensions() {
            (img.height(), img.width())
        } else {
            (img.width(), img.height())
        };
        WorkingImage::from_pixels(ow, oh, out)
    }
}

impl From<Orientation> for u16 {
    fn from(o: Orientation) -> Self {
        o.to_exif()
    }
}

impl TryFrom<u16> for Orientation {
    type Error = String;

    fn try_from(v: u16) -> Result<Self, Self::Error> {
        Self::from_exif(v).ok_or_else(|| format!("invalid orientation value {v}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3x2 image where the red channel is the pixel index.
    fn sample() -> WorkingImage {
        let px = (0..6_u8).map(|i| [f32::from(i), 0.0, 0.0, 1.0]).collect();
        WorkingImage::from_pixels(3, 2, px).unwrap()
    }

    fn reds(img: &WorkingImage) -> Vec<f32> {
        img.pixels().iter().map(|p| p[0]).collect()
    }

    #[test]
    fn all_orientations() {
        // Source:
        // 0 1 2
        // 3 4 5
        let cases: [(Orientation, (u32, u32), [f32; 6]); 8] = [
            (Orientation::Normal, (3, 2), [0., 1., 2., 3., 4., 5.]),
            (
                Orientation::FlipHorizontal,
                (3, 2),
                [2., 1., 0., 5., 4., 3.],
            ),
            (Orientation::Rotate180, (3, 2), [5., 4., 3., 2., 1., 0.]),
            (Orientation::FlipVertical, (3, 2), [3., 4., 5., 0., 1., 2.]),
            (Orientation::Transpose, (2, 3), [0., 3., 1., 4., 2., 5.]),
            (Orientation::Rotate90, (2, 3), [3., 0., 4., 1., 5., 2.]),
            (Orientation::Transverse, (2, 3), [5., 2., 4., 1., 3., 0.]),
            (Orientation::Rotate270, (2, 3), [2., 5., 1., 4., 0., 3.]),
        ];
        for (o, dims, expected) in cases {
            let out = o.apply(sample()).unwrap();
            assert_eq!((out.width(), out.height()), dims, "{o:?}");
            assert_eq!(reds(&out), expected, "{o:?}");
        }
    }

    #[test]
    fn exif_round_trip() {
        for v in 1..=8 {
            assert_eq!(Orientation::from_exif(v).map(Orientation::to_exif), Some(v));
        }
        assert!(Orientation::from_exif(0).is_none());
        assert!(Orientation::from_exif(9).is_none());
        let json = serde_json::to_string(&Orientation::Rotate90).unwrap();
        assert_eq!(json, "6");
    }
}
