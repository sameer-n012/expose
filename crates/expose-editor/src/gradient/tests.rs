//! Tests of the gradient fill.

// Tests compare exact values, and make test values from small integers.
#![allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]

use expose_core::buffer::WorkingImage;

use super::*;
use crate::adjust::{Adjustment, BrightnessContrast};
use crate::document::{Background, DocumentSpec};
use crate::flood::FillSettings;
use crate::history::History;
use crate::layer::{LayerContent, Locks, Mask, MaskFill, MaskImage, PixelLayer, Placement};
use crate::selection::{SelectionMode, SelectionShape};

const BLACK: [f32; 3] = [0.0; 3];
const WHITE: [f32; 3] = [1.0; 3];
const RED: [f32; 3] = [1.0, 0.0, 0.0];
const BLUE: [f32; 3] = [0.0, 0.0, 1.0];

fn doc_with_layer(
    canvas: (u32, u32),
    size: (u32, u32),
    fill: Rgba,
    offset: (i32, i32),
    space: BlendSpace,
) -> (Document, LayerId) {
    let mut spec = DocumentSpec::new(canvas.0, canvas.1);
    spec.background = Background::Transparent;
    spec.blend_space = space;
    let mut doc = Document::new(spec).unwrap();
    let image = WorkingImage::new_filled(size.0, size.1, fill).unwrap();
    let content = LayerContent::Pixel(PixelLayer::new(image, offset).unwrap());
    let (id, _) = doc.add_layer("Paint", content, Placement::root(1)).unwrap();
    (doc, id)
}

fn simple_doc(w: u32, h: u32, fill: Rgba) -> (Document, LayerId) {
    doc_with_layer((w, h), (w, h), fill, (0, 0), BlendSpace::Perceptual)
}

fn stop(position: f32, color: [f32; 3], opacity: f32) -> GradientStop {
    GradientStop {
        position,
        color,
        opacity,
    }
}

fn spec(start: (f64, f64), end: (f64, f64), shape: GradientShape) -> GradientSpec {
    GradientSpec {
        start,
        end,
        shape,
        stops: vec![stop(0.0, BLACK, 1.0), stop(1.0, WHITE, 1.0)],
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        reverse: false,
    }
}

fn t_at(s: &GradientSpec, x: usize, y: usize) -> f64 {
    Geometry::new(s).t(x, y)
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
    let p = doc.layer(id).unwrap().as_pixel().unwrap();
    p.image()
        .pixels()
        .iter()
        .flatten()
        .map(|v| v.to_bits())
        .collect()
}

fn mask_data(doc: &Document, id: LayerId) -> Vec<f32> {
    doc.layer(id).unwrap().mask().unwrap().image.data().to_vec()
}

/// The angle shape uses `atan2` in f32, so the tolerance is 1e-6.
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn linear_radial_and_reflected_positions() {
    // The line starts at the center of pixel (0, 0) and has the length 10.
    let s = spec((0.5, 0.5), (10.5, 0.5), GradientShape::Linear);
    assert!(close(t_at(&s, 0, 0), 0.0));
    assert!(close(t_at(&s, 5, 0), 0.5));
    assert!(close(t_at(&s, 5, 7), 0.5));
    assert!(close(t_at(&s, 15, 0), 1.0));
    let s = spec((0.5, 0.5), (10.5, 0.5), GradientShape::Radial);
    assert!(close(t_at(&s, 3, 4), 0.5));
    assert!(close(t_at(&s, 0, 10), 1.0));
    assert!(close(t_at(&s, 30, 30), 1.0));
    let s = spec((5.5, 0.5), (15.5, 0.5), GradientShape::Reflected);
    assert!(close(t_at(&s, 0, 0), 0.5));
    assert!(close(t_at(&s, 10, 0), 0.5));
    assert!(close(t_at(&s, 5, 3), 0.0));
    // Before the start, the linear position is clamped to 0.
    let s = spec((5.5, 0.5), (15.5, 0.5), GradientShape::Linear);
    assert!(close(t_at(&s, 0, 0), 0.0));
}

