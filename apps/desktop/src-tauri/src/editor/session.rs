//! One open document and its history, with no GPU and no thread. The
//! engine thread owns one [`EditSession`] for each open document; tests
//! use it directly.

use expose_core::buffer::WorkingImage;
use expose_editor::{
    Adjustment, BrushSettings, Document, DocumentSpec, Edit, EditorError, History, LayerContent,
    LayerId, Mask, MaskFill, MaskImage, PixelLayer, Placement, Stroke, StrokePoint, StrokeTarget,
};

use super::EditorEngineError;
use super::dto::{DocumentId, DocumentInfo, EditDto, HistoryInfo, MaskFillId};
use super::save::{DocumentOrigin, SavedFiles};

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
    // The photo layer is the only layer, at the canvas origin.
    let layer = PixelLayer::new(image, (0, 0))?;
    Ok(Document::from_layer(spec, BACKGROUND_NAME, layer)?)
}

/// An open document, its undo history, and its save state.
#[derive(Debug)]
pub struct EditSession {
    document: Document,
    history: History,
    origin: DocumentOrigin,
    /// The document revision at the last save, or at the open.
    saved_revision: u64,
    /// The stroke that the user draws now, if any.
    stroke: Option<ActiveStroke>,
    /// The ID of the next stroke. IDs start at 1 and stay far below 2^53.
    next_stroke_id: u64,
}

/// A stroke in progress, with its ID and the name of its history step.
#[derive(Debug)]
struct ActiveStroke {
    id: u64,
    name: &'static str,
    stroke: Stroke,
}

/// The name of the history step of a stroke with `target`.
const fn stroke_step_name(target: &StrokeTarget) -> &'static str {
    match target {
        StrokeTarget::Paint { .. } => "Brush",
        StrokeTarget::Erase => "Eraser",
        StrokeTarget::Mask { .. } => "Mask Brush",
    }
}

impl EditSession {
    /// Starts a session for `document` with an empty history of
    /// [`HISTORY_STEPS`] steps and no origin. The document is not dirty.
    ///
    /// # Errors
    ///
    /// [`EditorEngineError::Internal`] if the history cannot be made (the
    /// step count is a constant larger than 0, so this does not occur).
    pub fn new(document: Document) -> Result<Self, EditorEngineError> {
        Self::with_origin(document, DocumentOrigin::default())
    }

    /// Starts a session like [`Self::new`], with the photo and the files
    /// that the document came from.
    ///
    /// # Errors
    ///
    /// See [`Self::new`].
    pub fn with_origin(
        document: Document,
        origin: DocumentOrigin,
    ) -> Result<Self, EditorEngineError> {
        let history = History::new(HISTORY_STEPS)
            .map_err(|_| EditorEngineError::Internal("the history step count is 0"))?;
        let saved_revision = document.revision();
        Ok(Self {
            document,
            history,
            origin,
            saved_revision,
            stroke: None,
            next_stroke_id: 1,
        })
    }

    /// `true` if a stroke is active.
    pub const fn has_stroke(&self) -> bool {
        self.stroke.is_some()
    }

    /// Starts a stroke on the layer with the number `layer`. An active
    /// stroke ends first (it is committed as one history step). Returns the
    /// ID of the new stroke.
    ///
    /// # Errors
    ///
    /// - [`EditorEngineError::LayerNotFound`] if `layer` is not in the
    ///   document.
    /// - [`EditorEngineError::Editor`] for bad settings, a layer of the
    ///   wrong kind, a missing mask, or a lock.
    /// - The errors of [`Self::commit_stroke`].
    pub fn stroke_begin(
        &mut self,
        layer: u64,
        target: StrokeTarget,
        settings: BrushSettings,
    ) -> Result<u64, EditorEngineError> {
        self.commit_stroke()?;
        let layer = resolve(&self.document, layer)?;
        let name = stroke_step_name(&target);
        let stroke = Stroke::begin(&self.document, layer, target, settings)?;
        let id = self.next_stroke_id;
        self.next_stroke_id += 1;
        self.stroke = Some(ActiveStroke { id, name, stroke });
        Ok(id)
    }

    /// Adds points to the active stroke. The layer changes in place.
    ///
    /// # Errors
    ///
    /// - [`EditorEngineError::StrokeNotActive`] if `id` is not the ID of the
    ///   active stroke.
    /// - [`EditorEngineError::Editor`] if the stroke rejects the points. The
    ///   stroke stays active.
    pub fn stroke_points(
        &mut self,
        id: u64,
        points: &[StrokePoint],
    ) -> Result<(), EditorEngineError> {
        match self.stroke.as_mut() {
            Some(active) if active.id == id => {
                active.stroke.add_points(&mut self.document, points)?;
                Ok(())
            }
            _ => Err(EditorEngineError::StrokeNotActive(id)),
        }
    }

