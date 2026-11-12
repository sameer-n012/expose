//! Tests of the clone and heal strokes.

// Tests compare exact values, and make test values from small integers.
#![allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use expose_core::buffer::WorkingImage;

use super::*;
use crate::document::{Background, DocumentSpec};
use crate::history::History;
use crate::layer::{GroupIsolation, LayerContent, PixelLayer, Placement};
use crate::selection::{SelectionMode, SelectionShape};

/// A document with one canvas-sized pixel layer with the values `f(x, y)`.
fn doc_from(
    w: u32,
    h: u32,
    space: BlendSpace,
    f: impl Fn(u32, u32) -> Rgba,
) -> (Document, LayerId) {
    let mut spec = DocumentSpec::new(w, h);
    spec.background = Background::Transparent;
    let mut doc = Document::new(spec).unwrap();
    doc.blend_space = space;
    let pixels = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| f(x, y))
        .collect();
    let image = WorkingImage::from_pixels(w, h, pixels).unwrap();
    let content = LayerContent::Pixel(PixelLayer::new(image, (0, 0)).unwrap());
    let (id, _) = doc.add_layer("Paint", content, Placement::root(1)).unwrap();
    (doc, id)
}

fn image(doc: &Document, id: LayerId) -> WorkingImage {
    doc.layer(id).unwrap().as_pixel().unwrap().image().clone()
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

fn bits(doc: &Document, id: LayerId) -> Vec<u32> {
    image(doc, id)
        .pixels()
        .iter()
        .flatten()
        .map(|v| v.to_bits())
        .collect()
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

fn spec(kind: CloneKind, offset: (i32, i32)) -> CloneSpec {
    CloneSpec {
        kind,
        offset,
        source: CloneSource::Layer,
    }
}

fn clone_stroke(
    doc: &mut Document,
    id: LayerId,
    spec: CloneSpec,
    settings: BrushSettings,
    points: &[StrokePoint],
) -> Option<Edit> {
    let mut s = Stroke::begin_clone(doc, id, spec, settings).unwrap();
    s.add_points(doc, points).unwrap();
    s.end(doc).unwrap()
}

/// A gray texture: a value from the position.
fn texture(x: u32, y: u32) -> Rgba {
    let v = ((x * 7 + y * 13) % 11) as f32 / 20.0;
    [v, 0.5 * v, 0.25, 1.0]
}

#[test]
fn clone_copies_source_pixels() {
    let (mut doc, id) = doc_from(64, 64, BlendSpace::Perceptual, texture);
    let edit = clone_stroke(
        &mut doc,
        id,
        spec(CloneKind::Clone, (-20, 3)),
        hard(10.0),
        &[pt(40.5, 30.5)],
    );
    assert!(edit.is_some());
    for (x, y) in [(40, 30), (38, 31), (42, 28)] {
        assert_eq!(px(&doc, id, x, y), texture(x - 20, y + 3));
    }
    assert_eq!(px(&doc, id, 10, 10), texture(10, 10));
}

#[test]
fn stroke_across_own_source_reads_pre_stroke_layer() {
    let settings = BrushSettings {
        hardness: 0.5,
        flow: 0.5,
        spacing: 0.1,
        ..hard(16.0)
    };
    let points = [pt(20.0, 32.0), pt(110.0, 34.0), pt(40.0, 40.0)];
    for kind in [CloneKind::Clone, CloneKind::Heal] {
        let (mut a, ida) = doc_from(128, 64, BlendSpace::Perceptual, texture);
        let (mut b, idb) = doc_from(128, 64, BlendSpace::Perceptual, texture);
        // The source of `b` is a copy of the layer before the stroke.
        let copy = CloneSource::Image(Arc::new(image(&b, idb)));
        clone_stroke(&mut a, ida, spec(kind, (-6, 1)), settings, &points);
        let sb = CloneSpec {
            kind,
            offset: (-6, 1),
            source: copy,
        };
        clone_stroke(&mut b, idb, sb, settings, &points);
        assert_eq!(bits(&a, ida), bits(&b, idb), "{kind:?}");
        assert_ne!(px(&a, ida, 60, 33), texture(60, 33));
    }
}

#[test]
fn source_outside_does_not_change_pixels() {
    let (mut doc, id) = doc_from(64, 64, BlendSpace::Perceptual, texture);
    let before = bits(&doc, id);
    // All source positions are left of the layer.
    let edit = clone_stroke(
        &mut doc,
        id,
        spec(CloneKind::Clone, (-40, 0)),
        hard(10.0),
        &[pt(20.0, 20.0)],
    );
    assert!(edit.is_none());
    assert_eq!(bits(&doc, id), before);
    // Half of the dab has a source: only that half changes.
    clone_stroke(
        &mut doc,
        id,
        spec(CloneKind::Heal, (-30, 0)),
        hard(10.0),
        &[pt(30.0, 20.0)],
    );
    assert_eq!(px(&doc, id, 28, 20), texture(28, 20));
    assert_ne!(px(&doc, id, 32, 20), texture(32, 20));
    // An image source: positions outside the canvas do not change.
    let (mut doc, id) = doc_from(64, 64, BlendSpace::Perceptual, texture);
    let before = bits(&doc, id);
    let img = Arc::new(WorkingImage::new_filled(64, 64, [1.0; 4]).unwrap());
    let s = CloneSpec {
        kind: CloneKind::Clone,
        offset: (0, 50),
        source: CloneSource::Image(img),
    };
    assert!(clone_stroke(&mut doc, id, s, hard(10.0), &[pt(20.0, 30.0)]).is_none());
    assert_eq!(bits(&doc, id), before);
}

#[test]
fn begin_clone_errors() {
    let (mut doc, id) = doc_from(32, 32, BlendSpace::Perceptual, texture);
    let blend = BrushSettings {
        blend_mode: BlendMode::Multiply,
        ..hard(5.0)
    };
    assert!(matches!(
        Stroke::begin_clone(&doc, id, spec(CloneKind::Clone, (1, 0)), blend).unwrap_err(),
        EditorError::InvalidBrush(_)
    ));
    for offset in [(MAX_CLONE_OFFSET + 1, 0), (0, i32::MIN)] {
        assert!(matches!(
            Stroke::begin_clone(&doc, id, spec(CloneKind::Heal, offset), hard(5.0)).unwrap_err(),
            EditorError::OutOfRange { .. }
        ));
    }
    assert!(
        Stroke::begin_clone(
            &doc,
            id,
            spec(CloneKind::Clone, (MAX_CLONE_OFFSET, -MAX_CLONE_OFFSET)),
            hard(5.0)
        )
        .is_ok()
    );
    let small = CloneSpec {
        kind: CloneKind::Clone,
        offset: (0, 0),
        source: CloneSource::Image(Arc::new(WorkingImage::new(16, 32).unwrap())),
    };
    assert!(matches!(
        Stroke::begin_clone(&doc, id, small, hard(5.0)).unwrap_err(),
        EditorError::InvalidBrush(_)
    ));
    let (group, _) = doc
        .add_layer(
            "Group",
            LayerContent::Group(GroupIsolation::PassThrough),
            Placement::root(2),
        )
        .unwrap();
    assert_eq!(
        Stroke::begin_clone(&doc, group, spec(CloneKind::Clone, (1, 0)), hard(5.0)).unwrap_err(),
        EditorError::NotAPixelLayer(group)
    );
    let missing = LayerId(9999);
    assert_eq!(
        Stroke::begin_clone(&doc, missing, spec(CloneKind::Clone, (1, 0)), hard(5.0)).unwrap_err(),
        EditorError::LayerNotFound(missing)
    );
    let locks = Locks {
        pixels: true,
        ..Locks::NONE
    };
    doc.set_locks(id, locks).unwrap();
    assert_eq!(
        Stroke::begin_clone(&doc, id, spec(CloneKind::Heal, (1, 0)), hard(5.0)).unwrap_err(),
        EditorError::Locked {
            id,
            lock: LockKind::Pixels
        }
    );
}

/// The left half is opaque red; the right half is gray with alpha 0.5,
/// and one pixel at (49, 32) is transparent.
fn half_red(x: u32, y: u32) -> Rgba {
    if x < 32 {
        [1.0, 0.0, 0.0, 1.0]
    } else if (x, y) == (49, 32) {
        [0.0; 4]
    } else {
        [0.25, 0.25, 0.25, 0.5]
    }
}

#[test]
fn transparency_lock_keeps_alpha() {
    for kind in [CloneKind::Clone, CloneKind::Heal] {
        let (mut doc, id) = doc_from(64, 64, BlendSpace::Perceptual, half_red);
        let locks = Locks {
            transparency: true,
            ..Locks::NONE
        };
        doc.set_locks(id, locks).unwrap();
        clone_stroke(
            &mut doc,
            id,
            spec(kind, (-32, 0)),
            hard(12.0),
            &[pt(48.5, 32.5)],
        );
        assert_eq!(px(&doc, id, 49, 32), [0.0; 4], "{kind:?}");
        let p = px(&doc, id, 48, 32);
        assert_eq!(p[3], 0.5, "{kind:?}");
        if kind == CloneKind::Clone {
            assert_eq!(p, [0.5, 0.0, 0.0, 0.5]);
        }
    }
}

#[test]
fn cancel_restores_original() {
    for kind in [CloneKind::Clone, CloneKind::Heal] {
        let (mut doc, id) = doc_from(64, 64, BlendSpace::Perceptual, texture);
        let before = bits(&doc, id);
        let mut s = Stroke::begin_clone(&doc, id, spec(kind, (5, 5)), hard(12.0)).unwrap();
        s.add_points(&mut doc, &[pt(10.0, 10.0), pt(50.0, 40.0)])
            .unwrap();
        assert_ne!(bits(&doc, id), before);
        s.cancel(&mut doc).unwrap();
        assert_eq!(bits(&doc, id), before);
    }
}

#[test]
fn heal_undo_restores_original() {
    let (mut doc, id) = doc_from(64, 64, BlendSpace::Perceptual, texture);
    let before = bits(&doc, id);
    let v0 = doc.layer(id).unwrap().pixel_version();
    let edit = clone_stroke(
        &mut doc,
        id,
        spec(CloneKind::Heal, (-10, -4)),
        hard(14.0),
        &[pt(30.0, 30.0), pt(45.0, 35.0)],
    )
    .unwrap();
    assert_ne!(doc.layer(id).unwrap().pixel_version(), v0);
    let mut history = History::default();
    history.push("Heal", vec![edit]);
    history.undo(&mut doc).unwrap();
    assert_eq!(bits(&doc, id), before);
}

/// Left half: 0.5 + texture; right half: 0.3 + the same texture (period 4,
/// so a shift by 64 keeps it).
fn shifted(x: u32, y: u32) -> Rgba {
    let t = 0.05 * ((x / 2 + y / 2) % 2) as f32;
    let v = if x < 64 { 0.5 } else { 0.3 } + t;
    [v, v * 0.5, 0.2 + t, 1.0]
}

#[test]
fn heal_removes_constant_tone_shift() {
    let (mut doc, id) = doc_from(128, 64, BlendSpace::Linear, shifted);
    let mut s = Stroke::begin_clone(&doc, id, spec(CloneKind::Heal, (-64, 0)), hard(24.0)).unwrap();
    s.add_points(&mut doc, &[pt(90.0, 30.0), pt(100.0, 34.0)])
        .unwrap();
    // While the stroke is active, the clone result shows (the source tone).
    assert!((px(&doc, id, 95, 32)[0] - shifted(31, 32)[0]).abs() < 1e-6);
    s.end(&mut doc).unwrap();
    // The source is the destination + 0.2 (red channel): the heal gives the
    // destination tone with the texture, inside and at the boundary.
    for y in 0..64 {
        for x in 64..128 {
            let (a, e) = (px(&doc, id, x, y), shifted(x, y));
            for c in 0..4 {
                assert!((a[c] - e[c]).abs() < 1e-3, "({x}, {y}) {a:?} vs {e:?}");
            }
        }
    }
}

#[test]
fn heal_flat_colors_in_perceptual_space() {
    let f = |x: u32, _y: u32| {
        if x < 40 {
            [0.6, 0.4, 0.2, 1.0]
        } else {
            [0.1, 0.2, 0.3, 1.0]
        }
    };
    let (mut doc, id) = doc_from(80, 40, BlendSpace::Perceptual, f);
    clone_stroke(
        &mut doc,
        id,
        spec(CloneKind::Heal, (-40, 0)),
        hard(16.0),
        &[pt(60.0, 20.0)],
    );
    for y in 0..40 {
        for x in 40..80 {
            let (a, e) = (px(&doc, id, x, y), f(x, y));
            for c in 0..4 {
                assert!((a[c] - e[c]).abs() < 1e-3, "({x}, {y}) {a:?} vs {e:?}");
            }
        }
    }
}

#[test]
fn heal_keeps_source_texture() {
    // Left: 0.5 + a fine checker; right: flat 0.3.
    let f = |x: u32, y: u32| {
        let v = if x < 64 {
            0.5 + 0.05 * ((x + y) % 2) as f32
        } else {
            0.3
        };
        [v, v, v, 1.0]
    };
    let (mut doc, id) = doc_from(128, 64, BlendSpace::Linear, f);
    clone_stroke(
        &mut doc,
        id,
        spec(CloneKind::Heal, (-64, 0)),
        hard(40.0),
        &[pt(96.0, 32.0)],
    );
    let a = px(&doc, id, 96, 32)[0];
    let b = px(&doc, id, 97, 32)[0];
    assert!(((a - b).abs() - 0.05).abs() < 0.005, "{a} {b}");
    assert!(((a + b) * 0.5 - 0.3).abs() < 0.01, "{a} {b}");
    // Outside the stroke nothing changes.
    assert_eq!(px(&doc, id, 70, 5), f(70, 5));
}

#[test]
fn heal_is_deterministic() {
    let run = || {
        let (mut doc, id) = doc_from(96, 64, BlendSpace::Perceptual, texture);
        let settings = BrushSettings {
            hardness: 0.3,
            ..hard(20.0)
        };
        clone_stroke(
            &mut doc,
            id,
            spec(CloneKind::Heal, (-30, 2)),
            settings,
            &[pt(50.0, 20.0), pt(80.0, 50.0)],
        );
        bits(&doc, id)
    };
    assert_eq!(run(), run());
}

#[test]
fn selection_limits_clone_and_heal() {
    for kind in [CloneKind::Clone, CloneKind::Heal] {
        let (mut doc, id) = doc_from(64, 64, BlendSpace::Perceptual, texture);
        let shape = SelectionShape::Rect {
            x: 0.0,
            y: 0.0,
            width: 32.0,
            height: 64.0,
        };
        doc.select_shape(&shape, SelectionMode::Replace, false, 0.0)
            .unwrap();
        clone_stroke(
            &mut doc,
            id,
            spec(kind, (-15, 7)),
            hard(20.0),
            &[pt(32.0, 32.0)],
        );
        for y in 0..64 {
            for x in 32..64 {
                assert_eq!(px(&doc, id, x, y), texture(x, y), "{kind:?} ({x}, {y})");
            }
        }
        assert_ne!(px(&doc, id, 30, 32), texture(30, 32), "{kind:?}");
    }
}

#[test]
#[ignore = "timing; run in release"]
fn heal_timing_1mp() {
    use std::time::Instant;
    let (mut doc, id) = doc_from(1200, 1200, BlendSpace::Perceptual, texture);
    // A dab with radius 564 covers about 1.0 MP.
    let mut s =
        Stroke::begin_clone(&doc, id, spec(CloneKind::Heal, (30, 0)), hard(1128.0)).unwrap();
    s.add_points(&mut doc, &[pt(600.0, 600.0)]).unwrap();
    let start = Instant::now();
    s.end(&mut doc).unwrap();
    println!("heal pass 1 MP: {:?}", start.elapsed());
}