#[test]
fn angle_is_clockwise_on_the_screen() {
    // `d` points to the right. The y axis points down.
    let s = spec((5.5, 5.5), (15.5, 5.5), GradientShape::Angle);
    assert!(close(t_at(&s, 8, 5), 0.0)); // right
    assert!(close(t_at(&s, 5, 8), 0.25)); // below
    assert!(close(t_at(&s, 2, 5), 0.5)); // left
    assert!(close(t_at(&s, 5, 2), 0.75)); // above
    assert!(close(t_at(&s, 5, 5), 0.0)); // at the start
    assert!(close(t_at(&s, 8, 8), 0.125)); // below right
    // A small angle counterclockwise from `d` gives a value near 1.
    assert!(t_at(&s, 15, 4) > 0.98);
    // `d` points down: clockwise from down is left.
    let s = spec((5.5, 5.5), (5.5, 15.5), GradientShape::Angle);
    assert!(close(t_at(&s, 2, 5), 0.25));
    assert!(close(t_at(&s, 8, 5), 0.75));
}

#[test]
fn diamond_positions() {
    let s = spec((5.5, 5.5), (15.5, 5.5), GradientShape::Diamond);
    assert!(close(t_at(&s, 8, 7), 0.5));
    assert!(close(t_at(&s, 8, 3), 0.5));
    assert!(close(t_at(&s, 2, 3), 0.5));
    assert!(close(t_at(&s, 5, 10), 0.5));
    assert!(close(t_at(&s, 5, 5), 0.0));
    // A diagonal line turns the diamond by 45 degrees: the corners are on
    // the line.
    let s = spec((5.5, 5.5), (9.5, 9.5), GradientShape::Diamond);
    assert!(close(t_at(&s, 7, 7), 0.5));
    // (2, 0) from the start: u = 0.25, v = 0.25 (L^2 = 32).
    assert!(close(t_at(&s, 7, 5), 0.25 + 0.25));
}