    /// Ends the stroke `id` and records one history step.
    ///
    /// # Errors
    ///
    /// [`EditorEngineError::StrokeNotActive`] and the errors of
    /// [`Self::commit_stroke`].
    pub fn stroke_end(&mut self, id: u64) -> Result<(), EditorEngineError> {
        self.active_stroke(id)?;
        self.commit_stroke()
    }

    /// Cancels the stroke `id`. The layer returns to its state before the
    /// stroke. No history step is recorded.
    ///
    /// # Errors
    ///
    /// [`EditorEngineError::StrokeNotActive`] and
    /// [`EditorEngineError::Editor`] if the restore fails.
    pub fn stroke_cancel(&mut self, id: u64) -> Result<(), EditorEngineError> {
        self.active_stroke(id)?;
        if let Some(active) = self.stroke.take() {
            active.stroke.cancel(&mut self.document)?;
        }
        Ok(())
    }

    /// Ends the active stroke, if any, and records it as one history step.
    /// Every operation that is not a stroke call runs this first.
    ///
    /// # Errors
    ///
    /// [`EditorEngineError::Editor`] if the stroke cannot end. The stroke is
    /// then gone.
    pub fn commit_stroke(&mut self) -> Result<(), EditorEngineError> {
        if let Some(active) = self.stroke.take() {
            if let Some(edit) = active.stroke.end(&mut self.document)? {
                self.history.push(active.name, vec![edit]);
            }
        }
        Ok(())
    }

    fn active_stroke(&self, id: u64) -> Result<&ActiveStroke, EditorEngineError> {
        match self.stroke.as_ref() {
            Some(active) if active.id == id => Ok(active),
            _ => Err(EditorEngineError::StrokeNotActive(id)),
        }
    }

    /// The document.
    pub const fn document(&self) -> &Document {
        &self.document
    }

    /// The photo and the files that the document came from.
    pub const fn origin(&self) -> &DocumentOrigin {
        &self.origin
    }

    /// `true` if the document changed after the last save (or the open).
    /// An undo back to the saved state is also a change: the revision is a
    /// change stamp, not a content hash.
    pub const fn is_dirty(&self) -> bool {
        self.document.revision() != self.saved_revision
    }

    /// Records a save of the document at `revision` to `files`. Edits that
    /// came after the snapshot of the save keep the document dirty.
    pub fn mark_saved(&mut self, revision: u64, files: SavedFiles) {
        self.saved_revision = revision;
        self.origin.saved = Some(files);
    }

    /// The info for the UI, with the document ID `id`.
    pub fn info(&self, id: DocumentId) -> DocumentInfo {
        let mut info = DocumentInfo::new(
            id,
            &self.document,
            self.history.can_undo(),
            self.history.can_redo(),
        );
        info.dirty = self.is_dirty();
        info.saved_photo_id = self.origin.saved.as_ref().map(|s| s.photo_id);
        info.source_photo_id = self.origin.source_photo_id;
        info.name = self.name();
        info.history = HistoryInfo {
            steps: self
                .history
                .entries()
                .map(|e| e.name().to_owned())
                .collect(),
            // The history holds at most `HISTORY_STEPS` entries, so the
            // cursor fits in `u32`.
            cursor: u32::try_from(self.history.cursor()).unwrap_or(u32::MAX),
        };
        info
    }

    /// The title of the document: the file stem of the layered file of the
    /// last save or link, else the name of the origin, else "Untitled".
    pub fn name(&self) -> String {
        self.origin
            .saved
            .as_ref()
            .and_then(|s| s.document_path.file_stem())
            .map(|stem| stem.to_string_lossy().into_owned())
            .or_else(|| self.origin.name.clone())
            .unwrap_or_else(|| "Untitled".to_owned())
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
        self.commit_stroke()?;
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
        self.commit_stroke()?;
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
        self.commit_stroke()?;
        self.history
            .redo(&mut self.document)
            .map(drop)
            .map_err(EditorEngineError::History)
    }

    /// Undoes or redoes steps until `step` steps are applied. An active
    /// stroke ends first (it can add a step). A jump to the current cursor
    /// does nothing.
    ///
    /// # Errors
    ///
    /// - [`EditorEngineError::Editor`] with `IndexOutOfRange` if `step` is
    ///   more than the number of steps.
    /// - [`EditorEngineError::History`] if a step fails.
    pub fn history_jump(&mut self, step: usize) -> Result<(), EditorEngineError> {
        self.commit_stroke()?;
        let len = self.history.entries().len();
        if step > len {
            return Err(EditorEngineError::Editor(EditorError::IndexOutOfRange {
                index: step,
                len,
            }));
        }
        self.history
            .jump_to(&mut self.document, step)
            .map_err(EditorEngineError::History)
    }
}

