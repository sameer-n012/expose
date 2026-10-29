//! Color sampling: the average color of a small square of the canvas.

use std::fmt::Write;

use expose_core::buffer::Rect;
use expose_core::color::ColorSpaceId;
use expose_editor::Document;
use expose_editor::composite::composite_straight;
use serde::{Deserialize, Serialize};

use super::EditorEngineError;

/// Where `editor_sample_color` reads the pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SampleSourceDto {
    /// The composite of all visible layers.
    Composite,
    /// The pixels of one pixel layer (`layerId` is required).
    Layer,
}

/// The result of a color sample.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SampledColor {
    /// The color as `#rrggbb` (lowercase, sRGB), or `None` if the mean
    /// alpha is 0.
    pub color: Option<String>,
    /// The mean alpha of the region, 0 to 1.
    pub alpha: f32,
}

/// The side lengths that a sample accepts.
const SIZES: [u32; 3] = [1, 3, 5];

/// Samples the average color around the canvas pixel (`x`, `y`).
///
/// The region is a square of `size` pixels centred on the pixel, clipped
/// to the canvas. The mean is alpha-weighted: the color is
/// `sum(a * c) / sum(a)` and the alpha is `mean(a)`. The function reads
/// the document on the CPU only.
///
/// # Errors
///
/// - [`EditorEngineError::InvalidSample`] if `size` is not 1, 3 or 5, the
///   pixel is outside the canvas, `source` is `Layer` and `layer_number` is
///   `None`, or the layer is not a pixel layer.
/// - [`EditorEngineError::LayerNotFound`] if the layer does not exist.
/// - [`EditorEngineError::Editor`] if the composite fails.
pub fn sample_color(
    document: &Document,
    x: u32,
    y: u32,
    size: u32,
    source: SampleSourceDto,
    layer_number: Option<u64>,
) -> Result<SampledColor, EditorEngineError> {
    if !SIZES.contains(&size) {
        return Err(EditorEngineError::InvalidSample(
            "the sample size must be 1, 3 or 5",
        ));
    }
    let (width, height) = (document.width(), document.height());
    if x >= width || y >= height {
        return Err(EditorEngineError::InvalidSample(
            "the sample point is outside the canvas",
        ));
    }
    let half = size / 2;
    let x0 = x.saturating_sub(half);
    let y0 = y.saturating_sub(half);
    // `x < width`, so `width - 1` does not underflow.
    let x1 = (x + half).min(width - 1);
    let y1 = (y + half).min(height - 1);
    let region = Rect {
        x: x0,
        y: y0,
        width: x1 - x0 + 1,
        height: y1 - y0 + 1,
    };
    let count = f64::from(region.width) * f64::from(region.height);

    // Sum of straight color times alpha, and sum of alpha.
    let mut weighted = [0.0_f64; 3];
    let mut alpha_sum = 0.0_f64;
    match source {
        SampleSourceDto::Composite => {
            let image = composite_straight(document, region)?;
            for p in image.pixels() {
                let a = f64::from(p[3]);
                alpha_sum += a;
                for (w, c) in weighted.iter_mut().zip(p) {
                    *w += a * f64::from(*c);
                }
            }
        }
        SampleSourceDto::Layer => {
            let number = layer_number.ok_or(EditorEngineError::InvalidSample(
                "a layer sample needs a layer ID",
            ))?;
            let id = document
                .layer_by_number(number)
                .ok_or(EditorEngineError::LayerNotFound(number))?;
            let layer = document
                .layer(id)
                .ok_or(EditorEngineError::LayerNotFound(number))?;
            let pixel_layer = layer.as_pixel().ok_or(EditorEngineError::InvalidSample(
                "the layer is not a pixel layer",
            ))?;
            let (ox, oy) = pixel_layer.offset();
            let image = pixel_layer.image();
            for cy in region.y..region.y + region.height {
                for cx in region.x..region.x + region.width {
                    let lx = i64::from(cx) - i64::from(ox);
                    let ly = i64::from(cy) - i64::from(oy);
                    // Pixels outside the layer are transparent.
                    let (Ok(lx), Ok(ly)) = (u32::try_from(lx), u32::try_from(ly)) else {
                        continue;
                    };
                    let Some(p) = image.pixel(lx, ly) else {
                        continue;
                    };
                    // Layer pixels hold premultiplied color, which is
                    // already `a * c`.
                    alpha_sum += f64::from(p[3]);
                    for (w, c) in weighted.iter_mut().zip(p) {
                        *w += f64::from(c);
                    }
                }
            }
        }
    }

    // Narrowing to f32 is the precision of the result.
    #[allow(clippy::cast_possible_truncation)]
    let alpha = (alpha_sum / count) as f32;
    if alpha_sum <= 0.0 {
        return Ok(SampledColor {
            color: None,
            alpha: 0.0,
        });
    }
    let linear = weighted.map(|w| w / alpha_sum);
    Ok(SampledColor {
        color: Some(to_hex(linear)?),
        alpha,
    })
}

