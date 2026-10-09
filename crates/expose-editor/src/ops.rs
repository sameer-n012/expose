//! Document operations and reversible edits.
//!
//! Each operation checks its input and the layer locks, changes the
//! document, and returns an [`Edit`]. The edit is the **inverse** change:
//! apply it to undo the operation. Give the edits to
//! [`History::push`](crate::History::push) to make an undo step.
//!
//! An [`Edit`] is opaque. Only operations make edits, so that callers
//! cannot go around the lock checks. Undo and redo apply edits without lock
//! checks, because they restore an earlier state exactly (the lock state
//! itself is part of the history).
//!
//! # Memory
//!
//! Pixel and mask writes store only the old values of the written
//! rectangle, not a copy of the layer. The values are swapped in place, so
//! undo and redo need no extra copy. Delete stores the removed layer (with
//! its pixels), because undo must restore it.

use expose_core::buffer::{Rect, Rgba};

use crate::adjust::Adjustment;
use crate::blend::{BlendMode, BlendSpace};
use crate::document::Document;
use crate::error::{EditorError, EditorResult, check_range};
use crate::layer::{
    Group, GroupIsolation, Layer, LayerContent, LayerId, LayerKind, LayerProps, LockKind, Locks,
    Mask, Placement,
};

/// A reversible change to a document. See the module documentation.
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    kind: EditKind,
}

#[derive(Debug, Clone, PartialEq)]
enum EditKind {
    Insert {
        at: Placement,
        layer: Box<Layer>,
    },
    Remove {
        id: LayerId,
    },
    Move {
        id: LayerId,
        to: Placement,
    },
    Props {
        id: LayerId,
        props: LayerProps,
    },
    Isolation {
        id: LayerId,
        isolation: GroupIsolation,
    },
    Adjustment {
        id: LayerId,
        adjustment: Box<Adjustment>,
    },
    Mask {
        id: LayerId,
        mask: Option<Box<Mask>>,
    },
    MaskEnabled {
        id: LayerId,
        enabled: bool,
    },
    Translate {
        id: LayerId,
        dx: i32,
        dy: i32,
    },
    /// `rect` is in layer pixel coordinates.
    PixelPatch {
        id: LayerId,
        rect: Rect,
        data: Vec<Rgba>,
    },
    /// `rect` is in mask pixel coordinates.
    MaskPatch {
        id: LayerId,
        rect: Rect,
        data: Vec<f32>,
    },
    BlendSpace(BlendSpace),
}

impl Edit {
    const fn new(kind: EditKind) -> Self {
        Self { kind }
    }

    /// Returns the approximate heap memory of the edit in bytes. The
    /// history uses it for its memory report.
    pub fn memory_bytes(&self) -> usize {
        let base = size_of::<Self>();
        base + match &self.kind {
            EditKind::Insert { layer, .. } => layer_bytes(layer),
            EditKind::Props { props, .. } => props.name.capacity(),
            EditKind::Adjustment { adjustment, .. } => match adjustment.as_ref() {
                Adjustment::Curves(c) => {
                    (c.master.len() + c.red.len() + c.green.len() + c.blue.len()) * 8
                }
                _ => size_of::<Adjustment>(),
            },
            EditKind::Mask { mask, .. } => mask.as_ref().map_or(0, |m| size_of_val(m.image.data())),
            EditKind::PixelPatch { data, .. } => size_of_val(data.as_slice()),
            EditKind::MaskPatch { data, .. } => size_of_val(data.as_slice()),
            EditKind::Remove { .. }
            | EditKind::Move { .. }
            | EditKind::Isolation { .. }
            | EditKind::MaskEnabled { .. }
            | EditKind::Translate { .. }
            | EditKind::BlendSpace(_) => 0,
        }
    }
}

/// Returns the approximate heap memory of a layer subtree.
fn layer_bytes(layer: &Layer) -> usize {
    let mut n = 0;
    layer.visit(&mut |l| {
        n += size_of::<Layer>() + l.name().len();
        if let Some(p) = l.as_pixel() {
            n += p.image().as_bytes().len();
        }
        if let Some(m) = l.mask() {
            n += size_of_val(m.image.data());
        }
    });
    n
}

/// Swaps the values of `rect` in a row-major buffer with `data`.
///
/// The caller must prove that `rect` is inside the buffer and that
/// `data.len()` is the area of `rect`.
fn swap_rect<T>(buf: &mut [T], buf_width: u32, rect: Rect, data: &mut [T]) {
    let bw = buf_width as usize;
    let (x0, w) = (rect.x as usize, rect.width as usize);
    for (row, chunk) in data.chunks_exact_mut(w).enumerate() {
        let start = (rect.y as usize + row) * bw + x0;
        buf[start..start + w].swap_with_slice(chunk);
    }
}

