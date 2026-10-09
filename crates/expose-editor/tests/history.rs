//! History tests: undo/redo round trips, redo branch, eviction, and jumps.

// Test helpers outside `#[test]` functions also unwrap.
#![allow(clippy::unwrap_used, clippy::float_cmp)]

use expose_core::buffer::{Rect, WorkingImage};
use expose_editor::{
    BlendMode, BlendSpace, Document, DocumentSpec, EditorError, GroupIsolation, History,
    HistoryEntry, Layer, LayerContent, Mask, MaskFill, MaskImage, PixelLayer, Placement, composite,
};

/// The observable state: layer tree, blend space, and composite. The ID
/// counter is not part of it (IDs are not used again after an undo).
#[derive(Clone, PartialEq)]
struct Snapshot {
    layers: Vec<Layer>,
    blend_space: BlendSpace,
    image: WorkingImage,
}

fn snapshot(doc: &Document) -> Snapshot {
    Snapshot {
        layers: doc.layers().to_vec(),
        blend_space: doc.blend_space(),
        image: composite(doc, doc.bounds()).unwrap(),
    }
}

#[test]
fn undo_redo_round_trip_restores_identical_pixels() {
    let mut doc = Document::new(DocumentSpec::new(16, 16)).unwrap();
    let mut h = History::default();
    let start = snapshot(&doc);
    let bg = doc.layers()[0].id();

    // Step 1: add a layer.
    let content = LayerContent::Pixel(
        PixelLayer::new(
            WorkingImage::new_filled(8, 8, [0.1, 0.2, 0.3, 0.5]).unwrap(),
            (4, 4),
        )
        .unwrap(),
    );
    let (l, e) = doc.add_layer("L", content, Placement::root(1)).unwrap();
    h.push("New Layer", vec![e]);
    // Step 2: a brush stroke made of several patches.
    let e1 = doc
        .write_pixels(l, Rect::new(0, 0, 3, 3), &[[0.4, 0.0, 0.0, 0.5]; 9])
        .unwrap();
    let e2 = doc
        .write_pixels(l, Rect::new(2, 2, 3, 3), &[[0.0, 0.4, 0.0, 0.5]; 9])
        .unwrap();
    h.push("Brush Stroke", vec![e1, e2]);
    // Step 3: properties, mask, move, group.
    let e1 = doc.set_blend_mode(l, BlendMode::Difference).unwrap();
    let e2 = doc.set_opacity(bg, 0.7).unwrap();
    let mask = Mask::new(
        MaskImage::new_filled(4, 4, 0.5).unwrap(),
        (5, 5),
        MaskFill::White,
    );
    let e3 = doc.set_mask(l, Some(mask)).unwrap();
    let e4 = doc.translate_layer(l, -2, 1).unwrap();
    let (g, e5) = doc
        .add_layer(
            "G",
            LayerContent::Group(GroupIsolation::Isolated),
            Placement::root(2),
        )
        .unwrap();
    let e6 = doc.move_layer(l, Placement::new(Some(g), 0)).unwrap();
    h.push("Many Changes", vec![e1, e2, e3, e4, e5, e6]);
    // Step 4: delete.
    let e = doc.delete_layer(g).unwrap();
    h.push("Delete Layer", vec![e]);

    let end = snapshot(&doc);
    let names: Vec<&str> = h.entries().map(HistoryEntry::name).collect();
    assert_eq!(
        names,
        ["New Layer", "Brush Stroke", "Many Changes", "Delete Layer"]
    );

    // Record every state while undoing.
    let mut states = vec![end.clone()];
    while h.undo(&mut doc).unwrap() {
        states.push(snapshot(&doc));
    }
    assert_eq!(h.cursor(), 0);
    assert_eq!(states.len(), 5);
    assert!(
        states.last() == Some(&start),
        "undo all must restore the start"
    );
    // Redo restores each state exactly. states[i] is the state at cursor 4 - i.
    for (i, want) in states.iter().enumerate().rev().skip(1) {
        assert!(h.redo(&mut doc).unwrap());
        assert!(&snapshot(&doc) == want, "redo to cursor {}", 4 - i);
    }
    assert!(!h.redo(&mut doc).unwrap());
    assert!(snapshot(&doc) == end);

    // Jump to a step and back.
    h.jump_to(&mut doc, 2).unwrap();
    assert!(snapshot(&doc) == states[2]);
    h.jump_to(&mut doc, 4).unwrap();
    assert!(snapshot(&doc) == end);
    assert!(matches!(
        h.jump_to(&mut doc, 5),
        Err(EditorError::IndexOutOfRange { .. })
    ));
}