/// Finds the layer with the numeric ID `raw`.
fn resolve(document: &Document, raw: u64) -> Result<LayerId, EditorEngineError> {
    document
        .layer_by_number(raw)
        .ok_or(EditorEngineError::LayerNotFound(raw))
}

/// `true` if the layer with the number `raw` has a mask.
fn has_mask(document: &Document, raw: u64) -> Result<bool, EditorEngineError> {
    let id = resolve(document, raw)?;
    document
        .layer(id)
        .map(|layer| layer.mask().is_some())
        .ok_or(EditorEngineError::LayerNotFound(raw))
}

/// The placement directly above the layer `above_layer_id` in its parent,
/// or at the top of the root list for `None`.
fn placement_above(
    document: &Document,
    above_layer_id: Option<u64>,
) -> Result<Placement, EditorEngineError> {
    Ok(match above_layer_id {
        Some(raw) => {
            let above = resolve(document, raw)?;
            let placement = document
                .placement_of(above)
                .ok_or(EditorEngineError::LayerNotFound(raw))?;
            Placement::new(placement.parent, placement.index + 1)
        }
        None => Placement::root(document.layers().len()),
    })
}

/// The name "Layer N" of a new pixel layer: N is 1 more than the largest
/// N of the layers (in the whole tree) that have exactly the name
/// "Layer N", or 1 if there is none.
fn next_layer_name(document: &Document) -> String {
    let mut largest = 0_u64;
    for layer in document.layers() {
        layer.visit(&mut |l| {
            if let Some(n) = l
                .name()
                .strip_prefix("Layer ")
                .filter(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|rest| rest.parse::<u64>().ok())
            {
                largest = largest.max(n);
            }
        });
    }
    format!("Layer {}", largest.saturating_add(1))
}

/// Applies one edit and returns its inverse.
#[allow(clippy::too_many_lines, reason = "one arm per edit")]
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
            let at = placement_above(document, above_layer_id)?;
            let name = adjustment.layer_name();
            let content = LayerContent::Adjustment(Adjustment::from(adjustment));
            document.add_layer(name, content, at)?.1
        }
        EditDto::AddPixelLayer { above_layer_id } => {
            let at = placement_above(document, above_layer_id)?;
            let image = WorkingImage::new(document.width(), document.height())
                .map_err(expose_editor::EditorError::from)?;
            let layer = PixelLayer::new(image, (0, 0))?;
            let name = next_layer_name(document);
            document.add_layer(name, LayerContent::Pixel(layer), at)?.1
        }
        EditDto::SetLocks { layer_id, locks } => {
            let id = resolve(document, layer_id)?;
            document.set_locks(id, locks.into())?
        }
        EditDto::AddMask { layer_id, fill } => {
            let id = resolve(document, layer_id)?;
            if has_mask(document, layer_id)? {
                return Err(EditorEngineError::MaskExists(layer_id));
            }
            let (value, mask_fill) = match fill {
                MaskFillId::Reveal => (1.0, MaskFill::White),
                MaskFillId::Hide => (0.0, MaskFill::Black),
            };
            let image = MaskImage::new_filled(document.width(), document.height(), value)?;
            document.set_mask(id, Some(Mask::new(image, (0, 0), mask_fill)))?
        }
        EditDto::DeleteMask { layer_id } => {
            let id = resolve(document, layer_id)?;
            if !has_mask(document, layer_id)? {
                return Err(expose_editor::EditorError::NoMask(id).into());
            }
            document.set_mask(id, None)?
        }
        EditDto::SetMaskEnabled { layer_id, enabled } => {
            let id = resolve(document, layer_id)?;
            document.set_mask_enabled(id, enabled)?
        }
        EditDto::SetAdjustment {
            layer_id,
            adjustment,
        } => {
            let id = resolve(document, layer_id)?;
            document.set_adjustment(id, adjustment.into())?
        }
        edit @ (EditDto::SelectShape { .. }
        | EditDto::SelectAll
        | EditDto::Deselect
        | EditDto::Reselect
        | EditDto::InvertSelection
        | EditDto::FeatherSelection { .. }
        | EditDto::MaskFromSelection { .. }) => apply_selection(document, edit)?,
    };
    Ok(undo)
}

/// Applies a selection edit. Other edits are a bug and give an error.
fn apply_selection(document: &mut Document, edit: EditDto) -> Result<Edit, EditorEngineError> {
    Ok(match edit {
        EditDto::SelectShape {
            shape,
            mode,
            anti_alias,
            feather,
        } => document.select_shape(&shape.into(), mode.into(), anti_alias, feather)?,
        EditDto::SelectAll => document.select_all()?,
        EditDto::Deselect => document.deselect()?,
        EditDto::Reselect => document.reselect()?,
        EditDto::InvertSelection => document.invert_selection()?,
        EditDto::FeatherSelection { radius } => document.feather_selection(radius)?,
        EditDto::MaskFromSelection { layer_id } => {
            let id = resolve(document, layer_id)?;
            document.mask_from_selection(id)?
        }
        _ => return Err(EditorEngineError::Internal("not a selection edit")),
    })
}