/// Checks that `rect` is not empty, is inside `bounds`, and matches `len`.
fn check_patch(rect: Rect, bounds: Rect, len: usize) -> EditorResult<()> {
    if rect.is_empty() || !bounds.contains_rect(&rect) {
        return Err(EditorError::InvalidRect {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        });
    }
    // The rect is inside a buffer that fits in memory, so the area fits in
    // usize.
    let expected = usize::try_from(rect.area()).map_err(|_| EditorError::InvalidRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    })?;
    if expected != len {
        return Err(EditorError::LengthMismatch {
            expected,
            actual: len,
        });
    }
    Ok(())
}

fn add_offset(offset: (i32, i32), dx: i32, dy: i32) -> EditorResult<(i32, i32)> {
    Ok((
        offset
            .0
            .checked_add(dx)
            .ok_or(EditorError::OffsetOverflow)?,
        offset
            .1
            .checked_add(dy)
            .ok_or(EditorError::OffsetOverflow)?,
    ))
}

impl Document {
    // ---------------------------------------------------------------
    // Raw edit application (no lock checks). Used by operations and by the
    // history.
    // ---------------------------------------------------------------

    /// Applies an edit and returns its inverse.
    ///
    /// If the function returns an error, the document did not change.
    ///
    /// Versions: each applied edit gives the document a new revision. Pixel
    /// patches give the layer a new pixel version. Mask patches and mask
    /// replacements give the layer a new mask version. Insert and remove
    /// keep the versions of the layer: a layer that comes back by undo has
    /// the same values as before, so its old versions are still correct.
    /// A move (translate) keeps the versions, because the values do not
    /// change (only the offsets).
    pub(crate) fn apply_edit(&mut self, edit: Edit) -> EditorResult<Edit> {
        let inverse = match edit.kind {
            EditKind::Insert { at, layer } => self.raw_insert(at, *layer)?,
            EditKind::Remove { id } => self.raw_remove(id)?,
            EditKind::Move { id, to } => self.raw_move(id, to)?,
            EditKind::Props { id, props } => {
                let old = std::mem::replace(self.layer_mut(id)?.props_mut(), props);
                EditKind::Props { id, props: old }
            }
            EditKind::Isolation { id, isolation } => {
                let LayerKind::Group(g) = self.layer_mut(id)?.kind_mut() else {
                    return Err(EditorError::NotAGroup(id));
                };
                let old = g.isolation();
                g.set_isolation(isolation);
                EditKind::Isolation { id, isolation: old }
            }
            EditKind::Adjustment { id, adjustment } => {
                let LayerKind::Adjustment(a) = self.layer_mut(id)?.kind_mut() else {
                    return Err(EditorError::NotAnAdjustmentLayer(id));
                };
                let old = std::mem::replace(a, *adjustment);
                EditKind::Adjustment {
                    id,
                    adjustment: Box::new(old),
                }
            }
            EditKind::Mask { id, mask } => {
                let layer = self.layer_mut(id)?;
                let old = layer.replace_mask(mask.map(|m| *m));
                layer.touch_mask();
                EditKind::Mask {
                    id,
                    mask: old.map(Box::new),
                }
            }
            EditKind::MaskEnabled { id, enabled } => {
                let m = self
                    .layer_mut(id)?
                    .mask_mut()
                    .ok_or(EditorError::NoMask(id))?;
                let old = m.enabled;
                m.enabled = enabled;
                EditKind::MaskEnabled { id, enabled: old }
            }
            EditKind::Translate { id, dx, dy } => self.raw_translate(id, dx, dy)?,
            EditKind::PixelPatch { id, rect, mut data } => {
                let layer = self.layer_mut(id)?;
                let LayerKind::Pixel(p) = layer.kind_mut() else {
                    return Err(EditorError::NotAPixelLayer(id));
                };
                let img = p.image_mut();
                check_patch(rect, img.bounds(), data.len())?;
                let w = img.width();
                swap_rect(img.pixels_mut(), w, rect, &mut data);
                layer.touch_pixels();
                EditKind::PixelPatch { id, rect, data }
            }
            EditKind::MaskPatch { id, rect, mut data } => {
                let layer = self.layer_mut(id)?;
                let m = layer.mask_mut().ok_or(EditorError::NoMask(id))?;
                check_patch(rect, m.image.bounds(), data.len())?;
                let w = m.image.width();
                swap_rect(m.image.data_mut(), w, rect, &mut data);
                layer.touch_mask();
                EditKind::MaskPatch { id, rect, data }
            }
            EditKind::BlendSpace(space) => {
                let old = std::mem::replace(&mut self.blend_space, space);
                EditKind::BlendSpace(old)
            }
        };
        // The edit is applied (each error above returns before a change).
        self.touch();
        Ok(Edit::new(inverse))
    }

