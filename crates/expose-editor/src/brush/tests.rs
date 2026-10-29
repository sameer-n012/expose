//! Tests of the brush engine.

// Tests compare exact values, and make test values from small integers.
#![allow(clippy::unwrap_used, clippy::float_cmp, clippy::cast_precision_loss)]

use expose_core::buffer::WorkingImage;

use super::*;
use crate::document::{Background, DocumentSpec};
use crate::history::History;
use crate::layer::{GroupIsolation, LayerContent, Mask, MaskFill, PixelLayer, Placement};
use crate::selection::{SelectionMode, SelectionShape};

const RED: StrokeTarget = StrokeTarget::Paint {
    color: [1.0, 0.0, 0.0],
};

fn doc_with_layer(
    canvas: u32,
    size: (u32, u32),
    fill: Rgba,
    offset: (i32, i32),
) -> (Document, LayerId) {
    let mut spec = DocumentSpec::new(canvas, canvas);
    spec.background = Background::Transparent;
    let mut doc = Document::new(spec).unwrap();
    let image = WorkingImage::new_filled(size.0, size.1, fill).unwrap();
    let content = LayerContent::Pixel(PixelLayer::new(image, offset).unwrap());
    let (id, _) = doc.add_layer("Paint", content, Placement::root(1)).unwrap();
    (doc, id)
}

fn hard(size: f32) -> BrushSettings {
    BrushSettings {
        size,
        ..BrushSettings::default()
    }
}

fn pt(x: f64, y: f64) -> StrokePoint {
    StrokePoint::new(x, y, 1.0)
}

fn px(doc: &Document, id: LayerId, x: u32, y: u32) -> Rgba {
    doc.layer(id)
        .unwrap()
        .as_pixel()
        .unwrap()
        .image()
        .pixel(x, y)
        .unwrap()
}

fn mask_at(doc: &Document, id: LayerId, x: u32, y: u32) -> f32 {
    doc.layer(id)
        .unwrap()
        .mask()
        .unwrap()
        .image
        .get(x, y)
        .unwrap()
}

fn bits(doc: &Document, id: LayerId) -> Vec<u32> {
    let p = doc.layer(id).unwrap().as_pixel().unwrap();
    p.image()
        .pixels()
        .iter()
        .flatten()
        .map(|v| v.to_bits())
        .collect()
}

