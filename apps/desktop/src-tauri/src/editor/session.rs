//! One open document and its history, with no GPU and no thread. The
//! engine thread owns one [`EditSession`] for each open document; tests
//! use it directly.

use expose_core::buffer::WorkingImage;
use expose_editor::{
    Adjustment, Document, DocumentSpec, Edit, History, LayerContent, LayerId, PixelLayer, Placement,
};

use super::EditorEngineError;
use super::dto::{DocumentId, DocumentInfo, EditDto};

/// The maximum number of undo steps of a document. A delete step keeps the
/// removed layer, so the history of a large photo can hold several full
/// layer copies.
pub const HISTORY_STEPS: usize = 50;

/// The name of the photo layer of a new document.
pub const BACKGROUND_NAME: &str = "Background";

/// Makes a document with one pixel layer "Background" that holds `image`.
///
/// `image` is the developed photo with straight alpha (linear Rec. 2020).
/// Editor layers have premultiplied alpha (ADR 0002), so the function
/// premultiplies it (a straighten can give transparent corners).
///
/// # Errors
///
/// [`EditorEngineError::Editor`] if the image is larger than the document
/// limits of the editor.
pub fn build_document(mut image: WorkingImage) -> Result<Document, EditorEngineError> {
    image.premultiply_alpha();
    let spec = DocumentSpec::new(image.width(), image.height());
    // `new_empty` gives a document with no layers. The photo layer is then
    // the only layer, at the canvas origin.
    let mut document = Document::new_empty(spec)?;
    let layer = PixelLayer::new(image, (0, 0))?;
    // The open is not an undo step, so the inverse edit is not kept.
    let (_id, _undo) = document.add_layer(
        BACKGROUND_NAME,
        LayerContent::Pixel(layer),
        Placement::root(0),
    )?;
    Ok(document)
}

/// An open document and its undo history.
#[derive(Debug)]
pub struct EditSession {
    document: Document,
    history: History,
}

impl EditSession {
    /// Starts a session for `document` with an empty history of
    /// [`HISTORY_STEPS`] steps.
    ///
    /// # Errors
    ///
    /// [`EditorEngineError::Internal`] if the history cannot be made (the
    /// step count is a constant larger than 0, so this does not occur).
    pub fn new(document: Document) -> Result<Self, EditorEngineError> {
        let history = History::new(HISTORY_STEPS)
            .map_err(|_| EditorEngineError::Internal("the history step count is 0"))?;
        Ok(Self { document, history })
    }

    /// The document.
    pub const fn document(&self) -> &Document {
        &self.document
    }

    /// The info for the UI, with the document ID `id`.
    pub fn info(&self, id: DocumentId) -> DocumentInfo {
        DocumentInfo::new(
            id,
            &self.document,
            self.history.can_undo(),
            self.history.can_redo(),
        )
    }

    /// Applies `edit` and records it as one history step.
    ///
    /// With `gesture` `Some(key)`, the step merges with the step before it
    /// when that step was also an edit with the same key (one slider drag
    /// is one undo step). With `None`, the edit is always a new step.
    ///
    /// A failed edit changes neither the document nor the history.
    ///
    /// # Errors
    ///
    /// - [`EditorEngineError::LayerNotFound`] if a layer ID of the edit is
    ///   not in the document.
    /// - [`EditorEngineError::Editor`] if the editor rejects the edit (for
    ///   example, an opacity out of range, a lock, or a move into a
    ///   descendant).
    pub fn edit(&mut self, edit: EditDto, gesture: Option<u64>) -> Result<(), EditorEngineError> {
        let name = edit.step_name();
        let undo = apply(&mut self.document, edit)?;
        match gesture {
            Some(key) => self.history.push_merge(name, vec![undo], key),
            None => self.history.push(name, vec![undo]),
        }
        Ok(())
    }

    /// Undoes the last step. Does nothing if there is no step to undo.
    ///
    /// # Errors
    ///
    /// [`EditorEngineError::History`] if the step fails (a bug; the history
    /// restores the state before the step).
    pub fn undo(&mut self) -> Result<(), EditorEngineError> {
        self.history
            .undo(&mut self.document)
            .map(drop)
            .map_err(EditorEngineError::History)
    }

    /// Redoes the next step. Does nothing if there is no step to redo.
    ///
    /// # Errors
    ///
    /// [`EditorEngineError::History`] if the step fails.
    pub fn redo(&mut self) -> Result<(), EditorEngineError> {
        self.history
            .redo(&mut self.document)
            .map(drop)
            .map_err(EditorEngineError::History)
    }
}