    fn raw_insert(&mut self, at: Placement, layer: Layer) -> EditorResult<EditKind> {
        // IDs must stay unique.
        let mut ids = Vec::new();
        layer.visit(&mut |l| ids.push(l.id()));
        let existing = self.layer_ids();
        for id in &ids {
            if existing.contains(id) || ids.iter().filter(|x| *x == id).count() > 1 {
                return Err(EditorError::DuplicateId(*id));
            }
        }
        let len = self.child_count(at.parent)?;
        if at.index > len {
            return Err(EditorError::IndexOutOfRange {
                index: at.index,
                len,
            });
        }
        for id in &ids {
            self.reserve_id(*id);
        }
        let id = layer.id();
        self.children_mut(at.parent)?.insert(at.index, layer);
        Ok(EditKind::Remove { id })
    }

    fn raw_remove(&mut self, id: LayerId) -> EditorResult<EditKind> {
        let at = self
            .placement_of(id)
            .ok_or(EditorError::LayerNotFound(id))?;
        let layer = self.children_mut(at.parent)?.remove(at.index);
        Ok(EditKind::Insert {
            at,
            layer: Box::new(layer),
        })
    }

    fn raw_move(&mut self, id: LayerId, to: Placement) -> EditorResult<EditKind> {
        let from = self
            .placement_of(id)
            .ok_or(EditorError::LayerNotFound(id))?;
        if let Some(parent) = to.parent {
            // Cycle check: the new parent must not be the layer or inside it.
            if parent == id || self.ancestors(parent)?.contains(&id) {
                return Err(EditorError::Cycle(id));
            }
        }
        // The insert index is in the list after the removal.
        let mut len = self.child_count(to.parent)?;
        if to.parent == from.parent {
            len -= 1;
        }
        if to.index > len {
            return Err(EditorError::IndexOutOfRange {
                index: to.index,
                len,
            });
        }
        let layer = self.children_mut(from.parent)?.remove(from.index);
        self.children_mut(to.parent)?.insert(to.index, layer);
        Ok(EditKind::Move { id, to: from })
    }

    fn raw_translate(&mut self, id: LayerId, dx: i32, dy: i32) -> EditorResult<EditKind> {
        let inverse = (
            dx.checked_neg().ok_or(EditorError::OffsetOverflow)?,
            dy.checked_neg().ok_or(EditorError::OffsetOverflow)?,
        );
        let layer = self.layer_mut(id)?;
        // Check all offsets first, so that an overflow changes nothing.
        let mut overflow = false;
        layer.visit(&mut |l| {
            if let Some(p) = l.as_pixel() {
                overflow |= add_offset(p.offset(), dx, dy).is_err();
            }
            if let Some(m) = l.mask() {
                overflow |= add_offset(m.offset, dx, dy).is_err();
            }
        });
        if overflow {
            return Err(EditorError::OffsetOverflow);
        }
        layer.visit_mut(&mut |l| {
            if let LayerKind::Pixel(p) = l.kind_mut() {
                let (x, y) = p.offset();
                // Checked above.
                p.set_offset((x.wrapping_add(dx), y.wrapping_add(dy)));
            }
            if let Some(m) = l.mask_mut() {
                m.offset = (m.offset.0.wrapping_add(dx), m.offset.1.wrapping_add(dy));
            }
        });
        Ok(EditKind::Translate {
            id,
            dx: inverse.0,
            dy: inverse.1,
        })
    }

    // ---------------------------------------------------------------
    // Public operations.
    // ---------------------------------------------------------------

    fn require(&self, id: LayerId) -> EditorResult<&Layer> {
        self.layer(id).ok_or(EditorError::LayerNotFound(id))
    }

    fn check_lock(&self, id: LayerId, kind: LockKind) -> EditorResult<()> {
        self.effective_locks(id)?.check(id, kind)
    }

    /// Changes the common properties of a layer with `f`.
    fn update_props(
        &mut self,
        id: LayerId,
        lock: Option<LockKind>,
        f: impl FnOnce(&mut LayerProps),
    ) -> EditorResult<Edit> {
        if let Some(kind) = lock {
            self.check_lock(id, kind)?;
        }
        let mut props = self.require(id)?.props().clone();
        f(&mut props);
        self.apply_edit(Edit::new(EditKind::Props { id, props }))
    }