/// Converts a linear Rec. 2020 color to `#rrggbb` (sRGB, clipped).
fn to_hex(linear: [f64; 3]) -> Result<String, EditorEngineError> {
    let matrix = ColorSpaceId::Srgb
        .from_working()
        .map_err(|_| EditorEngineError::Internal("no working to sRGB matrix"))?;
    let transfer = ColorSpaceId::Srgb.transfer();
    let srgb = matrix.mul_vec(linear);
    let mut out = String::with_capacity(7);
    out.push('#');
    for v in srgb {
        // Narrowing to f32 is the precision of the transfer function.
        #[allow(clippy::cast_possible_truncation)]
        let encoded = transfer.encode((v as f32).clamp(0.0, 1.0)).clamp(0.0, 1.0);
        // The value is in 0 to 255, so the cast is exact.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let byte = (encoded * 255.0).round() as u8;
        // Writing to a `String` cannot fail.
        let _ = write!(out, "{byte:02x}");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    // Test code: a failed setup is a test failure.
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use expose_core::buffer::WorkingImage;
    use expose_editor::{Adjustment, BrightnessContrast, LayerContent, PixelLayer, Placement};

    use super::*;
    use crate::editor::session::build_document;

    /// Linear Rec. 2020 value of pure sRGB red.
    #[allow(clippy::cast_possible_truncation)]
    fn srgb_red() -> [f32; 3] {
        let m = ColorSpaceId::Srgb.to_working().unwrap();
        m.mul_vec([1.0, 0.0, 0.0]).map(|v| v as f32)
    }

    fn red_document(alpha: f32) -> Document {
        let [r, g, b] = srgb_red();
        let image =
            WorkingImage::new_filled(4, 3, [r * alpha, g * alpha, b * alpha, alpha]).unwrap();
        build_document(image).unwrap()
    }

    #[test]
    fn opaque_red_gives_ff0000() {
        let doc = red_document(1.0);
        let c = sample_color(&doc, 1, 1, 1, SampleSourceDto::Composite, None).unwrap();
        assert_eq!(c.color.as_deref(), Some("#ff0000"));
        assert!((c.alpha - 1.0).abs() < 1e-6);
    }

    #[test]
    fn transparent_gives_none() {
        let image = WorkingImage::new_filled(4, 3, [0.0; 4]).unwrap();
        let doc = build_document(image).unwrap();
        let c = sample_color(&doc, 1, 1, 3, SampleSourceDto::Composite, None).unwrap();
        assert_eq!(c.color, None);
        assert!(c.alpha.abs() < f32::EPSILON);
    }

    #[test]
    fn corner_region_is_clipped() {
        let doc = red_document(1.0);
        let c = sample_color(&doc, 0, 0, 5, SampleSourceDto::Composite, None).unwrap();
        assert_eq!(c.color.as_deref(), Some("#ff0000"));
        assert!((c.alpha - 1.0).abs() < 1e-6);
        let c = sample_color(&doc, 3, 2, 3, SampleSourceDto::Composite, None).unwrap();
        assert!((c.alpha - 1.0).abs() < 1e-6);
    }

    #[test]
    fn invalid_inputs_are_errors() {
        let doc = red_document(1.0);
        for (x, y, size) in [(4, 0, 1), (0, 3, 1), (1, 1, 2), (1, 1, 0)] {
            assert!(matches!(
                sample_color(&doc, x, y, size, SampleSourceDto::Composite, None),
                Err(EditorEngineError::InvalidSample(_))
            ));
        }
        assert!(matches!(
            sample_color(&doc, 1, 1, 1, SampleSourceDto::Layer, None),
            Err(EditorEngineError::InvalidSample(_))
        ));
        assert!(matches!(
            sample_color(&doc, 1, 1, 1, SampleSourceDto::Layer, Some(9999)),
            Err(EditorEngineError::LayerNotFound(9999))
        ));
    }

    #[test]
    fn layer_source_uses_the_offset() {
        let mut doc = red_document(1.0);
        let [r, g, b] = srgb_red();
        // A 2x2 half-transparent red layer at (2, 1); premultiplied.
        let image = WorkingImage::new_filled(2, 2, [r * 0.5, g * 0.5, b * 0.5, 0.5]).unwrap();
        let layer = PixelLayer::new(image, (2, 1)).unwrap();
        let (id, _) = doc
            .add_layer("L", LayerContent::Pixel(layer), Placement::root(1))
            .unwrap();
        let number = doc.layer(id).unwrap().id().get();
        let inside = sample_color(&doc, 2, 1, 1, SampleSourceDto::Layer, Some(number)).unwrap();
        assert_eq!(inside.color.as_deref(), Some("#ff0000"));
        assert!((inside.alpha - 0.5).abs() < 1e-6);
        let outside = sample_color(&doc, 0, 0, 1, SampleSourceDto::Layer, Some(number)).unwrap();
        assert_eq!(outside.color, None);
        // 3x3 around (2, 1) covers x 1..=3, y 0..=2: 4 layer pixels of 9.
        let mean = sample_color(&doc, 2, 1, 3, SampleSourceDto::Layer, Some(number)).unwrap();
        assert!((mean.alpha - 0.5 * 4.0 / 9.0).abs() < 1e-6);
        assert_eq!(mean.color.as_deref(), Some("#ff0000"));
    }

    #[test]
    fn adjustment_layer_is_an_error() {
        let mut doc = red_document(1.0);
        let (id, _) = doc
            .add_layer(
                "A",
                LayerContent::Adjustment(Adjustment::BrightnessContrast(
                    BrightnessContrast::default(),
                )),
                Placement::root(1),
            )
            .unwrap();
        let number = doc.layer(id).unwrap().id().get();
        assert!(matches!(
            sample_color(&doc, 1, 1, 1, SampleSourceDto::Layer, Some(number)),
            Err(EditorEngineError::InvalidSample(_))
        ));
    }

    #[test]
    fn dto_json_shapes() {
        let source: SampleSourceDto = serde_json::from_str("\"composite\"").unwrap();
        assert_eq!(source, SampleSourceDto::Composite);
        let none = SampledColor {
            color: None,
            alpha: 0.0,
        };
        assert_eq!(
            serde_json::to_string(&none).unwrap(),
            r#"{"color":null,"alpha":0.0}"#
        );
    }
}