#[test]
fn new_action_after_undo_drops_redo_branch() {
    let mut doc = Document::new(DocumentSpec::new(2, 2)).unwrap();
    let bg = doc.layers()[0].id();
    let mut h = History::default();
    for (i, o) in [0.9, 0.8, 0.7].into_iter().enumerate() {
        let e = doc.set_opacity(bg, o).unwrap();
        h.push(format!("Opacity {i}"), vec![e]);
    }
    h.undo(&mut doc).unwrap();
    h.undo(&mut doc).unwrap();
    assert!(h.can_redo());
    let e = doc.rename_layer(bg, "Base").unwrap();
    h.push("Rename", vec![e]);
    assert!(!h.can_redo());
    let names: Vec<&str> = h.entries().map(HistoryEntry::name).collect();
    assert_eq!(names, ["Opacity 0", "Rename"]);
    assert_eq!(doc.layer(bg).unwrap().opacity(), 0.9);
    h.undo(&mut doc).unwrap();
    h.undo(&mut doc).unwrap();
    assert_eq!(doc.layer(bg).unwrap().opacity(), 1.0);
    assert_eq!(doc.layer(bg).unwrap().name(), "Background");
    assert!(!h.undo(&mut doc).unwrap());
}

#[test]
fn eviction_removes_oldest_steps() {
    assert!(History::new(0).is_err());
    let mut doc = Document::new(DocumentSpec::new(2, 2)).unwrap();
    let bg = doc.layers()[0].id();
    let mut h = History::new(3).unwrap();
    for i in 0..5_u8 {
        let e = doc.set_opacity(bg, f32::from(i) / 10.0).unwrap();
        h.push(format!("Step {i}"), vec![e]);
    }
    let names: Vec<&str> = h.entries().map(HistoryEntry::name).collect();
    assert_eq!(names, ["Step 2", "Step 3", "Step 4"]);
    assert_eq!(h.cursor(), 3);
    while h.undo(&mut doc).unwrap() {}
    // The oldest restorable state is the one after step 1.
    assert_eq!(doc.layer(bg).unwrap().opacity(), 0.1);
}

#[test]
fn history_memory_counts_only_changed_regions() {
    let mut doc = Document::new(DocumentSpec::new(256, 256)).unwrap();
    let bg = doc.layers()[0].id();
    let mut h = History::default();
    let e = doc
        .write_pixels(bg, Rect::new(100, 100, 8, 8), &[[0.0, 0.0, 0.0, 1.0]; 64])
        .unwrap();
    h.push("Dab", vec![e]);
    let layer_bytes = 256 * 256 * 16;
    let used = h.memory_bytes();
    assert!(used >= 64 * 16, "used = {used}");
    assert!(used < layer_bytes / 100, "used = {used}");
}

#[test]
fn failed_undo_restores_document_and_drops_dependent_steps() {
    let mut doc = Document::new(DocumentSpec::new(2, 2)).unwrap();
    let bg = doc.layers()[0].id();
    let mut h = History::default();
    let e = doc.set_opacity(bg, 0.5).unwrap();
    h.push("Opacity", vec![e]);
    let (l, e) = doc
        .add_layer(
            "L",
            LayerContent::Group(GroupIsolation::Isolated),
            Placement::root(1),
        )
        .unwrap();
    let e2 = doc.rename_layer(l, "Renamed").unwrap();
    h.push("Add", vec![e, e2]);
    // Change the document without a history record: the layer is gone, so
    // the undo of "Add" fails at the rename edit.
    doc.delete_layer(l).unwrap();
    let before = doc.clone();
    assert!(h.undo(&mut doc).is_err());
    assert_eq!(doc, before);
    assert_eq!(h.entries().len(), 0);
    assert!(!h.can_undo());
}