    /// Adds a new layer at `at` and returns its ID.
    ///
    /// The new layer has default properties (see [`LayerProps::named`]).
    pub fn add_layer(
        &mut self,
        name: impl Into<String>,
        content: LayerContent,
        at: Placement,
    ) -> EditorResult<(LayerId, Edit)> {
        let kind = match content {
            LayerContent::Pixel(p) => LayerKind::Pixel(p),
            LayerContent::Group(isolation) => {
                let mut g = Group::default();
                g.set_isolation(isolation);
                LayerKind::Group(g)
            }
            LayerContent::Adjustment(a) => {
                a.validate()?;
                LayerKind::Adjustment(a)
            }
        };
        // Check the placement before the ID is used.
        let len = self.child_count(at.parent)?;
        if at.index > len {
            return Err(EditorError::IndexOutOfRange {
                index: at.index,
                len,
            });
        }
        let id = self.alloc_id();
        let layer = Layer::new(id, LayerProps::named(name), kind);
        let undo = self.apply_edit(Edit::new(EditKind::Insert {
            at,
            layer: Box::new(layer),
        }))?;
        Ok((id, undo))
    }

    /// Deletes a layer (and its children, for a group).
    ///
    /// Locks do not prevent a delete.
    pub fn delete_layer(&mut self, id: LayerId) -> EditorResult<Edit> {
        self.apply_edit(Edit::new(EditKind::Remove { id }))
    }

    /// Makes a deep copy of a layer (and its children) with new IDs, and
    /// puts it directly above the original. The copy has the name
    /// "`<name>` copy". Returns the ID of the copy.
    pub fn duplicate_layer(&mut self, id: LayerId) -> EditorResult<(LayerId, Edit)> {
        let at = self
            .placement_of(id)
            .ok_or(EditorError::LayerNotFound(id))?;
        let mut copy = self.require(id)?.clone();
        // Make the new IDs first (the visit closure cannot borrow self).
        let mut count = 0_usize;
        copy.visit(&mut |_| count += 1);
        let mut new_ids: Vec<LayerId> = (0..count).map(|_| self.alloc_id()).collect();
        new_ids.reverse();
        copy.visit_mut(&mut |l| {
            // `new_ids` has one ID for each layer in the subtree.
            if let Some(nid) = new_ids.pop() {
                l.set_id(nid);
            }
        });
        let new_id = copy.id();
        let name = format!("{} copy", copy.name());
        copy.props_mut().name = name;
        let undo = self.apply_edit(Edit::new(EditKind::Insert {
            at: Placement::new(at.parent, at.index + 1),
            layer: Box::new(copy),
        }))?;
        Ok((new_id, undo))
    }

    /// Moves a layer to a new position in the tree (reorder, into a group,
    /// or out of a group). `to.index` is in the child list after the layer
    /// is removed from its old position.
    ///
    /// Returns [`EditorError::Cycle`] if `to.parent` is the layer or one of
    /// its descendants. Locks do not prevent a reorder (the position lock
    /// is for the canvas offset).
    pub fn move_layer(&mut self, id: LayerId, to: Placement) -> EditorResult<Edit> {
        self.apply_edit(Edit::new(EditKind::Move { id, to }))
    }

    /// Renames a layer.
    pub fn rename_layer(&mut self, id: LayerId, name: impl Into<String>) -> EditorResult<Edit> {
        let name = name.into();
        self.update_props(id, None, |p| p.name = name)
    }

    /// Shows or hides a layer.
    pub fn set_visible(&mut self, id: LayerId, visible: bool) -> EditorResult<Edit> {
        self.update_props(id, None, |p| p.visible = visible)
    }

    /// Sets the opacity, `0.0..=1.0`. The `all` lock prevents it.
    pub fn set_opacity(&mut self, id: LayerId, opacity: f32) -> EditorResult<Edit> {
        check_range("opacity", opacity, 0.0, 1.0)?;
        self.update_props(id, Some(LockKind::All), |p| p.opacity = opacity)
    }

    /// Sets the fill opacity, `0.0..=1.0`. The `all` lock prevents it.
    pub fn set_fill_opacity(&mut self, id: LayerId, fill: f32) -> EditorResult<Edit> {
        check_range("fill_opacity", fill, 0.0, 1.0)?;
        self.update_props(id, Some(LockKind::All), |p| p.fill_opacity = fill)
    }

    /// Sets the blend mode. The `all` lock prevents it.
    pub fn set_blend_mode(&mut self, id: LayerId, mode: BlendMode) -> EditorResult<Edit> {
        self.update_props(id, Some(LockKind::All), |p| p.blend_mode = mode)
    }

    /// Sets the locks of the layer. This is always permitted.
    pub fn set_locks(&mut self, id: LayerId, locks: Locks) -> EditorResult<Edit> {
        self.update_props(id, None, |p| p.locks = locks)
    }

    /// Sets the isolation mode of a group. The `all` lock prevents it.
    pub fn set_group_isolation(
        &mut self,
        id: LayerId,
        isolation: GroupIsolation,
    ) -> EditorResult<Edit> {
        self.check_lock(id, LockKind::All)?;
        self.apply_edit(Edit::new(EditKind::Isolation { id, isolation }))
    }

