//! Mask thumbnails: a small gray image of a layer mask over the canvas.

use expose_editor::{Document, Mask};

use super::EditorEngineError;

/// The largest accepted `max_size` of a thumbnail.
pub const MAX_THUMBNAIL_SIZE: u32 = 256;

/// The most samples per thumbnail pixel along one axis. A thumbnail pixel
/// that covers more canvas pixels reads a grid of this size instead of
/// the full area. This bounds the cost to `16 * w * h` reads, and the
/// result is close to the area mean for smooth masks.
const MAX_SAMPLES_PER_AXIS: u32 = 4;

/// A gray thumbnail of a mask over the whole canvas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaskThumbnail {
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
    /// `width * height` bytes, rows top first. 0 hides the layer, 255
    /// shows it.
    pub pixels: Vec<u8>,
}

impl MaskThumbnail {
    /// Encodes the thumbnail for the UI: the width and the height as
    /// little-endian `u32`, then the gray bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + self.pixels.len());
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&self.pixels);
        out
    }
}

/// Returns the thumbnail size for a canvas: the long edge is
/// `min(max_size, canvas long edge)`; the short edge keeps the aspect
/// (rounded, at least 1). The canvas size must be 1 or more.
fn thumbnail_size(width: u32, height: u32, max_size: u32) -> (u32, u32) {
    let long = width.max(height);
    let target = max_size.min(long);
    let scaled = |edge: u32| {
        let value = (u64::from(edge) * u64::from(target) + u64::from(long) / 2) / u64::from(long);
        // `edge <= long`, so the value is at most `target`, which is a `u32`.
        u32::try_from(value).unwrap_or(target).max(1)
    };
    (scaled(width), scaled(height))
}

/// The mask value at canvas pixel (`x`, `y`), ignoring the `enabled` flag.
fn raw_value(mask: &Mask, x: i64, y: i64) -> f32 {
    let lx = x - i64::from(mask.offset.0);
    let ly = y - i64::from(mask.offset.1);
    let inside = match (u32::try_from(lx), u32::try_from(ly)) {
        (Ok(mx), Ok(my)) => mask.image.get(mx, my),
        _ => None,
    };
    inside.unwrap_or(mask.fill.value()).clamp(0.0, 1.0)
}

/// The sample positions along one axis for the thumbnail pixel `t` of
/// `count` pixels over `extent` canvas pixels.
fn sample_positions(t: u32, count: u32, extent: u32) -> impl Iterator<Item = i64> {
    let start = u64::from(t) * u64::from(extent) / u64::from(count);
    let end = ((u64::from(t) + 1) * u64::from(extent) / u64::from(count)).max(start + 1);
    let span = end - start;
    let n = span.min(u64::from(MAX_SAMPLES_PER_AXIS));
    // The centre of each of `n` equal parts of the span, as a canvas pixel.
    (0..n).map(move |i| i64::try_from(start + (2 * i + 1) * span / (2 * n)).unwrap_or(i64::MAX))
}