/// Runs one stroke with all points in one batch and ends it.
fn stroke(
    doc: &mut Document,
    id: LayerId,
    target: StrokeTarget,
    settings: BrushSettings,
    points: &[StrokePoint],
) -> Option<Edit> {
    let mut s = Stroke::begin(doc, id, target, settings).unwrap();
    s.add_points(doc, points).unwrap();
    s.end(doc).unwrap()
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn hard_dab_center_and_edge_values() {
    let (mut doc, id) = doc_with_layer(64, (64, 64), [0.0; 4], (0, 0));
    stroke(&mut doc, id, RED, hard(20.0), &[pt(32.5, 32.5)]);
    // Center and inside: full color.
    assert_eq!(px(&doc, id, 32, 32), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(px(&doc, id, 41, 32), [1.0, 0.0, 0.0, 1.0]);
    // Distance = radius: half coverage (1 px anti-alias edge).
    assert_eq!(px(&doc, id, 42, 32), [0.5, 0.0, 0.0, 0.5]);
    // Distance = radius + 1: no change.
    assert_eq!(px(&doc, id, 43, 32), [0.0; 4]);
}

#[test]
fn soft_dab_falloff_values() {
    let (mut doc, id) = doc_with_layer(64, (64, 64), [0.0; 4], (0, 0));
    let settings = BrushSettings {
        hardness: 0.5,
        ..hard(20.0)
    };
    stroke(&mut doc, id, RED, settings, &[pt(32.5, 32.5)]);
    // d = 0 and d = hardness * radius: full.
    assert_eq!(px(&doc, id, 32, 32)[3], 1.0);
    assert_eq!(px(&doc, id, 37, 32)[3], 1.0);
    // d = 8: t = 0.6, 1 - 0.36 * 1.8 = 0.352.
    assert!(near(px(&doc, id, 40, 32)[3], 0.352));
    // d = 6: t = 0.2, 1 - 0.04 * 2.6 = 0.896.
    assert!(near(px(&doc, id, 38, 32)[3], 0.896));
    // d = radius: 0.
    assert_eq!(px(&doc, id, 42, 32)[3], 0.0);
}

#[test]
fn batch_split_gives_same_pixels() {
    for blend_mode in [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Luminosity,
    ] {
        batch_split_gives_same_pixels_with(blend_mode);
    }
}

fn batch_split_gives_same_pixels_with(blend_mode: BlendMode) {
    let settings = BrushSettings {
        size: 9.0,
        hardness: 0.3,
        flow: 0.3,
        opacity: 0.8,
        spacing: 0.1,
        pressure_size: true,
        pressure_opacity: true,
        blend_mode,
    };
    let points: Vec<StrokePoint> = (0..40)
        .map(|i| {
            let t = f64::from(i);
            StrokePoint::new(
                5.0 + t * 1.37,
                20.0 + (t * 0.4).sin() * 12.0,
                0.2 + 0.02 * i as f32,
            )
        })
        .collect();
    let run = |split: usize| {
        let (mut doc, id) = doc_with_layer(64, (64, 64), [0.1, 0.2, 0.3, 0.5], (0, 0));
        let mut s = Stroke::begin(&doc, id, RED, settings).unwrap();
        for chunk in points.chunks(split) {
            s.add_points(&mut doc, chunk).unwrap();
        }
        s.end(&mut doc).unwrap().unwrap();
        bits(&doc, id)
    };
    let one = run(points.len());
    assert_eq!(one, run(1));
    assert_eq!(one, run(3));
    assert_eq!(one, run(7));
}

#[test]
fn opacity_caps_overlapping_dabs() {
    let (mut doc, id) = doc_with_layer(64, (64, 64), [0.0; 4], (0, 0));
    let settings = BrushSettings {
        opacity: 0.5,
        spacing: 0.05,
        ..hard(20.0)
    };
    // Many dabs that overlap at (32, 32).
    stroke(
        &mut doc,
        id,
        RED,
        settings,
        &[pt(28.5, 32.5), pt(36.5, 32.5), pt(28.5, 32.5)],
    );
    assert_eq!(px(&doc, id, 32, 32), [0.5, 0.0, 0.0, 0.5]);
}

#[test]
fn flow_accumulates() {
    let (mut doc, id) = doc_with_layer(64, (64, 64), [0.0; 4], (0, 0));
    let settings = BrushSettings {
        flow: 0.5,
        // Step = 0.05 * 20 = 1 px (a little more in f32): dabs at x = 32.5
        // and 33.5.
        spacing: 0.05,
        ..hard(20.0)
    };
    stroke(
        &mut doc,
        id,
        RED,
        settings,
        &[pt(32.5, 32.5), pt(34.0, 32.5)],
    );
    // Both dabs cover (33, 32) fully: 0.5, then 0.5 + 0.5 * 0.5.
    assert_eq!(px(&doc, id, 33, 32)[3], 0.75);
    // (10, 32) is at d = 22.5 / 23.5: no dab reaches it.
    assert_eq!(px(&doc, id, 10, 32)[3], 0.0);
}

#[test]
fn selection_multiplies_dab() {
    let (mut doc, id) = doc_with_layer(64, (64, 64), [0.0; 4], (0, 0));
    let shape = SelectionShape::Rect {
        x: 0.0,
        y: 0.0,
        width: 32.0,
        height: 64.0,
    };
    doc.select_shape(&shape, SelectionMode::Replace, false, 0.0)
        .unwrap();
    stroke(&mut doc, id, RED, hard(20.0), &[pt(32.0, 32.0)]);
    assert_eq!(px(&doc, id, 31, 32)[3], 1.0);
    assert_eq!(px(&doc, id, 32, 32)[3], 0.0);
}

#[test]
fn erase_removes_coverage() {
    let (mut doc, id) = doc_with_layer(32, (32, 32), [1.0; 4], (0, 0));
    let settings = BrushSettings {
        opacity: 0.25,
        ..hard(10.0)
    };
    stroke(
        &mut doc,
        id,
        StrokeTarget::Erase,
        settings,
        &[pt(16.5, 16.5)],
    );
    assert_eq!(px(&doc, id, 16, 16), [0.75; 4]);
    assert_eq!(px(&doc, id, 0, 0), [1.0; 4]);
}

#[test]
fn transparency_lock_keeps_alpha() {
    let (mut doc, id) = doc_with_layer(32, (32, 32), [0.25, 0.25, 0.25, 0.5], (0, 0));
    // A transparent pixel inside the dab.
    let edit = doc
        .write_pixels(id, Rect::new(17, 16, 1, 1), &[[0.0; 4]])
        .unwrap();
    drop(edit);
    let locks = Locks {
        transparency: true,
        ..Locks::NONE
    };
    doc.set_locks(id, locks).unwrap();
    stroke(&mut doc, id, RED, hard(10.0), &[pt(16.5, 16.5)]);
    assert_eq!(px(&doc, id, 16, 16), [0.5, 0.0, 0.0, 0.5]);
    assert_eq!(px(&doc, id, 17, 16), [0.0; 4]);
    // Erase with the transparency lock is an error.
    let err = Stroke::begin(&doc, id, StrokeTarget::Erase, hard(10.0)).unwrap_err();
    assert_eq!(err, EditorError::TransparencyLocked(id));
}

#[test]
fn mask_stroke_paints_mask_values() {
    let (mut doc, id) = doc_with_layer(32, (32, 32), [1.0; 4], (0, 0));
    let mask = Mask::new(
        MaskImage::new_filled(32, 32, 1.0).unwrap(),
        (0, 0),
        MaskFill::White,
    );
    doc.set_mask(id, Some(mask)).unwrap();
    let before = doc.layer(id).unwrap().mask_version();
    let pixels = doc.layer(id).unwrap().pixel_version();
    let settings = BrushSettings {
        opacity: 0.75,
        ..hard(10.0)
    };
    let target = StrokeTarget::Mask { value: 0.0 };
    stroke(&mut doc, id, target, settings, &[pt(16.5, 16.5)]);
    assert_eq!(mask_at(&doc, id, 16, 16), 0.25);
    assert_eq!(mask_at(&doc, id, 0, 0), 1.0);
    let layer = doc.layer(id).unwrap();
    assert_ne!(layer.mask_version(), before);
    assert_eq!(layer.pixel_version(), pixels);
    // The pixel values do not change.
    assert_eq!(px(&doc, id, 16, 16), [1.0; 4]);
}

#[test]
fn layer_offset_maps_canvas_to_layer() {
    let (mut doc, id) = doc_with_layer(64, (32, 32), [0.0; 4], (10, 20));
    let mut s = Stroke::begin(&doc, id, RED, hard(20.0)).unwrap();
    let rect = s.add_points(&mut doc, &[pt(20.5, 30.5)]).unwrap();
    // Changed pixels: canvas x in 10..=30 (clipped at the layer edge x =
    // 10), y in 20..=40 (clipped at the layer edge y = 20).
    assert_eq!(rect, Some(Rect::new(10, 20, 21, 21)));
    s.end(&mut doc).unwrap();
    assert_eq!(px(&doc, id, 10, 10), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(px(&doc, id, 20, 10), [0.5, 0.0, 0.0, 0.5]);
}

#[test]
fn changed_rect_is_clipped_to_canvas() {
    let (mut doc, id) = doc_with_layer(32, (64, 64), [0.0; 4], (-16, -16));
    let mut s = Stroke::begin(&doc, id, RED, hard(10.0)).unwrap();
    // A dab fully outside the canvas changes the layer, not the canvas.
    assert_eq!(s.add_points(&mut doc, &[pt(-8.5, -8.5)]).unwrap(), None);
    assert!(s.end(&mut doc).unwrap().is_some());
}

#[test]
fn undo_restores_exactly_and_redo_repeats() {
    let (mut doc, id) = doc_with_layer(200, (200, 200), [0.0; 4], (0, 0));
    // A gradient, so that each pixel has its own value.
    let data: Vec<Rgba> = (0..200 * 200)
        .map(|i| {
            let v = (i % 997) as f32 / 1000.0;
            [v * 0.3, v * 0.5, v * 0.7, v]
        })
        .collect();
    doc.write_pixels(id, Rect::new(0, 0, 200, 200), &data)
        .unwrap();
    let before = bits(&doc, id);
    let settings = BrushSettings {
        hardness: 0.2,
        flow: 0.4,
        ..hard(30.0)
    };
    let edit = stroke(
        &mut doc,
        id,
        RED,
        settings,
        &[pt(10.0, 10.0), pt(150.0, 40.0), pt(60.0, 190.0)],
    )
    .unwrap();
    let after = bits(&doc, id);
    assert_ne!(before, after);
    // The edit holds the changed tiles only (not the full 4 x 4 grid).
    let full = 200 * 200 * size_of::<Rgba>();
    assert!(edit.memory_bytes() < full);
    let mut history = History::default();
    history.push("Brush", vec![edit]);
    let v_after = doc.layer(id).unwrap().pixel_version();
    history.undo(&mut doc).unwrap();
    assert_eq!(bits(&doc, id), before);
    assert_ne!(doc.layer(id).unwrap().pixel_version(), v_after);
    history.redo(&mut doc).unwrap();
    assert_eq!(bits(&doc, id), after);
}

#[test]
fn mask_stroke_undo_restores() {
    let (mut doc, id) = doc_with_layer(32, (32, 32), [1.0; 4], (0, 0));
    let mask = Mask::new(
        MaskImage::new_filled(32, 32, 0.5).unwrap(),
        (0, 0),
        MaskFill::White,
    );
    doc.set_mask(id, Some(mask)).unwrap();
    let edit = stroke(
        &mut doc,
        id,
        StrokeTarget::Mask { value: 1.0 },
        hard(8.0),
        &[pt(4.0, 4.0), pt(28.0, 28.0)],
    )
    .unwrap();
    assert_eq!(mask_at(&doc, id, 16, 16), 1.0);
    let mut history = History::default();
    history.push("Brush", vec![edit]);
    history.undo(&mut doc).unwrap();
    assert_eq!(mask_at(&doc, id, 16, 16), 0.5);
    history.redo(&mut doc).unwrap();
    assert_eq!(mask_at(&doc, id, 16, 16), 1.0);
}

#[test]
fn cancel_restores_original() {
    let (mut doc, id) = doc_with_layer(64, (64, 64), [0.1, 0.2, 0.3, 0.4], (0, 0));
    let before = bits(&doc, id);
    let mut s = Stroke::begin(&doc, id, RED, hard(20.0)).unwrap();
    s.add_points(&mut doc, &[pt(5.0, 5.0), pt(60.0, 60.0)])
        .unwrap();
    let v_stroke = doc.layer(id).unwrap().pixel_version();
    assert_ne!(bits(&doc, id), before);
    s.cancel(&mut doc).unwrap();
    assert_eq!(bits(&doc, id), before);
    assert_ne!(doc.layer(id).unwrap().pixel_version(), v_stroke);
}

#[test]
fn each_changing_batch_gives_new_version() {
    let (mut doc, id) = doc_with_layer(64, (64, 64), [0.0; 4], (0, 0));
    let mut s = Stroke::begin(&doc, id, RED, hard(10.0)).unwrap();
    let v0 = doc.layer(id).unwrap().pixel_version();
    let r0 = doc.revision();
    s.add_points(&mut doc, &[pt(10.0, 10.0)]).unwrap();
    let v1 = doc.layer(id).unwrap().pixel_version();
    assert_ne!(v0, v1);
    assert_ne!(doc.revision(), r0);
    s.add_points(&mut doc, &[pt(30.0, 10.0)]).unwrap();
    let v2 = doc.layer(id).unwrap().pixel_version();
    assert_ne!(v1, v2);
    // No new dab (the distance is below the step): no new version.
    assert_eq!(s.add_points(&mut doc, &[pt(30.5, 10.0)]).unwrap(), None);
    assert_eq!(doc.layer(id).unwrap().pixel_version(), v2);
    s.end(&mut doc).unwrap();
}

#[test]
fn locks_and_layer_kind_errors() {
    let (mut doc, id) = doc_with_layer(32, (32, 32), [0.0; 4], (0, 0));
    let (group, _) = doc
        .add_layer(
            "Group",
            LayerContent::Group(GroupIsolation::PassThrough),
            Placement::root(2),
        )
        .unwrap();
    assert_eq!(
        Stroke::begin(&doc, group, RED, hard(5.0)).unwrap_err(),
        EditorError::NotAPixelLayer(group)
    );
    let mask = StrokeTarget::Mask { value: 1.0 };
    assert_eq!(
        Stroke::begin(&doc, id, mask, hard(5.0)).unwrap_err(),
        EditorError::NoMask(id)
    );
    let missing = LayerId(9999);
    assert_eq!(
        Stroke::begin(&doc, missing, RED, hard(5.0)).unwrap_err(),
        EditorError::LayerNotFound(missing)
    );
    let locks = Locks {
        pixels: true,
        ..Locks::NONE
    };
    doc.set_locks(id, locks).unwrap();
    assert_eq!(
        Stroke::begin(&doc, id, RED, hard(5.0)).unwrap_err(),
        EditorError::Locked {
            id,
            lock: LockKind::Pixels
        }
    );
}

#[test]
fn bad_settings_and_points_are_errors() {
    let (mut doc, id) = doc_with_layer(32, (32, 32), [0.0; 4], (0, 0));
    for bad in [
        hard(0.5),
        hard(6000.0),
        BrushSettings {
            spacing: 0.0,
            ..hard(5.0)
        },
        BrushSettings {
            flow: f32::NAN,
            ..hard(5.0)
        },
    ] {
        assert!(Stroke::begin(&doc, id, RED, bad).is_err());
    }
    let mut s = Stroke::begin(&doc, id, RED, hard(5.0)).unwrap();
    let before = bits(&doc, id);
    // The first point is good, the second point is bad: no change.
    let points = [pt(5.0, 5.0), StrokePoint::new(6.0, 6.0, 1.5)];
    assert!(s.add_points(&mut doc, &points).is_err());
    assert!(s.add_points(&mut doc, &[pt(f64::NAN, 1.0)]).is_err());
    assert_eq!(bits(&doc, id), before);
    assert_eq!(s.end(&mut doc).unwrap(), None);
}

#[test]
fn nothing_changed_gives_none() {
    let (mut doc, id) = doc_with_layer(32, (32, 32), [0.0; 4], (0, 0));
    // Outside the layer.
    let mut s = Stroke::begin(&doc, id, RED, hard(5.0)).unwrap();
    assert_eq!(s.add_points(&mut doc, &[pt(-50.0, -50.0)]).unwrap(), None);
    assert_eq!(s.end(&mut doc).unwrap(), None);
    // Flow 0.
    let zero = BrushSettings {
        flow: 0.0,
        ..hard(5.0)
    };
    assert_eq!(stroke(&mut doc, id, RED, zero, &[pt(10.0, 10.0)]), None);
    // Erase on transparent pixels: coverage changes, values do not.
    assert_eq!(
        stroke(
            &mut doc,
            id,
            StrokeTarget::Erase,
            hard(5.0),
            &[pt(10.0, 10.0)]
        ),
        None
    );
}

#[test]
fn layer_change_during_stroke_is_conflict() {
    let (mut doc, id) = doc_with_layer(32, (32, 32), [0.0; 4], (0, 0));
    let mut s = Stroke::begin(&doc, id, RED, hard(5.0)).unwrap();
    s.add_points(&mut doc, &[pt(10.0, 10.0)]).unwrap();
    doc.write_pixels(id, Rect::new(0, 0, 1, 1), &[[1.0; 4]])
        .unwrap();
    assert_eq!(
        s.add_points(&mut doc, &[pt(20.0, 10.0)]).unwrap_err(),
        EditorError::StrokeConflict(id)
    );
    let mut s = Stroke::begin(&doc, id, RED, hard(5.0)).unwrap();
    doc.translate_layer(id, 1, 0).unwrap();
    assert_eq!(
        s.add_points(&mut doc, &[pt(20.0, 10.0)]).unwrap_err(),
        EditorError::StrokeConflict(id)
    );
}

/// Timing of a large stroke. Run with
/// `cargo test -p expose-editor --release -- --ignored brush_timing --nocapture`.
#[test]
#[ignore = "timing; run in release"]
fn brush_timing_4096() {
    use std::time::Instant;
    let (mut doc, id) = doc_with_layer(4096, (4096, 4096), [0.0; 4], (0, 0));
    let settings = BrushSettings {
        hardness: 0.5,
        flow: 0.5,
        spacing: 0.1,
        ..hard(300.0)
    };
    let mut s = Stroke::begin(&doc, id, RED, settings).unwrap();
    // Step = 30 px. Each batch moves 16 * 30 px: 16 dabs.
    let mut x = 200.0;
    s.add_points(&mut doc, &[pt(x, 2048.0)]).unwrap();
    let mut times = Vec::new();
    for _ in 0..7 {
        let next = x + 16.0 * 30.0;
        let t = Instant::now();
        s.add_points(&mut doc, &[pt(next, 2048.0)]).unwrap();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        x = next;
    }
    let t = Instant::now();
    let edit = s.end(&mut doc).unwrap().unwrap();
    let end_ms = t.elapsed().as_secs_f64() * 1000.0;
    println!(
        "brush 300 px, 16 dabs per batch: {times:.2?} ms; end {end_ms:.2} ms; edit {} MiB",
        edit.memory_bytes() / (1 << 20)
    );
}

// ---------------------------------------------------------------------
// Blend modes.
// ---------------------------------------------------------------------

fn with_mode(blend_mode: BlendMode, size: f32) -> BrushSettings {
    BrushSettings {
        blend_mode,
        ..hard(size)
    }
}

/// A soft, partly opaque stroke, so that the coverage has many values.
fn soft_points() -> (BrushSettings, Vec<StrokePoint>) {
    let settings = BrushSettings {
        hardness: 0.2,
        opacity: 0.7,
        flow: 0.4,
        spacing: 0.1,
        ..hard(12.0)
    };
    let points = (0..20)
        .map(|i| pt(4.0 + f64::from(i) * 1.3, 16.0 + f64::from(i % 5)))
        .collect();
    (settings, points)
}

fn close3(a: [f32; 3], b: [f32; 3], eps: f32) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= eps)
}

#[test]
fn normal_mode_keeps_the_old_formula_bit_for_bit() {
    let (settings, points) = soft_points();
    let color = [0.9, 0.3, 0.05];
    let target = StrokeTarget::Paint { color };
    // On a transparent layer the result alpha is the coverage `c`.
    let (mut clear, cid) = doc_with_layer(32, (32, 32), [0.0; 4], (0, 0));
    stroke(&mut clear, cid, target, settings, &points);
    let o: Rgba = [0.1, 0.2, 0.3, 0.5];
    let (mut doc, id) = doc_with_layer(32, (32, 32), o, (0, 0));
    stroke(&mut doc, id, target, settings, &points);
    let mut partial = 0;
    for y in 0..32 {
        for x in 0..32 {
            let c = px(&clear, cid, x, y)[3];
            if c > 0.0 && c < 1.0 {
                partial += 1;
            }
            let k = 1.0 - c;
            let expected = [
                color[0] * c + o[0] * k,
                color[1] * c + o[1] * k,
                color[2] * c + o[2] * k,
                c + o[3] * k,
            ];
            let got = px(&doc, id, x, y);
            if c == 0.0 {
                assert_eq!(got, o);
            } else {
                assert_eq!(got.map(f32::to_bits), expected.map(f32::to_bits));
            }
        }
    }
    assert!(partial > 20);
}

#[test]
fn multiply_with_white_and_screen_with_black_change_nothing() {
    let (settings, points) = soft_points();
    let o: Rgba = [0.2, 0.4, 0.6, 1.0];
    for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
        for (mode, color) in [
            (BlendMode::Multiply, [1.0; 3]),
            (BlendMode::Screen, [0.0; 3]),
        ] {
            let (mut doc, id) = doc_with_layer(32, (32, 32), o, (0, 0));
            doc.set_blend_space(space).unwrap();
            let s = BrushSettings {
                blend_mode: mode,
                ..settings
            };
            stroke(&mut doc, id, StrokeTarget::Paint { color }, s, &points);
            for y in 0..32 {
                for x in 0..32 {
                    let p = px(&doc, id, x, y);
                    assert!(
                        close3([p[0], p[1], p[2]], [o[0], o[1], o[2]], 1e-5),
                        "{mode:?} {space:?} at {x},{y}: {p:?}"
                    );
                    assert_eq!(p[3], 1.0);
                }
            }
        }
    }
}