    /// Replaces the setting of an adjustment layer. The `all` lock prevents
    /// it.
    pub fn set_adjustment(&mut self, id: LayerId, adjustment: Adjustment) -> EditorResult<Edit> {
        adjustment.validate()?;
        self.check_lock(id, LockKind::All)?;
        self.apply_edit(Edit::new(EditKind::Adjustment {
            id,
            adjustment: Box::new(adjustment),
        }))
    }

    /// Moves a layer on the canvas by `(dx, dy)` pixels. For a group, all
    /// layers in the group move. Masks move with their layers.
    ///
    /// The position lock (or `all`) of the layer, of a parent group, or of
    /// any layer in the group prevents it.
    pub fn translate_layer(&mut self, id: LayerId, dx: i32, dy: i32) -> EditorResult<Edit> {
        self.check_lock(id, LockKind::Position)?;
        let mut locked: Option<EditorError> = None;
        self.require(id)?.visit(&mut |l| {
            if locked.is_none() {
                locked = l.locks().check(l.id(), LockKind::Position).err();
            }
        });
        if let Some(e) = locked {
            return Err(e);
        }
        self.apply_edit(Edit::new(EditKind::Translate { id, dx, dy }))
    }

    /// Adds, replaces, or removes (`None`) the mask of a layer. The pixels
    /// lock prevents it.
    pub fn set_mask(&mut self, id: LayerId, mask: Option<Mask>) -> EditorResult<Edit> {
        self.check_lock(id, LockKind::Pixels)?;
        if let Some(m) = &mask {
            if m.image.data().iter().any(|v| !v.is_finite()) {
                return Err(EditorError::NonFinite { name: "mask data" });
            }
        }
        self.apply_edit(Edit::new(EditKind::Mask {
            id,
            mask: mask.map(Box::new),
        }))
    }

    /// Enables or disables the mask of a layer. The `all` lock prevents it.
    pub fn set_mask_enabled(&mut self, id: LayerId, enabled: bool) -> EditorResult<Edit> {
        self.check_lock(id, LockKind::All)?;
        self.apply_edit(Edit::new(EditKind::MaskEnabled { id, enabled }))
    }

    /// Writes premultiplied pixels into `rect` (layer pixel coordinates) of
    /// a pixel layer. `data` is row-major with `rect.width * rect.height`
    /// pixels.
    ///
    /// - The pixels lock (or `all`) prevents it.
    /// - With the transparency lock, the write must keep the alpha of each
    ///   pixel, else the function returns
    ///   [`EditorError::TransparencyLocked`]. (A paint tool must keep alpha
    ///   itself; this function does not change the data silently.)
    /// - All values must be finite.
    ///
    /// The returned edit holds only the old values of `rect`.
    pub fn write_pixels(&mut self, id: LayerId, rect: Rect, data: &[Rgba]) -> EditorResult<Edit> {
        let locks = self.effective_locks(id)?;
        locks.check(id, LockKind::Pixels)?;
        let p = self
            .require(id)?
            .as_pixel()
            .ok_or(EditorError::NotAPixelLayer(id))?;
        let img = p.image();
        check_patch(rect, img.bounds(), data.len())?;
        if data.iter().flatten().any(|v| !v.is_finite()) {
            return Err(EditorError::NonFinite { name: "pixel data" });
        }
        if locks.transparency {
            let w = rect.width as usize;
            for (row, chunk) in data.chunks_exact(w).enumerate() {
                let y = rect.y + u32::try_from(row).map_err(|_| EditorError::OffsetOverflow)?;
                let old = img.row(y).ok_or(EditorError::InvalidRect {
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                })?;
                let old = &old[rect.x as usize..rect.x as usize + w];
                // Exact comparison is intended: the lock keeps alpha
                // bit-for-bit, so that undo history and output agree.
                #[allow(clippy::float_cmp)]
                let changed = old.iter().zip(chunk).any(|(o, n)| o[3] != n[3]);
                if changed {
                    return Err(EditorError::TransparencyLocked(id));
                }
            }
        }
        self.apply_edit(Edit::new(EditKind::PixelPatch {
            id,
            rect,
            data: data.to_vec(),
        }))
    }

    /// Writes mask values into `rect` (mask pixel coordinates). The pixels
    /// lock (or `all`) prevents it. All values must be finite.
    pub fn write_mask(&mut self, id: LayerId, rect: Rect, data: &[f32]) -> EditorResult<Edit> {
        self.check_lock(id, LockKind::Pixels)?;
        let m = self.require(id)?.mask().ok_or(EditorError::NoMask(id))?;
        check_patch(rect, m.image.bounds(), data.len())?;
        if data.iter().any(|v| !v.is_finite()) {
            return Err(EditorError::NonFinite { name: "mask data" });
        }
        self.apply_edit(Edit::new(EditKind::MaskPatch {
            id,
            rect,
            data: data.to_vec(),
        }))
    }