#[test]
fn reverse_mirrors_the_position() {
    let mut s = spec((0.5, 0.5), (10.5, 0.5), GradientShape::Linear);
    s.reverse = true;
    assert!(close(t_at(&s, 2, 0), 0.8));
    assert!(close(t_at(&s, 0, 0), 1.0));
    assert!(close(t_at(&s, 20, 0), 0.0));
    // In the pixels: the start pixel gets the last stop (white).
    let (mut doc, id) = simple_doc(11, 1, [0.0; 4]);
    doc.fill_gradient(id, &s, &GradientTarget::Pixels).unwrap();
    assert_eq!(px(&doc, id, 0, 0), [1.0; 4]);
    assert_eq!(px(&doc, id, 10, 0), [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn segments() {
    let stops = [
        stop(0.25, RED, 1.0),
        stop(0.5, RED, 1.0),
        stop(0.5, BLUE, 1.0),
        stop(0.75, BLUE, 1.0),
    ];
    assert_eq!(Segment::at(&stops, 0.0), Segment { a: 0, b: 0, w: 0.0 });
    assert_eq!(Segment::at(&stops, 0.375), Segment { a: 0, b: 1, w: 0.5 });
    // At equal positions the later stop applies.
    assert_eq!(Segment::at(&stops, 0.5), Segment { a: 2, b: 3, w: 0.0 });
    assert_eq!(Segment::at(&stops, 0.75), Segment { a: 3, b: 3, w: 0.0 });
    assert_eq!(Segment::at(&stops, 1.0), Segment { a: 3, b: 3, w: 0.0 });
}

#[test]
fn hard_edge_at_equal_positions() {
    let (mut doc, id) = simple_doc(8, 1, [0.0; 4]);
    let mut s = spec((0.0, 0.5), (8.0, 0.5), GradientShape::Linear);
    s.stops = vec![
        stop(0.0, RED, 1.0),
        stop(0.5, RED, 1.0),
        stop(0.5, BLUE, 1.0),
        stop(1.0, BLUE, 1.0),
    ];
    let (_, rect) = doc
        .fill_gradient(id, &s, &GradientTarget::Pixels)
        .unwrap()
        .unwrap();
    assert_eq!(rect, Rect::new(0, 0, 8, 1));
    for x in 0..4 {
        assert_eq!(px(&doc, id, x, 0), [1.0, 0.0, 0.0, 1.0], "x = {x}");
    }
    for x in 4..8 {
        assert_eq!(px(&doc, id, x, 0), [0.0, 0.0, 1.0, 1.0], "x = {x}");
    }
}

#[test]
fn interpolation_uses_the_blend_space() {
    // Pixel 1 has t = 1.5 / 3 = 0.5.
    let s = spec((0.0, 0.5), (3.0, 0.5), GradientShape::Linear);
    for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
        let (mut doc, id) = doc_with_layer((3, 1), (3, 1), [0.0; 4], (0, 0), space);
        doc.fill_gradient(id, &s, &GradientTarget::Pixels).unwrap();
        // The encoded midpoint is 0.5 (the encode of 1 can differ from 1
        // by one float step).
        let mid = space.decode([0.5; 3]);
        let p = px(&doc, id, 1, 0);
        for c in 0..3 {
            assert!((p[c] - mid[c]).abs() < 1e-6, "{space:?}: {p:?}");
        }
        assert_eq!(p[3], 1.0);
        if space == BlendSpace::Linear {
            assert_eq!(mid, [0.5; 3]);
        } else {
            assert!((mid[0] - 0.214_041).abs() < 1e-5);
        }
    }
}

#[test]
fn stop_opacity_and_spec_opacity_give_the_coverage() {
    let s0 = spec((0.0, 0.5), (3.0, 0.5), GradientShape::Linear);
    let mut s = s0.clone();
    s.stops = vec![stop(0.0, WHITE, 0.0), stop(1.0, WHITE, 1.0)];
    let black = [0.0, 0.0, 0.0, 1.0];
    let (mut doc, id) = simple_doc(3, 1, black);
    doc.fill_gradient(id, &s, &GradientTarget::Pixels).unwrap();
    // t = 0.5: coverage 0.5.
    assert_eq!(px(&doc, id, 1, 0), [0.5, 0.5, 0.5, 1.0]);
    s.opacity = 0.5;
    let (mut doc, id) = simple_doc(3, 1, black);
    doc.fill_gradient(id, &s, &GradientTarget::Pixels).unwrap();
    assert_eq!(px(&doc, id, 1, 0), [0.25, 0.25, 0.25, 1.0]);
    // Transparent stops on a transparent layer: no change.
    s.stops = vec![stop(0.0, WHITE, 0.0), stop(1.0, WHITE, 0.0)];
    let (mut doc, id) = simple_doc(3, 1, [0.0; 4]);
    let revision = doc.revision();
    assert!(
        doc.fill_gradient(id, &s, &GradientTarget::Pixels)
            .unwrap()
            .is_none()
    );
    assert_eq!(doc.revision(), revision);
}

#[test]
fn selection_limits_the_fill() {
    let o = [0.0, 0.0, 0.0, 1.0];
    let (mut doc, id) = simple_doc(4, 1, o);
    let shape = SelectionShape::Rect {
        x: 0.0,
        y: 0.0,
        width: 2.5,
        height: 1.0,
    };
    doc.select_shape(&shape, SelectionMode::Replace, true, 0.0)
        .unwrap();
    let mut s = spec((0.0, 0.5), (4.0, 0.5), GradientShape::Linear);
    s.stops = vec![stop(0.0, WHITE, 1.0), stop(1.0, WHITE, 1.0)];
    let (_, rect) = doc
        .fill_gradient(id, &s, &GradientTarget::Pixels)
        .unwrap()
        .unwrap();
    assert_eq!(rect, Rect::new(0, 0, 3, 1));
    assert_eq!(px(&doc, id, 0, 0), [1.0; 4]);
    assert_eq!(px(&doc, id, 1, 0), [1.0; 4]);
    assert_eq!(px(&doc, id, 2, 0), [0.5, 0.5, 0.5, 1.0]);
    assert_eq!(px(&doc, id, 3, 0), o);
}

#[test]
fn transparency_lock_keeps_alpha() {
    let (mut doc, id) = simple_doc(2, 1, [0.0; 4]);
    doc.write_pixels(
        id,
        Rect::new(0, 0, 2, 1),
        &[[0.25, 0.25, 0.25, 0.5], [0.0; 4]],
    )
    .unwrap();
    let locks = Locks {
        transparency: true,
        ..Locks::NONE
    };
    doc.set_locks(id, locks).unwrap();
    let mut s = spec((0.0, 0.5), (2.0, 0.5), GradientShape::Linear);
    s.stops = vec![stop(0.0, RED, 1.0), stop(1.0, RED, 1.0)];
    let (_, rect) = doc
        .fill_gradient(id, &s, &GradientTarget::Pixels)
        .unwrap()
        .unwrap();
    assert_eq!(rect, Rect::new(0, 0, 1, 1));
    assert_eq!(px(&doc, id, 0, 0), [0.5, 0.0, 0.0, 0.5]);
    assert_eq!(px(&doc, id, 1, 0), [0.0; 4]);
}

#[test]
fn flat_gradient_equals_fill_region() {
    let (w, h) = (70, 67);
    let pixels: Vec<Rgba> = (0..w * h)
        .map(|i| {
            let v = (i % 13) as f32 / 13.0;
            [v * 0.6, 0.2 + v * 0.1, 0.3, 0.4 + v * 0.6]
        })
        .collect();
    let (mut doc, id) = simple_doc(w, h, [0.0; 4]);
    doc.write_pixels(id, Rect::new(0, 0, w, h), &pixels)
        .unwrap();
    let color = [0.9, 0.3, 0.6];
    for shape in [GradientShape::Linear, GradientShape::Angle] {
        let mut s = spec((3.0, 4.0), (50.0, 20.0), shape);
        s.stops = vec![stop(0.2, color, 1.0), stop(0.7, color, 1.0)];
        s.opacity = 0.7;
        s.blend_mode = BlendMode::Multiply;
        let mut a = doc.clone();
        let mut b = doc.clone();
        a.fill_gradient(id, &s, &GradientTarget::Pixels).unwrap();
        let fill = FillSettings {
            color,
            opacity: 0.7,
            blend_mode: BlendMode::Multiply,
        };
        let region = MaskImage::new_filled(w, h, 1.0).unwrap();
        b.fill_region(id, &region, &fill).unwrap();
        assert_eq!(bits(&a, id), bits(&b, id));
        assert_ne!(bits(&a, id), bits(&doc, id));
    }
}

#[test]
fn mask_target_values_and_undo() {
    let (mut doc, id) = simple_doc(8, 2, [0.0, 0.0, 0.0, 1.0]);
    // A mask of 6 x 2 at canvas x 4..10: only x 4..8 is in the canvas.
    let mask = Mask::new(
        MaskImage::new_filled(6, 2, 1.0).unwrap(),
        (4, 0),
        MaskFill::White,
    );
    doc.set_mask(id, Some(mask)).unwrap();
    let before = mask_data(&doc, id);
    let pixels_before = bits(&doc, id);
    let version = doc.layer(id).unwrap().mask_version();
    let s = spec((0.0, 1.0), (8.0, 1.0), GradientShape::Linear);
    let target = GradientTarget::Mask {
        values: vec![0.0, 1.0],
    };
    let (edit, rect) = doc.fill_gradient(id, &s, &target).unwrap().unwrap();
    // m = t = (x + 0.5) / 8 < 1 on canvas x 4..8, so all of them change.
    // The mask values at canvas x 8..10 are outside the canvas.
    assert_eq!(rect, Rect::new(4, 0, 4, 2));
    assert_ne!(doc.layer(id).unwrap().mask_version(), version);
    let after = mask_data(&doc, id);
    for (x, &m) in after.iter().take(6).enumerate() {
        let cx = x + 4;
        if cx < 8 {
            assert_eq!(m, (cx as f32 + 0.5) / 8.0, "x = {cx}");
        } else {
            assert_eq!(m, 1.0, "x = {cx}");
        }
    }
    // The pixels do not change.
    assert_eq!(bits(&doc, id), pixels_before);
    let mut history = History::default();
    history.push("Gradient", vec![edit]);
    assert!(history.undo(&mut doc).unwrap());
    assert_eq!(mask_data(&doc, id), before);
    assert!(history.redo(&mut doc).unwrap());
    assert_eq!(mask_data(&doc, id), after);
}

#[test]
fn mask_target_on_a_non_pixel_layer() {
    let (mut doc, _) = simple_doc(4, 1, [0.0; 4]);
    let content =
        LayerContent::Adjustment(Adjustment::BrightnessContrast(BrightnessContrast::default()));
    let (adj, _) = doc.add_layer("Adj", content, Placement::root(2)).unwrap();
    let mask = Mask::new(
        MaskImage::new_filled(4, 1, 0.0).unwrap(),
        (0, 0),
        MaskFill::Black,
    );
    doc.set_mask(adj, Some(mask)).unwrap();
    let mut s = spec((0.0, 0.5), (4.0, 0.5), GradientShape::Linear);
    s.opacity = 0.5;
    let target = GradientTarget::Mask {
        values: vec![1.0, 1.0],
    };
    doc.fill_gradient(adj, &s, &target).unwrap().unwrap();
    assert_eq!(mask_data(&doc, adj), vec![0.5; 4]);
}

#[test]
#[allow(clippy::too_many_lines)]
fn errors() {
    let (mut doc, id) = simple_doc(4, 4, [0.0; 4]);
    let good = spec((0.0, 0.0), (4.0, 0.0), GradientShape::Linear);
    let px_target = GradientTarget::Pixels;
    let invalid = |s: &GradientSpec, t: &GradientTarget, doc: &mut Document| {
        matches!(
            doc.fill_gradient(id, s, t),
            Err(EditorError::InvalidGradient(_))
        )
    };
    let mut s = good.clone();
    s.stops.truncate(1);
    assert!(invalid(&s, &px_target, &mut doc));
    let mut s = good.clone();
    s.stops = vec![stop(0.6, BLACK, 1.0), stop(0.4, WHITE, 1.0)];
    assert!(invalid(&s, &px_target, &mut doc));
    let mut s = good.clone();
    s.stops = (0..33).map(|i| stop(i as f32 / 32.0, BLACK, 1.0)).collect();
    assert!(invalid(&s, &px_target, &mut doc));
    s.stops.pop();
    assert!(doc.fill_gradient(id, &s, &px_target).is_ok());
    let mut s = good.clone();
    s.end = (0.3, 0.3);
    assert!(invalid(&s, &px_target, &mut doc));
    let mut s = good.clone();
    s.blend_mode = BlendMode::Dissolve;
    assert!(invalid(&s, &px_target, &mut doc));
    let mut s = good.clone();
    s.blend_mode = BlendMode::Multiply;
    let mask = Mask::new(
        MaskImage::new_filled(4, 4, 1.0).unwrap(),
        (0, 0),
        MaskFill::White,
    );
    let one_value = GradientTarget::Mask { values: vec![1.0] };
    let two_values = GradientTarget::Mask {
        values: vec![0.0, 1.0],
    };
    assert!(invalid(&s, &two_values, &mut doc));
    assert!(invalid(&good, &one_value, &mut doc));
    // No mask.
    assert_eq!(
        doc.fill_gradient(id, &good, &two_values).unwrap_err(),
        EditorError::NoMask(id)
    );
    doc.set_mask(id, Some(mask)).unwrap();
    assert!(matches!(
        doc.fill_gradient(
            id,
            &good,
            &GradientTarget::Mask {
                values: vec![0.0, 1.5]
            }
        ),
        Err(EditorError::OutOfRange { .. })
    ));
    // Bad values.
    let mut s = good.clone();
    s.start.0 = f64::NAN;
    assert!(matches!(
        doc.fill_gradient(id, &s, &px_target),
        Err(EditorError::NonFinite { .. })
    ));
    let mut s = good.clone();
    s.end.1 = 2.0e5;
    assert!(matches!(
        doc.fill_gradient(id, &s, &px_target),
        Err(EditorError::OutOfRange { .. })
    ));
    let mut s = good.clone();
    s.stops[0].color[1] = f32::INFINITY;
    assert!(matches!(
        doc.fill_gradient(id, &s, &px_target),
        Err(EditorError::NonFinite { .. })
    ));
    let mut s = good.clone();
    s.stops[1].position = 1.5;
    assert!(matches!(
        doc.fill_gradient(id, &s, &px_target),
        Err(EditorError::OutOfRange { .. })
    ));
    let mut s = good.clone();
    s.stops[1].opacity = -0.1;
    assert!(matches!(
        doc.fill_gradient(id, &s, &px_target),
        Err(EditorError::OutOfRange { .. })
    ));
    let mut s = good.clone();
    s.opacity = 2.0;
    assert!(matches!(
        doc.fill_gradient(id, &s, &px_target),
        Err(EditorError::OutOfRange { .. })
    ));
    // Layers.
    let missing = LayerId(9999);
    assert_eq!(
        doc.fill_gradient(missing, &good, &px_target).unwrap_err(),
        EditorError::LayerNotFound(missing)
    );
    let content =
        LayerContent::Adjustment(Adjustment::BrightnessContrast(BrightnessContrast::default()));
    let (adj, _) = doc.add_layer("Adj", content, Placement::root(2)).unwrap();
    assert_eq!(
        doc.fill_gradient(adj, &good, &px_target).unwrap_err(),
        EditorError::NotAPixelLayer(adj)
    );
    let before = bits(&doc, id);
    let before_mask = mask_data(&doc, id);
    let locks = Locks {
        pixels: true,
        ..Locks::NONE
    };
    doc.set_locks(id, locks).unwrap();
    let locked = EditorError::Locked {
        id,
        lock: LockKind::Pixels,
    };
    assert_eq!(
        doc.fill_gradient(id, &good, &px_target).unwrap_err(),
        locked
    );
    assert_eq!(
        doc.fill_gradient(id, &good, &two_values).unwrap_err(),
        locked
    );
    assert_eq!(bits(&doc, id), before);
    assert_eq!(mask_data(&doc, id), before_mask);
}

#[test]
fn layer_does_not_grow() {
    let (mut doc, id) = doc_with_layer((8, 8), (4, 4), [0.0; 4], (6, -2), BlendSpace::Perceptual);
    let mut s = spec((0.0, 0.0), (8.0, 0.0), GradientShape::Radial);
    s.stops = vec![stop(0.0, RED, 1.0), stop(1.0, RED, 1.0)];
    let (_, rect) = doc
        .fill_gradient(id, &s, &GradientTarget::Pixels)
        .unwrap()
        .unwrap();
    assert_eq!(rect, Rect::new(6, 0, 2, 2));
    let p = doc.layer(id).unwrap().as_pixel().unwrap();
    assert_eq!((p.image().width(), p.image().height()), (4, 4));
    assert_eq!(px(&doc, id, 0, 2), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(px(&doc, id, 2, 2), [0.0; 4]);
    assert_eq!(px(&doc, id, 0, 1), [0.0; 4]);
    let (mut doc, id) = doc_with_layer((8, 8), (4, 4), [0.0; 4], (20, 0), BlendSpace::Perceptual);
    assert!(
        doc.fill_gradient(id, &s, &GradientTarget::Pixels)
            .unwrap()
            .is_none()
    );
}

#[test]
fn undo_restores_the_exact_values() {
    let (w, h) = (150, 140);
    let pixels: Vec<Rgba> = (0..w * h)
        .map(|i| {
            let v = (i % 17) as f32 / 17.0;
            [v * 0.5, 0.1, v * 0.25, 0.5 + v * 0.5]
        })
        .collect();
    let (mut doc, id) = simple_doc(w, h, [0.0; 4]);
    doc.write_pixels(id, Rect::new(0, 0, w, h), &pixels)
        .unwrap();
    let before = bits(&doc, id);
    let version = doc.layer(id).unwrap().pixel_version();
    let revision = doc.revision();
    let mut s = spec((40.0, 50.0), (60.0, 50.0), GradientShape::Radial);
    s.stops = vec![
        stop(0.0, [0.9, 0.2, 0.4], 1.0),
        stop(0.5, [0.1, 0.8, 0.3], 0.5),
        stop(1.0, [0.0, 0.0, 0.0], 0.0),
    ];
    s.blend_mode = BlendMode::Overlay;
    let (edit, rect) = doc
        .fill_gradient(id, &s, &GradientTarget::Pixels)
        .unwrap()
        .unwrap();
    // Opacity 0 after the radius 20: the changed area is near the circle.
    assert!(rect.x >= 19 && rect.y >= 29 && rect.x + rect.width <= 61);
    assert_ne!(doc.layer(id).unwrap().pixel_version(), version);
    assert_ne!(doc.revision(), revision);
    let after = bits(&doc, id);
    assert_ne!(after, before);
    // Only the tiles near the circle: x 0..128, y 0..128.
    assert!(edit.memory_bytes() < 4 * 64 * 64 * 16 + 4096);
    let mut history = History::default();
    history.push("Gradient", vec![edit]);
    assert!(history.undo(&mut doc).unwrap());
    assert_eq!(bits(&doc, id), before);
    assert!(history.redo(&mut doc).unwrap());
    assert_eq!(bits(&doc, id), after);
}

#[test]
fn table_error_is_small() {
    // A black to white gradient in the linear space: the color is `t`.
    let s = spec((0.0, 0.5), (1000.0, 0.5), GradientShape::Linear);
    let (mut doc, id) = doc_with_layer((1000, 1), (1000, 1), [0.0; 4], (0, 0), BlendSpace::Linear);
    doc.fill_gradient(id, &s, &GradientTarget::Pixels).unwrap();
    for x in 0..1000 {
        let t = (x as f32 + 0.5) / 1000.0;
        let v = px(&doc, id, x, 0)[0];
        assert!((v - t).abs() <= 1.0 / 8192.0 + 1e-6, "x = {x}");
    }
}

/// Timing at 4096 x 4096 (run with `--release --ignored --nocapture`).
#[test]
#[ignore = "timing only"]
fn timing_4096() {
    use std::time::Instant;
    let n = 4096;
    let (doc, id) = simple_doc(n, n, [0.2, 0.3, 0.4, 1.0]);
    let fl = f64::from(n);
    // For comparison: a paint bucket fill of the full canvas.
    let region = MaskImage::new_filled(n, n, 1.0).unwrap();
    let fill = FillSettings {
        color: RED,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
    };
    let t = Instant::now();
    let mut d = doc.clone();
    println!("clone: {:?}", t.elapsed());
    let t = Instant::now();
    let _ = d.fill_region(id, &region, &fill).unwrap();
    println!("fill_region Normal: {:?}", t.elapsed());
    for shape in [
        GradientShape::Linear,
        GradientShape::Radial,
        GradientShape::Angle,
        GradientShape::Diamond,
    ] {
        let mut s = spec((0.0, 0.0), (fl, fl), shape);
        s.stops = vec![
            stop(0.0, RED, 1.0),
            stop(0.5, [0.1, 0.8, 0.3], 0.8),
            stop(1.0, BLUE, 1.0),
        ];
        for mode in [BlendMode::Normal, BlendMode::Normal, BlendMode::Multiply] {
            s.blend_mode = mode;
            let mut d = doc.clone();
            let t = Instant::now();
            let _ = d.fill_gradient(id, &s, &GradientTarget::Pixels).unwrap();
            let first = t.elapsed();
            // A second fill with the same result changes no value: this is
            // the time without the undo memory.
            let t = Instant::now();
            let _ = d.fill_gradient(id, &s, &GradientTarget::Pixels).unwrap();
            println!(
                "{shape:?} {mode:?}: {first:?} (no change: {:?})",
                t.elapsed()
            );
        }
    }
}