/// A slider gesture: three opacity changes with one merge key.
fn opacity_gesture(doc: &mut Document, h: &mut History, key: u64, values: &[f32]) {
    let bg = doc.layers()[0].id();
    for &v in values {
        let e = doc.set_opacity(bg, v).unwrap();
        h.push_merge("Opacity", vec![e], key);
    }
}

fn opacity(doc: &Document) -> f32 {
    doc.layers()[0].opacity()
}

#[test]
fn push_merge_merges_one_gesture() {
    let mut doc = Document::new(DocumentSpec::new(4, 4)).unwrap();
    let mut h = History::default();
    opacity_gesture(&mut doc, &mut h, 7, &[0.9, 0.8, 0.7]);
    assert_eq!(h.entries().len(), 1);
    assert_eq!(h.entries().next().unwrap().name(), "Opacity");
    assert_eq!(opacity(&doc), 0.7);
    // Undo restores the state before the first edit of the gesture.
    assert!(h.undo(&mut doc).unwrap());
    assert_eq!(opacity(&doc), 1.0);
    // Redo restores the end of the gesture.
    assert!(h.redo(&mut doc).unwrap());
    assert_eq!(opacity(&doc), 0.7);
    assert!(h.undo(&mut doc).unwrap());
    assert_eq!(opacity(&doc), 1.0);
    assert!(!h.can_undo());
}

#[test]
fn push_merge_does_not_merge_after_undo() {
    let mut doc = Document::new(DocumentSpec::new(4, 4)).unwrap();
    let mut h = History::default();
    opacity_gesture(&mut doc, &mut h, 1, &[0.9]);
    let bg = doc.layers()[0].id();
    let e = doc.set_visible(bg, false).unwrap();
    h.push_merge("Visible", vec![e], 2);
    h.undo(&mut doc).unwrap();
    // The same key as the undone entry: no merge into "Opacity", and the
    // redo branch is dropped.
    opacity_gesture(&mut doc, &mut h, 2, &[0.5]);
    assert_eq!(h.entries().len(), 2);
    assert!(!h.can_redo());
    h.undo(&mut doc).unwrap();
    assert_eq!(opacity(&doc), 0.9);
    assert!(doc.layers()[0].visible());

    // Undo then redo also ends the merge.
    let mut doc = Document::new(DocumentSpec::new(4, 4)).unwrap();
    let mut h = History::default();
    opacity_gesture(&mut doc, &mut h, 3, &[0.9]);
    h.undo(&mut doc).unwrap();
    h.redo(&mut doc).unwrap();
    opacity_gesture(&mut doc, &mut h, 3, &[0.8]);
    assert_eq!(h.entries().len(), 2);
}

#[test]
fn push_merge_does_not_merge_other_keys_or_after_push() {
    let mut doc = Document::new(DocumentSpec::new(4, 4)).unwrap();
    let mut h = History::default();
    opacity_gesture(&mut doc, &mut h, 1, &[0.9, 0.8]);
    opacity_gesture(&mut doc, &mut h, 2, &[0.7, 0.6]);
    assert_eq!(h.entries().len(), 2);

    // A plain push between two calls with the same key ends the merge.
    let bg = doc.layers()[0].id();
    let e = doc.rename_layer(bg, "Base").unwrap();
    h.push("Rename", vec![e]);
    opacity_gesture(&mut doc, &mut h, 2, &[0.5]);
    assert_eq!(h.entries().len(), 4);

    // `end_merge` also ends it.
    h.end_merge();
    opacity_gesture(&mut doc, &mut h, 2, &[0.4]);
    assert_eq!(h.entries().len(), 5);

    // Undo step by step: each gesture is one step.
    for expected in [0.5, 0.6, 0.6, 0.8, 1.0] {
        h.undo(&mut doc).unwrap();
        assert_eq!(opacity(&doc), expected);
    }
    assert_eq!(doc.layers()[0].name(), "Background");
}