#[cfg(test)]
mod tests {
    // Test code: a failed setup is a test failure.
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use expose_editor::{BlendMode, EditorError, GroupIsolation, LayerKind};

    use super::*;
    use crate::editor::dto::{AdjustmentDto, BlendModeId, LayerKindId, LocksDto};

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
    fn dirty_follows_the_saved_revision() {
        let mut s = session();
        let bg = background(&s);
        let info = s.info(1);
        assert!(!info.dirty);
        assert_eq!(info.saved_photo_id, None);
        s.edit(
            EditDto::SetVisible {
                layer_id: bg,
                visible: false,
            },
            None,
        )
        .unwrap();
        assert!(s.info(1).dirty);
        let files = SavedFiles {
            photo_id: expose_catalog::PhotoId(7),
            document_path: "/p/a-edit.exdoc".into(),
            image_path: "/p/a-edit.tif".into(),
        };
        // A save of the current revision.
        s.mark_saved(s.document().revision(), files.clone());
        let info = s.info(1);
        assert!(!info.dirty);
        assert_eq!(info.saved_photo_id, Some(expose_catalog::PhotoId(7)));
        assert_eq!(s.origin().saved.as_ref(), Some(&files));
        // An undo after the save is a change.
        s.undo().unwrap();
        assert!(s.info(1).dirty);
        // A save of an older revision (an edit came after the snapshot)
        // keeps the document dirty.
        let old = s.document().revision();
        s.redo().unwrap();
        s.mark_saved(old, files);
        assert!(s.info(1).dirty);
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

    fn names(s: &EditSession) -> Vec<String> {
        s.document()
            .layers()
            .iter()
            .map(|l| l.name().to_owned())
            .collect()
    }

    #[test]
    fn add_pixel_layer_places_and_names_it() {
        let mut s = session();
        let bg = background(&s);
        s.edit(
            EditDto::AddPixelLayer {
                above_layer_id: None,
            },
            None,
        )
        .unwrap();
        s.edit(
            EditDto::AddPixelLayer {
                above_layer_id: Some(bg),
            },
            None,
        )
        .unwrap();
        // Layer 2 sits directly above the background, Layer 1 on top.
        assert_eq!(names(&s), ["Background", "Layer 2", "Layer 1"]);
        let layer = &s.document().layers()[1];
        let LayerKind::Pixel(p) = layer.kind() else {
            panic!("not a pixel layer");
        };
        assert_eq!((p.image().width(), p.image().height()), (4, 3));
        assert!(
            p.image()
                .pixels()
                .iter()
                .all(|px| px.iter().all(|c| *c < 1e-9))
        );
        s.undo().unwrap();
        s.undo().unwrap();
        assert_eq!(names(&s), ["Background"]);
    }

    #[test]
    fn pixel_layer_name_counts_up_from_the_largest() {
        let mut s = session();
        let bg = background(&s);
        s.edit(
            EditDto::Rename {
                layer_id: bg,
                name: "Layer 7".into(),
            },
            None,
        )
        .unwrap();
        s.edit(
            EditDto::AddPixelLayer {
                above_layer_id: None,
            },
            None,
        )
        .unwrap();
        assert_eq!(names(&s), ["Layer 7", "Layer 8"]);
    }

    #[test]
    fn locks_block_property_edits_and_are_undoable() {
        let mut s = session();
        let bg = background(&s);
        let locks = LocksDto {
            all: true,
            ..LocksDto::default()
        };
        s.edit(
            EditDto::SetLocks {
                layer_id: bg,
                locks,
            },
            None,
        )
        .unwrap();
        assert!(s.info(1).layers[0].locks.all);
        let before = s.info(1).revision;
        let error = s
            .edit(
                EditDto::SetOpacity {
                    layer_id: bg,
                    opacity: 0.5,
                },
                None,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            EditorEngineError::Editor(EditorError::Locked { .. })
        ));
        assert_eq!(s.info(1).revision, before);
        s.undo().unwrap();
        assert!(!s.info(1).layers[0].locks.all);
        assert!(!s.info(1).can_undo);
    }

    fn mask_values(s: &EditSession) -> (f32, MaskFill) {
        let mask = s.document().layers()[0].mask().unwrap();
        (mask.image.data()[0], mask.fill)
    }