/// Finds the layer with the numeric ID `raw`.
///
/// `LayerId` has no public constructor, so the function searches the
/// document. The tree of a document is small (tens of layers), so a linear
/// search is fast enough.
fn resolve(document: &Document, raw: u64) -> Result<LayerId, EditorEngineError> {
    document
        .layer_ids()
        .into_iter()
        .find(|id| id.get() == raw)
        .ok_or(EditorEngineError::LayerNotFound(raw))
}

/// Applies one edit and returns its inverse.
fn apply(document: &mut Document, edit: EditDto) -> Result<Edit, EditorEngineError> {
    let undo = match edit {
        EditDto::SetVisible { layer_id, visible } => {
            let id = resolve(document, layer_id)?;
            document.set_visible(id, visible)?
        }
        EditDto::SetOpacity { layer_id, opacity } => {
            let id = resolve(document, layer_id)?;
            document.set_opacity(id, opacity)?
        }
        EditDto::SetFillOpacity {
            layer_id,
            fill_opacity,
        } => {
            let id = resolve(document, layer_id)?;
            document.set_fill_opacity(id, fill_opacity)?
        }
        EditDto::SetBlendMode { layer_id, mode } => {
            let id = resolve(document, layer_id)?;
            document.set_blend_mode(id, mode.into())?
        }
        EditDto::Rename { layer_id, name } => {
            let id = resolve(document, layer_id)?;
            document.rename_layer(id, name)?
        }
        EditDto::Move {
            layer_id,
            parent_id,
            index,
        } => {
            let id = resolve(document, layer_id)?;
            let parent = parent_id.map(|raw| resolve(document, raw)).transpose()?;
            document.move_layer(id, Placement::new(parent, index))?
        }
        EditDto::Delete { layer_id } => {
            let id = resolve(document, layer_id)?;
            document.delete_layer(id)?
        }
        EditDto::Duplicate { layer_id } => {
            let id = resolve(document, layer_id)?;
            document.duplicate_layer(id)?.1
        }
        EditDto::AddAdjustment {
            adjustment,
            above_layer_id,
        } => {
            let at = match above_layer_id {
                Some(raw) => {
                    let above = resolve(document, raw)?;
                    let placement = document
                        .placement_of(above)
                        .ok_or(EditorEngineError::LayerNotFound(raw))?;
                    Placement::new(placement.parent, placement.index + 1)
                }
                None => Placement::root(document.layers().len()),
            };
            let name = adjustment.layer_name();
            let content = LayerContent::Adjustment(Adjustment::from(adjustment));
            document.add_layer(name, content, at)?.1
        }
        EditDto::SetAdjustment {
            layer_id,
            adjustment,
        } => {
            let id = resolve(document, layer_id)?;
            document.set_adjustment(id, adjustment.into())?
        }
    };
    Ok(undo)
}

#[cfg(test)]
mod tests {
    // Test code: a failed setup is a test failure.
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use expose_editor::{BlendMode, EditorError, GroupIsolation, LayerKind};

    use super::*;
    use crate::editor::dto::{AdjustmentDto, BlendModeId, LayerKindId};

    fn session() -> EditSession {
        let image = WorkingImage::new_filled(4, 3, [0.5, 0.25, 0.125, 1.0]).unwrap();
        EditSession::new(build_document(image).unwrap()).unwrap()
    }

    fn background(session: &EditSession) -> u64 {
        session.document().layers()[0].id().get()
    }

    fn bc(brightness: f32) -> AdjustmentDto {
        AdjustmentDto::BrightnessContrast {
            brightness,
            contrast: 0.0,
        }
    }

    #[test]
    fn build_document_has_one_premultiplied_background_layer() {
        let mut image = WorkingImage::new_filled(3, 2, [0.8, 0.4, 0.2, 1.0]).unwrap();
        image.pixels_mut()[0] = [0.8, 0.4, 0.2, 0.5];
        let doc = build_document(image).unwrap();
        assert_eq!((doc.width(), doc.height()), (3, 2));
        assert_eq!(doc.layers().len(), 1);
        let layer = &doc.layers()[0];
        assert_eq!(layer.name(), BACKGROUND_NAME);
        let pixel = layer.as_pixel().unwrap();
        assert_eq!(pixel.offset(), (0, 0));
        let close = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
        assert!(close(pixel.image().pixels()[0], [0.4, 0.2, 0.1, 0.5]));
        assert!(close(pixel.image().pixels()[1], [0.8, 0.4, 0.2, 1.0]));
        let session = EditSession::new(doc).unwrap();
        let info = session.info(1);
        assert!(!info.can_undo && !info.can_redo);
    }

