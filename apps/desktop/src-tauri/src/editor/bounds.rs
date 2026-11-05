//! The content bounds of a pixel layer, for `editor_layer_bounds`.

use expose_editor::Document;

use super::EditorEngineError;
use super::dto::LayerBounds;

/// Returns the canvas rectangle of the pixels with alpha above 0 of the
/// layer with the number `number`, or `None` if all pixels are transparent.
/// The rectangle is not clipped to the canvas.
///
/// # Errors
///
/// - [`EditorEngineError::LayerNotFound`] if the layer does not exist.
/// - The `NotAPixelLayer` editor error if the layer is not a pixel layer.
pub fn layer_bounds(
    document: &Document,
    number: u64,
) -> Result<Option<LayerBounds>, EditorEngineError> {
    let layer = document
        .layer_by_number(number)
        .ok_or(EditorEngineError::LayerNotFound(number))?;
    Ok(document.layer_content_bounds(layer)?.map(|r| LayerBounds {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    }))
}

#[cfg(test)]
mod tests {
    use expose_core::buffer::WorkingImage;
    use expose_editor::EditorError;

    use super::*;
    use crate::editor::dto::{AdjustmentDto, EditDto};
    use crate::editor::session::{EditSession, build_document};

    fn session() -> EditSession {
        let image = WorkingImage::new_filled(4, 3, [0.5, 0.25, 0.125, 1.0]).unwrap();
        EditSession::new(build_document(image).unwrap()).unwrap()
    }

    #[test]
    fn full_layer_has_canvas_bounds_and_empty_layer_has_none() {
        let mut s = session();
        let background = s.document().layers()[0].id().get();
        assert_eq!(
            layer_bounds(s.document(), background).unwrap(),
            Some(LayerBounds {
                x: 0,
                y: 0,
                width: 4,
                height: 3
            })
        );
        s.edit(
            EditDto::AddPixelLayer {
                above_layer_id: None,
            },
            None,
        )
        .unwrap();
        let empty = s.document().layers()[1].id().get();
        assert_eq!(layer_bounds(s.document(), empty).unwrap(), None);
    }

    #[test]
    fn unknown_and_adjustment_layers_are_errors() {
        let mut s = session();
        assert!(matches!(
            layer_bounds(s.document(), 999),
            Err(EditorEngineError::LayerNotFound(999))
        ));
        s.edit(
            EditDto::AddAdjustment {
                above_layer_id: None,
                adjustment: AdjustmentDto::BrightnessContrast {
                    brightness: 0.0,
                    contrast: 0.0,
                },
            },
            None,
        )
        .unwrap();
        let adjustment = s.document().layers()[1].id().get();
        assert!(matches!(
            layer_bounds(s.document(), adjustment),
            Err(EditorEngineError::Editor(EditorError::NotAPixelLayer(_)))
        ));
    }
}