    #[test]
    fn mask_add_reveal_and_hide() {
        for (fill, value, mask_fill) in [
            (MaskFillId::Reveal, 1.0, MaskFill::White),
            (MaskFillId::Hide, 0.0, MaskFill::Black),
        ] {
            let mut s = session();
            let bg = background(&s);
            s.edit(EditDto::AddMask { layer_id: bg, fill }, None)
                .unwrap();
            let info = &s.info(1).layers[0];
            assert!(info.has_mask && info.mask_enabled);
            assert_eq!(mask_values(&s), (value, mask_fill));
            let mask = s.document().layers()[0].mask().unwrap();
            assert_eq!(mask.offset, (0, 0));
            assert_eq!((mask.image.width(), mask.image.height()), (4, 3));
            s.undo().unwrap();
            assert!(!s.info(1).layers[0].has_mask);
            s.redo().unwrap();
            assert!(s.info(1).layers[0].has_mask);
        }
    }

    #[test]
    fn mask_double_add_is_an_error() {
        let mut s = session();
        let bg = background(&s);
        let add = EditDto::AddMask {
            layer_id: bg,
            fill: MaskFillId::Reveal,
        };
        s.edit(add.clone(), None).unwrap();
        let error = s.edit(add, None).unwrap_err();
        assert!(matches!(error, EditorEngineError::MaskExists(id) if id == bg));
    }

    fn select_rect() -> EditDto {
        EditDto::SelectShape {
            shape: super::super::dto::SelectionShapeDto::Rect {
                x: 1.0,
                y: 1.0,
                width: 2.0,
                height: 2.0,
            },
            mode: super::super::dto::SelectionModeId::Replace,
            anti_alias: false,
            feather: 0.0,
        }
    }

    #[test]
    fn select_and_undo_restore_the_selection_state() {
        let mut s = session();
        let before = s.info(1);
        assert!(!before.has_selection);
        s.edit(select_rect(), None).unwrap();
        let selected = s.info(1);
        assert!(selected.has_selection);
        assert_ne!(selected.selection_version, before.selection_version);
        s.undo().unwrap();
        let undone = s.info(1);
        assert!(!undone.has_selection);
        assert_ne!(undone.selection_version, selected.selection_version);
        s.redo().unwrap();
        assert!(s.info(1).has_selection);
    }

    #[test]
    fn selection_edits_without_selection_fail_without_a_step() {
        let mut s = session();
        for edit in [
            EditDto::Deselect,
            EditDto::InvertSelection,
            EditDto::FeatherSelection { radius: 2.0 },
        ] {
            let error = s.edit(edit, None).unwrap_err();
            assert!(matches!(
                error,
                EditorEngineError::Editor(EditorError::NoSelection)
            ));
        }
        assert!(!s.info(1).can_undo);
    }

    #[test]
    fn reselect_restores_the_selection() {
        let mut s = session();
        assert!(!s.info(1).can_reselect);
        assert!(s.edit(EditDto::Reselect, None).is_err());
        s.edit(EditDto::SelectAll, None).unwrap();
        s.edit(EditDto::Deselect, None).unwrap();
        let info = s.info(1);
        assert!(!info.has_selection);
        assert!(info.can_reselect);
        s.edit(EditDto::Reselect, None).unwrap();
        assert!(s.info(1).has_selection);
    }

    #[test]
    fn selection_edits_apply() {
        let mut s = session();
        let bg = background(&s);
        s.edit(EditDto::SelectAll, None).unwrap();
        assert!(s.info(1).has_selection);
        s.edit(EditDto::InvertSelection, None).unwrap();
        s.edit(EditDto::FeatherSelection { radius: 1.0 }, None)
            .unwrap();
        s.edit(select_rect(), None).unwrap();
        s.edit(EditDto::MaskFromSelection { layer_id: bg }, None)
            .unwrap();
        assert!(s.document().layers()[0].mask().is_some());
        s.edit(EditDto::Deselect, None).unwrap();
        assert!(!s.info(1).has_selection);
    }

    #[test]
    fn mask_delete_and_enable_with_undo() {
        let mut s = session();
        let bg = background(&s);
        // Without a mask, both edits fail.
        for edit in [
            EditDto::DeleteMask { layer_id: bg },
            EditDto::SetMaskEnabled {
                layer_id: bg,
                enabled: false,
            },
        ] {
            let error = s.edit(edit, None).unwrap_err();
            assert!(matches!(
                error,
                EditorEngineError::Editor(EditorError::NoMask(_))
            ));
        }
        s.edit(
            EditDto::AddMask {
                layer_id: bg,
                fill: MaskFillId::Hide,
            },
            None,
        )
        .unwrap();
        s.edit(
            EditDto::SetMaskEnabled {
                layer_id: bg,
                enabled: false,
            },
            None,
        )
        .unwrap();
        let info = &s.info(1).layers[0];
        assert!(info.has_mask && !info.mask_enabled);
        s.undo().unwrap();
        assert!(s.info(1).layers[0].mask_enabled);
        s.edit(EditDto::DeleteMask { layer_id: bg }, None).unwrap();
        let info = &s.info(1).layers[0];
        assert!(!info.has_mask && !info.mask_enabled);
        s.undo().unwrap();
        assert!(s.info(1).layers[0].has_mask);
        assert_eq!(mask_values(&s), (0.0, MaskFill::Black));
    }