#[test]
fn blend_mode_on_transparent_pixels_equals_normal() {
    let (settings, points) = soft_points();
    let target = StrokeTarget::Paint {
        color: [0.7, 0.2, 0.9],
    };
    let run = |mode: BlendMode| {
        let (mut doc, id) = doc_with_layer(32, (32, 32), [0.0; 4], (0, 0));
        let s = BrushSettings {
            blend_mode: mode,
            ..settings
        };
        stroke(&mut doc, id, target, s, &points);
        bits(&doc, id)
    };
    let normal = run(BlendMode::Normal);
    assert_eq!(run(BlendMode::Multiply), normal);
    assert_eq!(run(BlendMode::Luminosity), normal);
}

#[test]
fn luminosity_on_an_opaque_pixel() {
    let o: Rgba = [0.6, 0.2, 0.1, 1.0];
    let color = [0.1, 0.1, 0.8];
    let (mut doc, id) = doc_with_layer(64, (64, 64), o, (0, 0));
    let space = doc.blend_space();
    stroke(
        &mut doc,
        id,
        StrokeTarget::Paint { color },
        with_mode(BlendMode::Luminosity, 20.0),
        &[pt(32.5, 32.5)],
    );
    // Center: coverage 1, so the result is `B` (ab = 1).
    let expected = space
        .decode(BlendMode::Luminosity.blend(space.encode([o[0], o[1], o[2]]), space.encode(color)));
    let p = px(&doc, id, 32, 32);
    assert!(close3([p[0], p[1], p[2]], expected, 1e-6), "{p:?}");
    assert_eq!(p[3], 1.0);
    // The result is not the brush color and not the original.
    assert!(!close3([p[0], p[1], p[2]], color, 1e-2));
    assert!(!close3([p[0], p[1], p[2]], [o[0], o[1], o[2]], 1e-2));
    // The encoded luminance is the luminance of the encoded brush color.
    let lum = |c: [f32; 3]| expose_core::color::luminance(space.encode(c));
    assert!((lum([p[0], p[1], p[2]]) - lum(color)).abs() < 1e-5);
    // Outside the dab: no change.
    assert_eq!(px(&doc, id, 2, 2), o);
}