/// Makes a thumbnail of the mask of layer `layer_number`.
///
/// The thumbnail covers the whole canvas. Each pixel is the mean of up to
/// 4 x 4 mask values that are spread evenly over its canvas area. Canvas
/// pixels outside the mask buffer have the fill value of the mask. The
/// `enabled` flag of the mask does not change the values. The function
/// reads the document on the CPU only.
///
/// # Errors
///
/// - [`EditorEngineError::InvalidThumbnailSize`] if `max_size` is not in
///   `1..=256`.
/// - [`EditorEngineError::LayerNotFound`] if the layer does not exist.
/// - [`EditorEngineError::Editor`] (`NoMask`) if the layer has no mask.
pub fn mask_thumbnail(
    document: &Document,
    layer_number: u64,
    max_size: u32,
) -> Result<MaskThumbnail, EditorEngineError> {
    if !(1..=MAX_THUMBNAIL_SIZE).contains(&max_size) {
        return Err(EditorEngineError::InvalidThumbnailSize);
    }
    let id = document
        .layer_by_number(layer_number)
        .ok_or(EditorEngineError::LayerNotFound(layer_number))?;
    let layer = document
        .layer(id)
        .ok_or(EditorEngineError::LayerNotFound(layer_number))?;
    let mask = layer.mask().ok_or(EditorEngineError::Editor(
        expose_editor::EditorError::NoMask(id),
    ))?;

    let (cw, ch) = (document.width(), document.height());
    let (tw, th) = thumbnail_size(cw, ch, max_size);
    let mut pixels = Vec::with_capacity(tw as usize * th as usize);
    for ty in 0..th {
        for tx in 0..tw {
            let mut sum = 0.0_f32;
            let mut count = 0_u32;
            for y in sample_positions(ty, th, ch) {
                for x in sample_positions(tx, tw, cw) {
                    sum += raw_value(mask, x, y);
                    count += 1;
                }
            }
            // `count` is at least 1: every span has one sample or more.
            #[allow(clippy::cast_precision_loss)]
            let mean = sum / count as f32;
            // The mean is in 0 to 1, so the cast is exact.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            pixels.push((mean.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    Ok(MaskThumbnail {
        width: tw,
        height: th,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    // Test code: a failed setup is a test failure.
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use expose_core::buffer::WorkingImage;
    use expose_editor::{EditorError, MaskFill, MaskImage};

    use super::*;
    use crate::editor::session::build_document;

    /// A document of the given size; layer 1 has a mask when `mask` is set.
    fn document(w: u32, h: u32, mask: Option<Mask>) -> (Document, u64) {
        let image = WorkingImage::new_filled(w, h, [1.0; 4]).unwrap();
        let mut doc = build_document(image).unwrap();
        let id = doc.layer_by_number(1).unwrap();
        if mask.is_some() {
            doc.set_mask(id, mask).unwrap();
        }
        (doc, id.get())
    }

    fn white_mask() -> Mask {
        let image = MaskImage::new_filled(1, 1, 1.0).unwrap();
        Mask::new(image, (0, 0), MaskFill::White)
    }

    #[test]
    fn size_keeps_the_aspect() {
        assert_eq!(thumbnail_size(400, 200, 100), (100, 50));
        assert_eq!(thumbnail_size(200, 400, 100), (50, 100));
        // The canvas is smaller than `max_size`.
        assert_eq!(thumbnail_size(40, 20, 100), (40, 20));
        // The short edge is at least 1.
        assert_eq!(thumbnail_size(1000, 2, 10), (10, 1));
    }

    #[test]
    fn thumbnail_has_the_computed_size() {
        let (doc, n) = document(40, 20, Some(white_mask()));
        let t = mask_thumbnail(&doc, n, 10).unwrap();
        assert_eq!((t.width, t.height), (10, 5));
        assert_eq!(t.pixels.len(), 50);
        assert!(t.pixels.iter().all(|&p| p == 255));
        let small = mask_thumbnail(&doc, n, 256).unwrap();
        assert_eq!((small.width, small.height), (40, 20));
    }

    #[test]
    fn fill_applies_outside_the_buffer() {
        for (fill, expected) in [(MaskFill::White, 255), (MaskFill::Black, 0)] {
            // A 1x1 mask at (0, 0) of the opposite value; the rest is fill.
            let value = if expected == 255 { 0.0 } else { 1.0 };
            let image = MaskImage::new_filled(1, 1, value).unwrap();
            let mask = Mask::new(image, (0, 0), fill);
            let (doc, n) = document(4, 4, Some(mask));
            let t = mask_thumbnail(&doc, n, 4).unwrap();
            assert_eq!(t.pixels[0], 255 - expected);
            assert_eq!(t.pixels[5], expected);
            assert_eq!(t.pixels[15], expected);
        }
    }

    #[test]
    fn offset_buffer_is_placed_on_the_canvas() {
        let image = MaskImage::new_filled(2, 2, 1.0).unwrap();
        let mask = Mask::new(image, (2, 2), MaskFill::Black);
        let (doc, n) = document(4, 4, Some(mask));
        let t = mask_thumbnail(&doc, n, 4).unwrap();
        assert_eq!(t.pixels[0], 0);
        assert_eq!(t.pixels[2 * 4 + 2], 255);
        assert_eq!(t.pixels[3 * 4 + 3], 255);
        assert_eq!(t.pixels[2 * 4 + 1], 0);
    }

    #[test]
    fn half_black_edge_pixel_averages() {
        // 8x2 canvas, thumbnail 4x1: each pixel covers 2x2 canvas pixels.
        // Columns 0..3 are white, 3..8 black, so pixel 1 (columns 2, 3)
        // is half white.
        let mut data = vec![0.0; 16];
        for row in 0..2 {
            for x in 0..3 {
                data[row * 8 + x] = 1.0;
            }
        }
        let image = MaskImage::from_data(8, 2, data).unwrap();
        let mask = Mask::new(image, (0, 0), MaskFill::Black);
        let (doc, n) = document(8, 2, Some(mask));
        let t = mask_thumbnail(&doc, n, 4).unwrap();
        assert_eq!((t.width, t.height), (4, 1));
        assert_eq!(t.pixels, vec![255, 128, 0, 0]);
    }

    #[test]
    fn disabled_flag_does_not_change_values() {
        let mut mask = white_mask();
        mask.enabled = false;
        let (doc, n) = document(4, 4, Some(mask));
        let t = mask_thumbnail(&doc, n, 4).unwrap();
        assert!(t.pixels.iter().all(|&p| p == 255));
    }

    #[test]
    fn bad_input_is_refused() {
        let (doc, n) = document(4, 4, Some(white_mask()));
        for size in [0, 257] {
            assert!(matches!(
                mask_thumbnail(&doc, n, size),
                Err(EditorEngineError::InvalidThumbnailSize)
            ));
        }
        assert!(matches!(
            mask_thumbnail(&doc, n + 99, 4),
            Err(EditorEngineError::LayerNotFound(_))
        ));
        let (plain, n) = document(4, 4, None);
        assert!(matches!(
            mask_thumbnail(&plain, n, 4),
            Err(EditorEngineError::Editor(EditorError::NoMask(_)))
        ));
    }

    #[test]
    fn header_encoding() {
        let t = MaskThumbnail {
            width: 3,
            height: 2,
            pixels: vec![1, 2, 3, 4, 5, 6],
        };
        assert_eq!(t.encode(), vec![3, 0, 0, 0, 2, 0, 0, 0, 1, 2, 3, 4, 5, 6]);
    }

    /// Timing at 4096x4096; run with `--release --ignored --nocapture`.
    #[test]
    #[ignore = "timing probe"]
    fn timing_at_4096() {
        let image = MaskImage::new_filled(4096, 4096, 0.5).unwrap();
        let mask = Mask::new(image, (0, 0), MaskFill::White);
        let (doc, n) = document(4096, 4096, Some(mask));
        let start = std::time::Instant::now();
        let t = mask_thumbnail(&doc, n, 256).unwrap();
        println!(
            "mask thumbnail 4096^2 -> {}x{}: {:?}",
            t.width,
            t.height,
            start.elapsed()
        );
    }
}