    /// Sets the blending space of the compositor.
    pub fn set_blend_space(&mut self, space: BlendSpace) -> EditorResult<Edit> {
        self.apply_edit(Edit::new(EditKind::BlendSpace(space)))
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // Tests compare exact expected values.
mod tests {
    use super::*;
    use crate::adjust::{BrightnessContrast, HueSaturation};
    use crate::document::DocumentSpec;
    use crate::layer::{MaskFill, MaskImage, PixelLayer};
    use expose_core::buffer::WorkingImage;

    fn doc() -> Document {
        Document::new(DocumentSpec::new(4, 4)).unwrap()
    }

    fn pixel(w: u32, h: u32, c: Rgba) -> LayerContent {
        LayerContent::Pixel(
            PixelLayer::new(WorkingImage::new_filled(w, h, c).unwrap(), (0, 0)).unwrap(),
        )
    }

    fn names(list: &[Layer]) -> Vec<&str> {
        list.iter().map(Layer::name).collect()
    }

    #[test]
    fn add_delete_and_undo_edits() {
        let mut d = doc();
        let (a, undo_add) = d
            .add_layer("A", pixel(2, 2, [1.0; 4]), Placement::root(1))
            .unwrap();
        assert_eq!(names(d.layers()), ["Background", "A"]);
        let undo_del = d.delete_layer(a).unwrap();
        assert!(!d.contains(a));
        // Undo the delete: the layer comes back with the same ID.
        d.apply_edit(undo_del).unwrap();
        assert_eq!(d.placement_of(a), Some(Placement::root(1)));
        // Undo the add.
        d.apply_edit(undo_add).unwrap();
        assert_eq!(names(d.layers()), ["Background"]);
        // Bad placement.
        assert!(matches!(
            d.add_layer("X", pixel(1, 1, [0.0; 4]), Placement::root(5)),
            Err(EditorError::IndexOutOfRange { index: 5, len: 1 })
        ));
        let bg = d.layers()[0].id();
        assert!(matches!(
            d.add_layer("X", pixel(1, 1, [0.0; 4]), Placement::new(Some(bg), 0)),
            Err(EditorError::NotAGroup(_))
        ));
        // Bad adjustment parameters.
        let bad = Adjustment::BrightnessContrast(BrightnessContrast {
            brightness: 2.0,
            contrast: 0.0,
        });
        assert!(
            d.add_layer("adj", LayerContent::Adjustment(bad), Placement::root(0))
                .is_err()
        );
    }

    #[test]
    fn duplicate_is_deep_with_new_ids() {
        let mut d = doc();
        let (g, _) = d
            .add_layer(
                "G",
                LayerContent::Group(GroupIsolation::Isolated),
                Placement::root(1),
            )
            .unwrap();
        let (c1, _) = d
            .add_layer("C1", pixel(1, 1, [0.5; 4]), Placement::new(Some(g), 0))
            .unwrap();
        let (c2, _) = d
            .add_layer("C2", pixel(1, 1, [0.25; 4]), Placement::new(Some(g), 1))
            .unwrap();
        let (copy, undo) = d.duplicate_layer(g).unwrap();
        assert_eq!(names(d.layers()), ["Background", "G", "G copy"]);
        let copy_layer = d.layer(copy).unwrap();
        let kids: Vec<LayerId> = copy_layer.children().iter().map(Layer::id).collect();
        assert_eq!(kids.len(), 2);
        assert!(!kids.contains(&c1) && !kids.contains(&c2) && copy != g);
        assert_eq!(names(copy_layer.children()), ["C1", "C2"]);
        // All IDs unique.
        let mut ids = d.layer_ids();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
        // Deep copy: a change to the copy does not change the original.
        let copied_child = kids[0];
        d.write_pixels(copied_child, Rect::new(0, 0, 1, 1), &[[0.0; 4]])
            .unwrap();
        assert_eq!(
            d.layer(c1).unwrap().as_pixel().unwrap().image().pixel(0, 0),
            Some([0.5; 4])
        );
        d.apply_edit(undo).unwrap();
        assert!(!d.contains(copy));
    }

    #[test]
    fn reorder_into_and_out_of_groups() {
        let mut d = doc();
        let bg = d.layers()[0].id();
        let (a, _) = d
            .add_layer("A", pixel(1, 1, [1.0; 4]), Placement::root(1))
            .unwrap();
        let (b, _) = d
            .add_layer("B", pixel(1, 1, [1.0; 4]), Placement::root(2))
            .unwrap();
        let (g, _) = d
            .add_layer(
                "G",
                LayerContent::Group(GroupIsolation::PassThrough),
                Placement::root(3),
            )
            .unwrap();
        // Move to index: B to the bottom.
        let undo = d.move_layer(b, Placement::root(0)).unwrap();
        assert_eq!(names(d.layers()), ["B", "Background", "A", "G"]);
        d.apply_edit(undo).unwrap();
        assert_eq!(names(d.layers()), ["Background", "A", "B", "G"]);
        // Into a group.
        d.move_layer(a, Placement::new(Some(g), 0)).unwrap();
        d.move_layer(b, Placement::new(Some(g), 1)).unwrap();
        assert_eq!(names(d.layers()), ["Background", "G"]);
        assert_eq!(names(d.layer(g).unwrap().children()), ["A", "B"]);
        assert_eq!(d.ancestors(b).unwrap(), vec![g]);
        // Out of the group.
        let undo = d.move_layer(a, Placement::root(0)).unwrap();
        assert_eq!(names(d.layers()), ["A", "Background", "G"]);
        d.apply_edit(undo).unwrap();
        assert_eq!(names(d.layer(g).unwrap().children()), ["A", "B"]);
        // Index after removal: moving within the same list.
        assert!(d.move_layer(bg, Placement::root(1)).is_ok());
        assert!(matches!(
            d.move_layer(bg, Placement::root(2)),
            Err(EditorError::IndexOutOfRange { .. })
        ));
        // Cycle prevention.
        let (inner, _) = d
            .add_layer(
                "Inner",
                LayerContent::Group(GroupIsolation::Isolated),
                Placement::new(Some(g), 0),
            )
            .unwrap();
        assert_eq!(
            d.move_layer(g, Placement::new(Some(g), 0)),
            Err(EditorError::Cycle(g))
        );
        assert_eq!(
            d.move_layer(g, Placement::new(Some(inner), 0)),
            Err(EditorError::Cycle(g))
        );
        // Into a non-group.
        assert_eq!(
            d.move_layer(a, Placement::new(Some(bg), 0)),
            Err(EditorError::NotAGroup(bg))
        );
    }

    #[test]
    fn locks_are_enforced() {
        let mut d = doc();
        let bg = d.layers()[0].id();
        let all = Locks {
            all: true,
            ..Locks::NONE
        };
        let position = Locks {
            position: true,
            ..Locks::NONE
        };
        let pixels = Locks {
            pixels: true,
            ..Locks::NONE
        };
        let transparency = Locks {
            transparency: true,
            ..Locks::NONE
        };

        d.set_locks(bg, position).unwrap();
        assert_eq!(
            d.translate_layer(bg, 1, 0),
            Err(EditorError::Locked {
                id: bg,
                lock: LockKind::Position
            })
        );
        // Position lock does not prevent paint or opacity.
        d.write_pixels(bg, Rect::new(0, 0, 1, 1), &[[0.5, 0.5, 0.5, 1.0]])
            .unwrap();
        d.set_opacity(bg, 0.5).unwrap();

        d.set_locks(bg, pixels).unwrap();
        assert!(matches!(
            d.write_pixels(bg, Rect::new(0, 0, 1, 1), &[[0.0; 4]]),
            Err(EditorError::Locked {
                lock: LockKind::Pixels,
                ..
            })
        ));
        d.translate_layer(bg, 1, 1).unwrap();

        d.set_locks(bg, transparency).unwrap();
        // Same alpha: permitted.
        d.write_pixels(bg, Rect::new(0, 0, 1, 1), &[[0.1, 0.2, 0.3, 1.0]])
            .unwrap();
        // Alpha change: refused.
        assert_eq!(
            d.write_pixels(bg, Rect::new(0, 0, 1, 1), &[[0.0; 4]]),
            Err(EditorError::TransparencyLocked(bg))
        );

        d.set_locks(bg, all).unwrap();
        assert!(d.set_opacity(bg, 1.0).is_err());
        assert!(d.set_blend_mode(bg, BlendMode::Multiply).is_err());
        assert!(d.translate_layer(bg, 1, 0).is_err());
        assert!(
            d.write_pixels(bg, Rect::new(0, 0, 1, 1), &[[0.0; 4]])
                .is_err()
        );
        // Always permitted.
        d.rename_layer(bg, "Base").unwrap();
        d.set_visible(bg, false).unwrap();
        d.set_locks(bg, Locks::NONE).unwrap();
        d.set_opacity(bg, 1.0).unwrap();

        // A group lock applies to its children; a child lock stops a group move.
        let (g, _) = d
            .add_layer(
                "G",
                LayerContent::Group(GroupIsolation::Isolated),
                Placement::root(1),
            )
            .unwrap();
        d.move_layer(bg, Placement::new(Some(g), 0)).unwrap();
        d.set_locks(g, pixels).unwrap();
        assert!(
            d.write_pixels(bg, Rect::new(0, 0, 1, 1), &[[0.0; 4]])
                .is_err()
        );
        d.set_locks(g, Locks::NONE).unwrap();
        d.set_locks(bg, position).unwrap();
        assert_eq!(
            d.translate_layer(g, 1, 0),
            Err(EditorError::Locked {
                id: bg,
                lock: LockKind::Position
            })
        );
    }

    #[test]
    fn property_setters_validate_and_undo() {
        let mut d = doc();
        let bg = d.layers()[0].id();
        assert!(matches!(
            d.set_opacity(bg, 1.5),
            Err(EditorError::OutOfRange { .. })
        ));
        assert!(matches!(
            d.set_fill_opacity(bg, f32::NAN),
            Err(EditorError::NonFinite { .. })
        ));
        let undo = d.set_fill_opacity(bg, 0.25).unwrap();
        assert_eq!(d.layer(bg).unwrap().fill_opacity(), 0.25);
        d.apply_edit(undo).unwrap();
        assert_eq!(d.layer(bg).unwrap().fill_opacity(), 1.0);
        // Kind-specific operations check the kind.
        assert_eq!(
            d.set_group_isolation(bg, GroupIsolation::Isolated),
            Err(EditorError::NotAGroup(bg))
        );
        assert_eq!(
            d.set_adjustment(bg, Adjustment::HueSaturation(HueSaturation::default())),
            Err(EditorError::NotAnAdjustmentLayer(bg))
        );
        assert_eq!(d.set_mask_enabled(bg, false), Err(EditorError::NoMask(bg)));
    }

    #[test]
    fn translate_moves_masks_and_rejects_overflow() {
        let mut d = doc();
        let bg = d.layers()[0].id();
        let mask = Mask::new(
            MaskImage::new_filled(2, 2, 1.0).unwrap(),
            (1, 1),
            MaskFill::Black,
        );
        d.set_mask(bg, Some(mask)).unwrap();
        let undo = d.translate_layer(bg, -3, 2).unwrap();
        let l = d.layer(bg).unwrap();
        assert_eq!(l.as_pixel().unwrap().offset(), (-3, 2));
        assert_eq!(l.mask().unwrap().offset, (-2, 3));
        d.apply_edit(undo).unwrap();
        assert_eq!(d.layer(bg).unwrap().as_pixel().unwrap().offset(), (0, 0));
        d.translate_layer(bg, i32::MAX - 1, 0).unwrap();
        // The pixel offset fits, but the mask offset overflows.
        assert_eq!(
            d.translate_layer(bg, 1, 0),
            Err(EditorError::OffsetOverflow)
        );
        // Nothing changed after the failed move.
        let l = d.layer(bg).unwrap();
        assert_eq!(l.mask().unwrap().offset, (i32::MAX, 1));
        assert_eq!(l.as_pixel().unwrap().offset(), (i32::MAX - 1, 0));
        assert_eq!(
            d.translate_layer(bg, i32::MIN, 0),
            Err(EditorError::OffsetOverflow)
        );
    }

    #[test]
    fn patches_store_only_the_rect() {
        let mut d = Document::new(DocumentSpec::new(64, 64)).unwrap();
        let bg = d.layers()[0].id();
        let undo = d
            .write_pixels(bg, Rect::new(10, 10, 4, 2), &[[0.0, 0.0, 0.0, 1.0]; 8])
            .unwrap();
        assert!(undo.memory_bytes() < 64 * 64 * 16 / 10);
        assert!(undo.memory_bytes() >= 8 * 16);
        let img = d.layer(bg).unwrap().as_pixel().unwrap().image().clone();
        assert_eq!(img.pixel(13, 11), Some([0.0, 0.0, 0.0, 1.0]));
        assert_eq!(img.pixel(14, 11), Some([1.0; 4]));
        let redo = d.apply_edit(undo).unwrap();
        assert_eq!(
            d.layer(bg)
                .unwrap()
                .as_pixel()
                .unwrap()
                .image()
                .pixel(13, 11),
            Some([1.0; 4])
        );
        d.apply_edit(redo).unwrap();
        assert_eq!(d.layer(bg).unwrap().as_pixel().unwrap().image(), &img);
        // Bad rects and data.
        assert!(matches!(
            d.write_pixels(bg, Rect::new(63, 0, 2, 1), &[[0.0; 4]; 2]),
            Err(EditorError::InvalidRect { .. })
        ));
        assert!(matches!(
            d.write_pixels(bg, Rect::new(0, 0, 2, 1), &[[0.0; 4]; 3]),
            Err(EditorError::LengthMismatch { .. })
        ));
        assert!(matches!(
            d.write_pixels(bg, Rect::new(0, 0, 1, 1), &[[f32::NAN; 4]]),
            Err(EditorError::NonFinite { .. })
        ));
        assert_eq!(
            d.write_mask(bg, Rect::new(0, 0, 1, 1), &[0.0]),
            Err(EditorError::NoMask(bg))
        );
    }
}