#[test]
fn blend_mode_with_transparency_lock_keeps_alpha() {
    let o: Rgba = [0.25, 0.25, 0.25, 0.5];
    let (mut doc, id) = doc_with_layer(32, (32, 32), o, (0, 0));
    let locks = Locks {
        transparency: true,
        ..Locks::NONE
    };
    doc.set_locks(id, locks).unwrap();
    let color = [1.0, 0.0, 0.0];
    stroke(
        &mut doc,
        id,
        StrokeTarget::Paint { color },
        with_mode(BlendMode::Multiply, 10.0),
        &[pt(16.5, 16.5)],
    );
    // c = 1: rgb = Cs' * a, Cs' = 0.5 * red + 0.5 * multiply(gray, red).
    let space = doc.blend_space();
    let b = space.decode(BlendMode::Multiply.blend(space.encode([0.5; 3]), space.encode(color)));
    let cs = [0.5 + 0.5 * b[0], 0.5 * b[1], 0.5 * b[2]];
    let p = px(&doc, id, 16, 16);
    assert!(
        close3([p[0], p[1], p[2]], cs.map(|v| v * 0.5), 1e-6),
        "{p:?}"
    );
    assert_eq!(p[3], 0.5);
}

#[test]
fn blend_mode_errors() {
    let (mut doc, id) = doc_with_layer(32, (32, 32), [1.0; 4], (0, 0));
    let mask = Mask::new(
        MaskImage::new_filled(32, 32, 1.0).unwrap(),
        (0, 0),
        MaskFill::White,
    );
    doc.set_mask(id, Some(mask)).unwrap();
    let multiply = with_mode(BlendMode::Multiply, 5.0);
    for target in [StrokeTarget::Erase, StrokeTarget::Mask { value: 0.0 }] {
        assert!(matches!(
            Stroke::begin(&doc, id, target, multiply).unwrap_err(),
            EditorError::InvalidBrush(_)
        ));
        // Normal is good for both targets.
        Stroke::begin(&doc, id, target, hard(5.0))
            .unwrap()
            .cancel(&mut doc)
            .unwrap();
    }
    let dissolve = with_mode(BlendMode::Dissolve, 5.0);
    assert!(matches!(
        dissolve.validate(),
        Err(EditorError::InvalidBrush(_))
    ));
    assert!(matches!(
        Stroke::begin(&doc, id, RED, dissolve).unwrap_err(),
        EditorError::InvalidBrush(_)
    ));
    // All other modes are good for paint.
    for mode in BlendMode::ALL {
        if mode != BlendMode::Dissolve {
            Stroke::begin(&doc, id, RED, with_mode(mode, 5.0))
                .unwrap()
                .cancel(&mut doc)
                .unwrap();
        }
    }
}