    #[test]
    fn mask_version_follows_the_mask() {
        let mut s = session();
        let bg = s.info(1).layers[0].id;
        assert_eq!(s.info(1).layers[0].mask_version, None);
        let add = |fill| EditDto::AddMask { layer_id: bg, fill };
        s.edit(add(MaskFillId::Hide), None).unwrap();
        let first = s.info(1).layers[0].mask_version.unwrap();
        assert_eq!(s.info(1).layers[0].mask_version, Some(first));
        s.edit(EditDto::DeleteMask { layer_id: bg }, None).unwrap();
        assert_eq!(s.info(1).layers[0].mask_version, None);
        s.edit(add(MaskFillId::Reveal), None).unwrap();
        let second = s.info(1).layers[0].mask_version.unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn mask_edits_obey_locks() {
        let mut s = session();
        let bg = background(&s);
        s.edit(
            EditDto::AddMask {
                layer_id: bg,
                fill: MaskFillId::Reveal,
            },
            None,
        )
        .unwrap();
        // The pixels lock stops add and delete, not the enable switch.
        let pixels = LocksDto {
            pixels: true,
            ..LocksDto::default()
        };
        s.edit(
            EditDto::SetLocks {
                layer_id: bg,
                locks: pixels,
            },
            None,
        )
        .unwrap();
        let error = s
            .edit(EditDto::DeleteMask { layer_id: bg }, None)
            .unwrap_err();
        assert!(matches!(
            error,
            EditorEngineError::Editor(EditorError::Locked { .. })
        ));
        s.edit(
            EditDto::SetMaskEnabled {
                layer_id: bg,
                enabled: false,
            },
            None,
        )
        .unwrap();
        // The all lock stops the enable switch.
        let all = LocksDto {
            all: true,
            ..LocksDto::default()
        };
        s.edit(
            EditDto::SetLocks {
                layer_id: bg,
                locks: all,
            },
            None,
        )
        .unwrap();
        let error = s
            .edit(
                EditDto::SetMaskEnabled {
                    layer_id: bg,
                    enabled: true,
                },
                None,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            EditorEngineError::Editor(EditorError::Locked { .. })
        ));
    }

    #[test]
    fn info_has_the_source_photo() {
        let image = WorkingImage::new_filled(2, 2, [0.0; 4]).unwrap();
        let origin = DocumentOrigin {
            source_photo_id: Some(expose_catalog::PhotoId(5)),
            ..DocumentOrigin::default()
        };
        let s = EditSession::with_origin(build_document(image).unwrap(), origin).unwrap();
        assert_eq!(s.info(1).source_photo_id, Some(expose_catalog::PhotoId(5)));
    }

    #[test]
    fn info_name_follows_origin_and_saves() {
        // Default origin: no name known.
        assert_eq!(session().info(1).name, "Untitled");
        // Opened photo or new document: the name of the origin.
        let image = WorkingImage::new_filled(2, 2, [0.0; 4]).unwrap();
        let origin = DocumentOrigin {
            name: Some("IMG_0001".to_owned()),
            ..DocumentOrigin::default()
        };
        let mut s = EditSession::with_origin(build_document(image).unwrap(), origin).unwrap();
        assert_eq!(s.info(1).name, "IMG_0001");
        // A save sets the name to the stem of the layered file.
        let files = |stem: &str| SavedFiles {
            photo_id: expose_catalog::PhotoId(7),
            document_path: format!("/p/{stem}.exdoc").into(),
            image_path: format!("/p/{stem}.tif").into(),
        };
        s.mark_saved(s.document().revision(), files("IMG_0001-edit"));
        assert_eq!(s.info(1).name, "IMG_0001-edit");
        // Edits and undo keep the name; a save as changes it.
        let bg = background(&s);
        s.edit(
            EditDto::SetVisible {
                layer_id: bg,
                visible: false,
            },
            None,
        )
        .unwrap();
        s.undo().unwrap();
        assert_eq!(s.info(1).name, "IMG_0001-edit");
        s.mark_saved(s.document().revision(), files("other-edit-2"));
        assert_eq!(s.info(1).name, "other-edit-2");
    }

    fn brush() -> BrushSettings {
        BrushSettings {
            size: 3.0,
            hardness: 1.0,
            opacity: 1.0,
            flow: 1.0,
            spacing: 0.25,
            pressure_size: false,
            pressure_opacity: false,
            ..BrushSettings::default()
        }
    }