    #[test]
    fn property_edits_apply_and_undo() {
        let mut s = session();
        let id = background(&s);
        let edits = [
            EditDto::SetVisible {
                layer_id: id,
                visible: false,
            },
            EditDto::SetOpacity {
                layer_id: id,
                opacity: 0.5,
            },
            EditDto::SetFillOpacity {
                layer_id: id,
                fill_opacity: 0.25,
            },
            EditDto::SetBlendMode {
                layer_id: id,
                mode: BlendModeId::Multiply,
            },
            EditDto::Rename {
                layer_id: id,
                name: "Photo".into(),
            },
        ];
        for edit in edits {
            s.edit(edit, None).unwrap();
        }
        let info = s.info(1);
        let layer = &info.layers[0];
        assert!(!layer.visible);
        assert!((layer.opacity - 0.5).abs() < f32::EPSILON);
        assert!((layer.fill_opacity - 0.25).abs() < f32::EPSILON);
        assert_eq!(layer.blend_mode, BlendModeId::Multiply);
        assert_eq!(layer.name, "Photo");
        assert!(info.can_undo);
        for _ in 0..5 {
            s.undo().unwrap();
        }
        let layer = &s.document().layers()[0];
        assert!(layer.visible());
        assert_eq!(layer.blend_mode(), BlendMode::Normal);
        assert_eq!(layer.name(), BACKGROUND_NAME);
        let info = s.info(1);
        assert!(!info.can_undo && info.can_redo);
        s.redo().unwrap();
        assert!(!s.document().layers()[0].visible());
    }

    #[test]
    fn add_adjustment_goes_above_the_layer_or_on_top() {
        let mut s = session();
        let bg = background(&s);
        s.edit(
            EditDto::AddAdjustment {
                adjustment: bc(0.2),
                above_layer_id: None,
            },
            None,
        )
        .unwrap();
        s.edit(
            EditDto::AddAdjustment {
                adjustment: AdjustmentDto::HueSaturation {
                    hue: 10.0,
                    saturation: 0.0,
                    lightness: 0.0,
                },
                above_layer_id: Some(bg),
            },
            None,
        )
        .unwrap();
        let info = s.info(1);
        let names: Vec<&str> = info.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(
            names,
            [BACKGROUND_NAME, "Hue/Saturation", "Brightness/Contrast"]
        );
        assert_eq!(info.layers[2].kind, LayerKindId::Adjustment);
        assert_eq!(info.layers[2].adjustment, Some(bc(0.2)));
    }

    #[test]
    fn add_adjustment_above_a_layer_in_a_group_stays_in_the_group() {
        let image = WorkingImage::new_filled(2, 2, [0.5; 4]).unwrap();
        let mut doc = build_document(image).unwrap();
        let (group, _) = doc
            .add_layer(
                "Group",
                LayerContent::Group(GroupIsolation::PassThrough),
                Placement::root(1),
            )
            .unwrap();
        let bg = doc.layers()[0].id();
        doc.move_layer(bg, Placement::new(Some(group), 0)).unwrap();
        let mut s = EditSession::new(doc).unwrap();
        s.edit(
            EditDto::AddAdjustment {
                adjustment: bc(0.1),
                above_layer_id: Some(bg.get()),
            },
            None,
        )
        .unwrap();
        let doc = s.document();
        assert_eq!(doc.layers().len(), 1);
        let LayerKind::Group(g) = doc.layers()[0].kind() else {
            panic!("the root layer is not the group");
        };
        assert_eq!(g.children().len(), 2);
        assert_eq!(g.children()[0].id(), bg);
        assert!(matches!(g.children()[1].kind(), LayerKind::Adjustment(_)));
    }