#[test]
fn undo_of_merged_pixel_gesture_restores_start_pixels() {
    let mut doc = Document::new(DocumentSpec::new(8, 8)).unwrap();
    let mut h = History::default();
    let start = snapshot(&doc);
    let bg = doc.layers()[0].id();
    // Overlapping writes: the undo must apply them newest first.
    for (x, v) in [(0_u32, 0.1_f32), (2, 0.2), (4, 0.3)] {
        let rect = Rect::new(x, 0, 4, 4);
        let e = doc
            .write_pixels(bg, rect, &vec![[v, v, v, 1.0]; 16])
            .unwrap();
        h.push_merge("Brush Stroke", vec![e], 99);
    }
    assert_eq!(h.entries().len(), 1);
    let end = snapshot(&doc);
    h.undo(&mut doc).unwrap();
    assert!(snapshot(&doc) == start);
    h.redo(&mut doc).unwrap();
    assert!(snapshot(&doc) == end);
}

#[test]
fn revision_changes_on_each_change() {
    let mut doc = Document::new(DocumentSpec::new(4, 4)).unwrap();
    let mut h = History::default();
    let bg = doc.layers()[0].id();
    let mut seen = vec![doc.revision()];
    let mut check = |doc: &Document| {
        assert!(!seen.contains(&doc.revision()));
        seen.push(doc.revision());
    };
    let e = doc.set_opacity(bg, 0.5).unwrap();
    check(&doc);
    h.push("Opacity", vec![e]);
    h.undo(&mut doc).unwrap();
    check(&doc);
    h.redo(&mut doc).unwrap();
    check(&doc);
    // A failed operation changes nothing.
    let before = doc.revision();
    assert!(doc.set_opacity(bg, 2.0).is_err());
    assert_eq!(doc.revision(), before);
    // A clone has the same revision; a change of the clone gives a new one.
    let mut copy = doc.clone();
    assert_eq!(copy.revision(), doc.revision());
    copy.set_visible(bg, false).unwrap();
    check(&copy);
}

#[test]
fn content_versions_track_pixels_and_masks_only() {
    let mut doc = Document::new(DocumentSpec::new(8, 8)).unwrap();
    let mut h = History::default();
    let bg = doc.layers()[0].id();
    let versions = |doc: &Document| {
        let l = doc.layer(bg).unwrap();
        (l.pixel_version(), l.mask_version())
    };
    let v0 = versions(&doc);

    // Property changes and moves keep both versions.
    let mut edits = vec![
        doc.set_opacity(bg, 0.5).unwrap(),
        doc.set_blend_mode(bg, BlendMode::Multiply).unwrap(),
        doc.set_visible(bg, false).unwrap(),
        doc.translate_layer(bg, 2, 1).unwrap(),
    ];
    assert_eq!(versions(&doc), v0);

    // A pixel write changes the pixel version only.
    edits.push(
        doc.write_pixels(bg, Rect::new(0, 0, 1, 1), &[[0.0, 0.0, 0.0, 1.0]])
            .unwrap(),
    );
    let v1 = versions(&doc);
    assert_ne!(v1.0, v0.0);
    assert_eq!(v1.1, v0.1);

    // Mask add and mask write change the mask version only.
    let mask = Mask::new(
        MaskImage::new_filled(4, 4, 1.0).unwrap(),
        (0, 0),
        MaskFill::White,
    );
    edits.push(doc.set_mask(bg, Some(mask)).unwrap());
    let v2 = versions(&doc);
    assert_eq!(v2.0, v1.0);
    assert_ne!(v2.1, v1.1);
    edits.push(doc.write_mask(bg, Rect::new(0, 0, 1, 1), &[0.0]).unwrap());
    let v3 = versions(&doc);
    assert_eq!(v3.0, v1.0);
    assert_ne!(v3.1, v2.1);
    // Mask enable is a property of the mask, not its values.
    edits.push(doc.set_mask_enabled(bg, false).unwrap());
    assert_eq!(versions(&doc), v3);

    // Undo of all changes gives new versions (the values changed again).
    h.push("All", edits);
    h.undo(&mut doc).unwrap();
    let v4 = versions(&doc);
    assert_ne!(v4.0, v1.0);
    assert_ne!(v4.1, v3.1);

    // Delete and undo of the delete keep the versions of the layer.
    let e = doc.delete_layer(bg).unwrap();
    h.push("Delete", vec![e]);
    h.undo(&mut doc).unwrap();
    assert_eq!(versions(&doc), v4);
}