    fn white() -> StrokeTarget {
        StrokeTarget::Paint {
            color: [1.0, 1.0, 1.0],
        }
    }

    fn pixels(s: &EditSession) -> Vec<[f32; 4]> {
        s.document().layers()[0]
            .as_pixel()
            .unwrap()
            .image()
            .pixels()
            .to_vec()
    }

    fn dabs() -> Vec<StrokePoint> {
        vec![
            StrokePoint::new(1.5, 1.5, 1.0),
            StrokePoint::new(2.5, 1.5, 1.0),
        ]
    }

    #[test]
    fn stroke_is_one_undo_step() {
        let mut s = session();
        let bg = background(&s);
        let before = pixels(&s);
        let id = s.stroke_begin(bg, white(), brush()).unwrap();
        s.stroke_points(id, &dabs()).unwrap();
        s.stroke_points(id, &dabs()).unwrap();
        assert_ne!(pixels(&s), before);
        s.stroke_end(id).unwrap();
        let info = s.info(1);
        assert!(info.can_undo && info.dirty);
        s.undo().unwrap();
        assert_eq!(pixels(&s), before);
        // The stroke was the only step.
        assert!(!s.info(1).can_undo);
        s.redo().unwrap();
        assert_ne!(pixels(&s), before);
    }

    fn two_edits(s: &mut EditSession) {
        let id = background(s);
        s.edit(
            EditDto::SetVisible {
                layer_id: id,
                visible: false,
            },
            None,
        )
        .unwrap();
        s.edit(
            EditDto::SetOpacity {
                layer_id: id,
                opacity: 0.5,
            },
            None,
        )
        .unwrap();
    }

    #[test]
    fn info_has_the_history() {
        let mut s = session();
        let h = s.info(1).history;
        assert!(h.steps.is_empty() && h.cursor == 0);
        two_edits(&mut s);
        let h = s.info(1).history;
        assert_eq!((h.steps.len(), h.cursor), (2, 2));
        s.undo().unwrap();
        let h = s.info(1).history;
        assert_eq!((h.steps.len(), h.cursor), (2, 1));
    }

    #[test]
    fn jump_matches_undo_and_redo() {
        let mut jumped = session();
        let mut stepped = session();
        two_edits(&mut jumped);
        two_edits(&mut stepped);
        jumped.history_jump(0).unwrap();
        stepped.undo().unwrap();
        stepped.undo().unwrap();
        assert!(jumped.document().layers()[0].visible());
        assert_eq!(jumped.info(1).history.cursor, 0);
        assert_eq!(jumped.is_dirty(), stepped.is_dirty());
        jumped.history_jump(2).unwrap();
        stepped.redo().unwrap();
        stepped.redo().unwrap();
        let (a, b) = (
            &jumped.document().layers()[0],
            &stepped.document().layers()[0],
        );
        assert_eq!(a.visible(), b.visible());
        assert!((a.opacity() - b.opacity()).abs() < f32::EPSILON);
        assert_eq!(jumped.info(1).history.cursor, 2);
        // A jump to the current cursor does nothing.
        let revision = jumped.document().revision();
        jumped.history_jump(2).unwrap();
        assert_eq!(jumped.document().revision(), revision);
    }

    #[test]
    fn jump_past_the_end_is_refused() {
        let mut s = session();
        two_edits(&mut s);
        assert!(matches!(
            s.history_jump(3),
            Err(EditorEngineError::Editor(
                EditorError::IndexOutOfRange { .. }
            ))
        ));
        assert_eq!(s.info(1).history.cursor, 2);
    }

    #[test]
    fn jump_commits_an_active_stroke_first() {
        let mut s = session();
        let bg = background(&s);
        let id = s.stroke_begin(bg, white(), brush()).unwrap();
        s.stroke_points(id, &dabs()).unwrap();
        s.history_jump(0).unwrap();
        assert!(!s.has_stroke());
        let h = s.info(1).history;
        assert_eq!((h.steps.len(), h.cursor), (1, 0));
        s.history_jump(1).unwrap();
        assert_eq!(s.info(1).history.cursor, 1);
    }

    #[test]
    fn stroke_cancel_restores_and_adds_no_step() {
        let mut s = session();
        let bg = background(&s);
        let before = pixels(&s);
        let id = s.stroke_begin(bg, white(), brush()).unwrap();
        s.stroke_points(id, &dabs()).unwrap();
        s.stroke_cancel(id).unwrap();
        assert_eq!(pixels(&s), before);
        assert!(!s.info(1).can_undo);
        assert!(!s.has_stroke());
    }