    #[test]
    fn move_duplicate_delete_and_set_adjustment() {
        let mut s = session();
        let bg = background(&s);
        s.edit(EditDto::Duplicate { layer_id: bg }, None).unwrap();
        let info = s.info(1);
        assert_eq!(info.layers.len(), 2);
        assert_eq!(info.layers[1].name, "Background copy");
        let copy = info.layers[1].id;
        assert_ne!(copy, bg);
        // Move the copy to the bottom.
        s.edit(
            EditDto::Move {
                layer_id: copy,
                parent_id: None,
                index: 0,
            },
            None,
        )
        .unwrap();
        assert_eq!(s.info(1).layers[0].id, copy);
        s.edit(
            EditDto::AddAdjustment {
                adjustment: bc(0.1),
                above_layer_id: None,
            },
            None,
        )
        .unwrap();
        let adjust = s.info(1).layers[2].id;
        s.edit(
            EditDto::SetAdjustment {
                layer_id: adjust,
                adjustment: bc(-0.3),
            },
            None,
        )
        .unwrap();
        assert_eq!(s.info(1).layers[2].adjustment, Some(bc(-0.3)));
        // A pixel layer cannot take an adjustment setting.
        let error = s
            .edit(
                EditDto::SetAdjustment {
                    layer_id: bg,
                    adjustment: bc(0.0),
                },
                None,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            EditorEngineError::Editor(EditorError::NotAnAdjustmentLayer(_))
        ));
        s.edit(EditDto::Delete { layer_id: copy }, None).unwrap();
        assert_eq!(s.info(1).layers.len(), 2);
        s.undo().unwrap();
        assert_eq!(s.info(1).layers[0].id, copy);
    }

    #[test]
    fn bad_edits_fail_with_no_change() {
        let mut s = session();
        let bg = background(&s);
        let revision = s.document().revision();
        let error = s
            .edit(
                EditDto::SetOpacity {
                    layer_id: bg,
                    opacity: 1.5,
                },
                None,
            )
            .unwrap_err();
        assert!(matches!(error, EditorEngineError::Editor(_)));
        let error = s
            .edit(
                EditDto::SetVisible {
                    layer_id: 999,
                    visible: false,
                },
                None,
            )
            .unwrap_err();
        assert!(matches!(error, EditorEngineError::LayerNotFound(999)));
        let error = s
            .edit(
                EditDto::Move {
                    layer_id: bg,
                    parent_id: Some(bg),
                    index: 0,
                },
                None,
            )
            .unwrap_err();
        assert!(matches!(error, EditorEngineError::Editor(_)));
        let error = s
            .edit(
                EditDto::AddAdjustment {
                    adjustment: bc(5.0),
                    above_layer_id: None,
                },
                None,
            )
            .unwrap_err();
        assert!(matches!(error, EditorEngineError::Editor(_)));
        assert_eq!(s.document().revision(), revision);
        assert!(!s.info(1).can_undo);
    }

    #[test]
    fn edits_of_one_gesture_are_one_undo_step() {
        let mut s = session();
        let bg = background(&s);
        for opacity in [0.9, 0.7, 0.5] {
            s.edit(
                EditDto::SetOpacity {
                    layer_id: bg,
                    opacity,
                },
                Some(7),
            )
            .unwrap();
        }
        // A new gesture is a new step.
        s.edit(
            EditDto::SetOpacity {
                layer_id: bg,
                opacity: 0.3,
            },
            Some(8),
        )
        .unwrap();
        let opacity = |s: &EditSession| s.document().layers()[0].opacity();
        assert!((opacity(&s) - 0.3).abs() < f32::EPSILON);
        s.undo().unwrap();
        assert!((opacity(&s) - 0.5).abs() < f32::EPSILON);
        s.undo().unwrap();
        assert!((opacity(&s) - 1.0).abs() < f32::EPSILON);
        assert!(!s.info(1).can_undo);
        // An undo ends the merge: the same key starts a new step.
        s.redo().unwrap();
        s.edit(
            EditDto::SetOpacity {
                layer_id: bg,
                opacity: 0.1,
            },
            Some(7),
        )
        .unwrap();
        s.undo().unwrap();
        assert!((opacity(&s) - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn revision_changes_with_edit_undo_and_redo() {
        let mut s = session();
        let bg = background(&s);
        let r0 = s.info(1).revision;
        s.edit(
            EditDto::SetVisible {
                layer_id: bg,
                visible: false,
            },
            None,
        )
        .unwrap();
        let r1 = s.info(1).revision;
        s.undo().unwrap();
        let r2 = s.info(1).revision;
        s.redo().unwrap();
        let r3 = s.info(1).revision;
        assert!(r0 != r1 && r1 != r2 && r2 != r3);
        // Undo with nothing to undo is not an error.
        let mut fresh = session();
        fresh.undo().unwrap();
        fresh.redo().unwrap();
    }
}