    #[test]
    fn old_stroke_id_is_rejected() {
        let mut s = session();
        let bg = background(&s);
        let id = s.stroke_begin(bg, white(), brush()).unwrap();
        s.stroke_points(id, &dabs()).unwrap();
        s.stroke_end(id).unwrap();
        assert!(matches!(
            s.stroke_points(id, &dabs()),
            Err(EditorEngineError::StrokeNotActive(i)) if i == id
        ));
        assert!(matches!(
            s.stroke_end(id),
            Err(EditorEngineError::StrokeNotActive(_))
        ));
        assert!(matches!(
            s.stroke_cancel(id),
            Err(EditorEngineError::StrokeNotActive(_))
        ));
        assert!(matches!(
            s.stroke_points(id + 99, &dabs()),
            Err(EditorEngineError::StrokeNotActive(_))
        ));
    }

    #[test]
    fn stroke_on_an_adjustment_layer_fails() {
        let mut s = session();
        s.edit(
            EditDto::AddAdjustment {
                adjustment: bc(0.2),
                above_layer_id: None,
            },
            None,
        )
        .unwrap();
        let adj = s
            .document()
            .layers()
            .iter()
            .find(|l| l.as_pixel().is_none())
            .unwrap()
            .id()
            .get();
        assert!(matches!(
            s.stroke_begin(adj, white(), brush()),
            Err(EditorEngineError::Editor(EditorError::NotAPixelLayer(_)))
        ));
        assert!(!s.has_stroke());
    }

    #[test]
    fn mask_stroke_needs_a_mask() {
        let mut s = session();
        let bg = background(&s);
        let target = StrokeTarget::Mask { value: 0.0 };
        assert!(matches!(
            s.stroke_begin(bg, target, brush()),
            Err(EditorEngineError::Editor(EditorError::NoMask(_)))
        ));
        s.edit(
            EditDto::AddMask {
                layer_id: bg,
                fill: MaskFillId::Reveal,
            },
            None,
        )
        .unwrap();
        let id = s.stroke_begin(bg, target, brush()).unwrap();
        s.stroke_points(id, &dabs()).unwrap();
        s.stroke_end(id).unwrap();
        s.undo().unwrap();
        // The mask is still there; only the stroke is undone.
        assert!(s.info(1).layers[0].has_mask);
    }

    #[test]
    fn unknown_layer_is_not_found() {
        let mut s = session();
        assert!(matches!(
            s.stroke_begin(9999, white(), brush()),
            Err(EditorEngineError::LayerNotFound(9999))
        ));
    }

    #[test]
    fn undo_right_after_points_undoes_the_whole_stroke() {
        let mut s = session();
        let bg = background(&s);
        let before = pixels(&s);
        let id = s.stroke_begin(bg, white(), brush()).unwrap();
        s.stroke_points(id, &dabs()).unwrap();
        // No stroke_end: undo commits the stroke, then undoes it.
        s.undo().unwrap();
        assert!(!s.has_stroke());
        assert_eq!(pixels(&s), before);
        assert!(s.info(1).can_redo);
    }

    #[test]
    fn edit_and_begin_and_redo_commit_the_stroke() {
        let mut s = session();
        let bg = background(&s);
        let before = pixels(&s);
        // Edit.
        let id = s.stroke_begin(bg, white(), brush()).unwrap();
        s.stroke_points(id, &dabs()).unwrap();
        s.edit(
            EditDto::AddAdjustment {
                adjustment: bc(0.2),
                above_layer_id: None,
            },
            None,
        )
        .unwrap();
        assert!(!s.has_stroke());
        s.undo().unwrap(); // the adjustment
        assert_ne!(pixels(&s), before);
        s.undo().unwrap(); // the stroke
        assert_eq!(pixels(&s), before);
        // Begin.
        let first = s.stroke_begin(bg, white(), brush()).unwrap();
        s.stroke_points(first, &dabs()).unwrap();
        let second = s.stroke_begin(bg, white(), brush()).unwrap();
        assert_ne!(first, second);
        assert!(matches!(
            s.stroke_points(first, &dabs()),
            Err(EditorEngineError::StrokeNotActive(_))
        ));
        s.stroke_cancel(second).unwrap();
        s.undo().unwrap();
        assert_eq!(pixels(&s), before);
        // Redo.
        let id = s.stroke_begin(bg, white(), brush()).unwrap();
        s.stroke_points(id, &dabs()).unwrap();
        s.redo().unwrap();
        assert!(!s.has_stroke());
        assert!(s.info(1).can_undo);
    }

    #[test]
    fn stroke_changes_the_layer_content_version() {
        let mut s = session();
        let bg = background(&s);
        let id = s.stroke_begin(bg, white(), brush()).unwrap();
        let v0 = s.document().layers()[0].pixel_version();
        s.stroke_points(id, &dabs()).unwrap();
        assert_ne!(s.document().layers()[0].pixel_version(), v0);
    }
}
