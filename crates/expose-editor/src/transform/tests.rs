//! Tests of the free transform.

// Tests compare exact values, and make test values from small integers.
#![allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use std::f64::consts::FRAC_PI_2;

use super::*;
use crate::adjust::{Adjustment, BrightnessContrast};
use crate::document::{Background, DocumentSpec, MAX_DIMENSION};
use crate::history::History;
use crate::layer::{LayerContent, Locks, MaskFill, PixelLayer, Placement};

fn doc_with_image(image: WorkingImage, offset: (i32, i32)) -> (Document, LayerId) {
    let mut spec = DocumentSpec::new(64, 64);
    spec.background = Background::Transparent;
    let mut doc = Document::new(spec).unwrap();
    let content = LayerContent::Pixel(PixelLayer::new(image, offset).unwrap());
    let (id, _) = doc.add_layer("Paint", content, Placement::root(1)).unwrap();
    (doc, id)
}

/// An opaque image with a different color at each pixel.
fn pattern(w: u32, h: u32) -> WorkingImage {
    let pixels = (0..w * h)
        .map(|i| {
            let v = i as f32;
            [v, v * 0.5 + 1.0, 100.0 - v, 1.0]
        })
        .collect();
    WorkingImage::from_pixels(w, h, pixels).unwrap()
}

fn image_of(doc: &Document, id: LayerId) -> (WorkingImage, (i32, i32)) {
    let p = doc.layer(id).unwrap().as_pixel().unwrap();
    (p.image().clone(), p.offset())
}

fn close(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
}

// ---------------------------------------------------------------
// Affine algebra.
// ---------------------------------------------------------------

#[test]
fn rotate_turns_x_axis_to_y_axis() {
    assert!(close(Affine::rotate(FRAC_PI_2).apply(1.0, 0.0), (0.0, 1.0)));
}

#[test]
fn then_applies_self_first() {
    let m = Affine::translate(1.0, 0.0).then(Affine::scale(2.0, 3.0));
    assert!(close(m.apply(0.0, 1.0), (2.0, 3.0)));
    let n = Affine::scale(2.0, 3.0).then(Affine::translate(1.0, 0.0));
    assert!(close(n.apply(0.0, 1.0), (1.0, 3.0)));
}

#[test]
fn inverse_undoes_the_transform() {
    let m = Affine::scale(2.0, 0.5)
        .then(Affine::rotate(0.3))
        .then(Affine::translate(5.0, -2.0));
    let inv = m.inverse().unwrap();
    let (x, y) = m.apply(3.0, 7.0);
    assert!(close(inv.apply(x, y), (3.0, 7.0)));
    let id = m.then(inv).to_array();
    for (v, e) in id.iter().zip(Affine::IDENTITY.to_array()) {
        assert!((v - e).abs() < 1e-12);
    }
    assert!((m.determinant() - 1.0).abs() < 1e-12);
    assert_eq!(Affine::from_array(m.to_array()), m);
}

#[test]
fn validate_refuses_bad_matrices() {
    let nan = Affine::from_array([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]);
    assert!(matches!(
        nan.validate(),
        Err(EditorError::InvalidTransform(_))
    ));
    let inf = Affine::translate(f64::INFINITY, 0.0);
    assert!(matches!(
        inf.validate(),
        Err(EditorError::InvalidTransform(_))
    ));
    let singular = Affine::scale(1.0, 0.0);
    assert!(matches!(
        singular.validate(),
        Err(EditorError::InvalidTransform(_))
    ));
    assert!(singular.inverse().is_none());
    assert!(Affine::IDENTITY.validate().is_ok());
}

// ---------------------------------------------------------------
// Content bounds.
// ---------------------------------------------------------------

#[test]
fn content_bounds_of_empty_layer_is_none() {
    let (doc, id) = doc_with_image(WorkingImage::new(8, 8).unwrap(), (0, 0));
    assert_eq!(doc.layer_content_bounds(id).unwrap(), None);
}

#[test]
fn content_bounds_include_offset_and_outside_area() {
    let mut image = WorkingImage::new(10, 8).unwrap();
    *image.pixel_mut(2, 3).unwrap() = [0.1, 0.1, 0.1, 0.1];
    *image.pixel_mut(6, 5).unwrap() = [0.5, 0.5, 0.5, 0.5];
    let (doc, id) = doc_with_image(image, (-20, 4));
    assert_eq!(
        doc.layer_content_bounds(id).unwrap(),
        Some(CanvasRect {
            x: -18,
            y: 7,
            width: 5,
            height: 3
        })
    );
}

#[test]
fn content_bounds_of_adjustment_layer_is_an_error() {
    let (mut doc, _) = doc_with_image(pattern(2, 2), (0, 0));
    let content =
        LayerContent::Adjustment(Adjustment::BrightnessContrast(BrightnessContrast::default()));
    let (adj, _) = doc.add_layer("Adj", content, Placement::root(2)).unwrap();
    assert_eq!(
        doc.layer_content_bounds(adj),
        Err(EditorError::NotAPixelLayer(adj))
    );
}

// ---------------------------------------------------------------
// Transforms.
// ---------------------------------------------------------------

#[test]
fn whole_pixel_translation_changes_only_the_offset() {
    let (mut doc, id) = doc_with_image(pattern(5, 4), (1, 2));
    let mask = Mask::new(
        MaskImage::new_filled(5, 4, 0.5).unwrap(),
        (1, 2),
        MaskFill::Black,
    );
    doc.set_mask(id, Some(mask)).unwrap();
    let (before, _) = image_of(&doc, id);
    let version = doc.layer(id).unwrap().pixel_version();
    let m = Affine::translate(3.0, -7.0 + 1e-12);
    doc.transform_layer(id, &m, Interpolation::Bicubic).unwrap();
    let (after, offset) = image_of(&doc, id);
    assert_eq!(after, before);
    assert_eq!(offset, (4, -5));
    assert_eq!(doc.layer(id).unwrap().mask().unwrap().offset, (4, -5));
    assert_eq!(doc.layer(id).unwrap().pixel_version(), version);
}

#[test]
fn scale_two_nearest_doubles_pixels() {
    let src = pattern(3, 2);
    let (mut doc, id) = doc_with_image(src.clone(), (0, 0));
    doc.transform_layer(id, &Affine::scale(2.0, 2.0), Interpolation::Nearest)
        .unwrap();
    let (out, offset) = image_of(&doc, id);
    assert_eq!(offset, (0, 0));
    assert_eq!((out.width(), out.height()), (6, 4));
    for y in 0..4 {
        for x in 0..6 {
            assert_eq!(out.pixel(x, y), src.pixel(x / 2, y / 2), "at {x},{y}");
        }
    }
}

#[test]
fn rotate_ninety_about_center_nearest_is_exact() {
    let src = pattern(4, 2);
    let (mut doc, id) = doc_with_image(src.clone(), (0, 0));
    let m = Affine::translate(-2.0, -1.0)
        .then(Affine::rotate(FRAC_PI_2))
        .then(Affine::translate(2.0, 1.0));
    doc.transform_layer(id, &m, Interpolation::Nearest).unwrap();
    let (out, offset) = image_of(&doc, id);
    assert_eq!(offset, (1, -1));
    assert_eq!((out.width(), out.height()), (2, 4));
    // Source pixel (i, j) goes to canvas pixel (2 - j, i - 1), which is
    // image pixel (1 - j, i).
    for j in 0..2 {
        for i in 0..4 {
            assert_eq!(out.pixel(1 - j, i), src.pixel(i, j), "source {i},{j}");
        }
    }
}

#[test]
fn bicubic_subpixel_shift_keeps_total_alpha() {
    // A smooth blob, so that no clamp of overshoot changes the sum.
    let n = 32u32;
    let pixels = (0..n * n)
        .map(|i| {
            let (x, y) = ((i % n) as f32 - 15.5, (i / n) as f32 - 15.5);
            let a = (-(x * x + y * y) / (2.0 * 16.0)).exp();
            [a * 0.2, a * 0.4, a * 0.6, a]
        })
        .collect();
    let src = WorkingImage::from_pixels(n, n, pixels).unwrap();
    let total = |img: &WorkingImage| img.pixels().iter().map(|p| f64::from(p[3])).sum::<f64>();
    let before = total(&src);
    let (mut doc, id) = doc_with_image(src, (0, 0));
    doc.transform_layer(id, &Affine::translate(0.3, 0.6), Interpolation::Bicubic)
        .unwrap();
    let (out, offset) = image_of(&doc, id);
    assert_eq!(offset, (0, 0));
    assert_eq!((out.width(), out.height()), (33, 33));
    let after = total(&out);
    assert!(
        (after - before).abs() / before < 1e-3,
        "{before} vs {after}"
    );
}

fn checkerboard(n: u32) -> WorkingImage {
    let pixels = (0..n * n)
        .map(|i| {
            let v = ((i % n + i / n) % 2) as f32;
            [v, v, v, 1.0]
        })
        .collect();
    WorkingImage::from_pixels(n, n, pixels).unwrap()
}

#[test]
fn downscale_of_checkerboard_gives_mid_gray() {
    for interp in [Interpolation::Bicubic, Interpolation::Bilinear] {
        let (mut doc, id) = doc_with_image(checkerboard(64), (0, 0));
        doc.transform_layer(id, &Affine::scale(0.25, 0.25), interp)
            .unwrap();
        let (out, _) = image_of(&doc, id);
        assert_eq!((out.width(), out.height()), (16, 16));
        for y in 2..14 {
            for x in 2..14 {
                let p = out.pixel(x, y).unwrap();
                assert!((p[0] - 0.5).abs() < 0.02, "{interp:?} at {x},{y}: {p:?}");
                assert!((p[3] - 1.0).abs() < 1e-4);
            }
        }
    }
}

#[test]
fn rotated_downscale_of_checkerboard_gives_mid_gray() {
    let (mut doc, id) = doc_with_image(checkerboard(64), (0, 0));
    let m = Affine::scale(0.25, 0.25).then(Affine::rotate(0.5));
    doc.transform_layer(id, &m, Interpolation::Bicubic).unwrap();
    let (out, offset) = image_of(&doc, id);
    // The center of the source maps to (32, 32) · 0.25 rotated.
    let (cx, cy) = m.apply(32.0, 32.0);
    let x = (cx - f64::from(offset.0)) as u32;
    let y = (cy - f64::from(offset.1)) as u32;
    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        let p = out.pixel(x + dx, y + dy).unwrap();
        assert!((p[0] - 0.5).abs() < 0.03, "{p:?}");
    }
}

#[test]
fn mask_is_transformed_and_keeps_its_fill() {
    let (mut doc, id) = doc_with_image(pattern(4, 4), (0, 0));
    let values = (0..16).map(|i| if i % 4 < 2 { 1.0 } else { 0.0 }).collect();
    let mask = Mask::new(
        MaskImage::from_data(4, 4, values).unwrap(),
        (0, 0),
        MaskFill::Black,
    );
    doc.set_mask(id, Some(mask.clone())).unwrap();
    doc.transform_layer(id, &Affine::scale(2.0, 2.0), Interpolation::Nearest)
        .unwrap();
    let m = doc.layer(id).unwrap().mask().unwrap();
    assert_eq!(m.fill, MaskFill::Black);
    assert!(m.enabled);
    assert_eq!(m.offset, (0, 0));
    assert_eq!((m.image.width(), m.image.height()), (8, 8));
    for y in 0..8 {
        for x in 0..8 {
            assert_eq!(m.image.get(x, y), mask.image.get(x / 2, y / 2));
        }
    }
    assert_eq!(m.value_at(9, 0), 0.0);
}

#[test]
fn filtered_mask_blends_with_its_fill_at_the_edge() {
    let (mut doc, id) = doc_with_image(pattern(8, 8), (0, 0));
    let mask = Mask::new(
        MaskImage::new_filled(4, 4, 0.0).unwrap(),
        (2, 2),
        MaskFill::White,
    );
    doc.set_mask(id, Some(mask)).unwrap();
    doc.transform_layer(id, &Affine::translate(0.5, 0.0), Interpolation::Bilinear)
        .unwrap();
    let m = doc.layer(id).unwrap().mask().unwrap();
    assert_eq!(m.offset, (2, 2));
    assert_eq!((m.image.width(), m.image.height()), (5, 4));
    // The left column samples half fill (1) and half mask (0).
    assert!((m.image.get(0, 1).unwrap() - 0.5).abs() < 1e-6);
    assert_eq!(m.image.get(2, 1).unwrap(), 0.0);
    assert!((m.image.get(4, 1).unwrap() - 0.5).abs() < 1e-6);
    assert_eq!(m.fill, MaskFill::White);
}

#[test]
fn transform_uses_only_the_content_bounds() {
    let mut image = WorkingImage::new(20, 20).unwrap();
    for y in 5..7 {
        for x in 10..13 {
            *image.pixel_mut(x, y).unwrap() = [1.0, 0.0, 0.0, 1.0];
        }
    }
    let (mut doc, id) = doc_with_image(image, (-4, 3));
    doc.transform_layer(id, &Affine::scale(2.0, 2.0), Interpolation::Nearest)
        .unwrap();
    let (out, offset) = image_of(&doc, id);
    // Content canvas rect (6, 8, 3, 2) -> (12, 16, 6, 4).
    assert_eq!(offset, (12, 16));
    assert_eq!((out.width(), out.height()), (6, 4));
    assert!(out.pixels().iter().all(|p| *p == [1.0, 0.0, 0.0, 1.0]));
}

// ---------------------------------------------------------------
// Errors.
// ---------------------------------------------------------------

#[test]
fn locks_prevent_transform() {
    let cases = [
        (
            Locks {
                pixels: true,
                ..Locks::NONE
            },
            LockKind::Pixels,
        ),
        (
            Locks {
                position: true,
                ..Locks::NONE
            },
            LockKind::Position,
        ),
        (
            Locks {
                all: true,
                ..Locks::NONE
            },
            LockKind::All,
        ),
    ];
    let m = Affine::scale(2.0, 2.0);
    for (locks, lock) in cases {
        let (mut doc, id) = doc_with_image(pattern(2, 2), (0, 0));
        doc.set_locks(id, locks).unwrap();
        let rev = doc.revision();
        assert_eq!(
            doc.transform_layer(id, &m, Interpolation::Bicubic),
            Err(EditorError::Locked { id, lock })
        );
        assert_eq!(doc.revision(), rev);
    }
    // The transparency lock does not prevent a transform.
    let (mut doc, id) = doc_with_image(pattern(2, 2), (0, 0));
    doc.set_locks(
        id,
        Locks {
            transparency: true,
            ..Locks::NONE
        },
    )
    .unwrap();
    assert!(doc.transform_layer(id, &m, Interpolation::Bicubic).is_ok());
}

#[test]
fn bad_targets_and_matrices_are_errors() {
    let (mut doc, id) = doc_with_image(pattern(2, 2), (0, 0));
    let content =
        LayerContent::Adjustment(Adjustment::BrightnessContrast(BrightnessContrast::default()));
    let (adj, _) = doc.add_layer("Adj", content, Placement::root(2)).unwrap();
    let m = Affine::scale(2.0, 2.0);
    assert_eq!(
        doc.transform_layer(adj, &m, Interpolation::Bicubic),
        Err(EditorError::NotAPixelLayer(adj))
    );
    assert!(matches!(
        doc.transform_layer(id, &Affine::scale(0.0, 1.0), Interpolation::Bicubic),
        Err(EditorError::InvalidTransform(_))
    ));
    assert!(matches!(
        doc.transform_layer(
            id,
            &Affine::translate(f64::NAN, 0.0),
            Interpolation::Bicubic
        ),
        Err(EditorError::InvalidTransform(_))
    ));

    let (mut empty, eid) = doc_with_image(WorkingImage::new(4, 4).unwrap(), (0, 0));
    assert!(matches!(
        empty.transform_layer(eid, &m, Interpolation::Bicubic),
        Err(EditorError::InvalidTransform(_))
    ));
}

#[test]
fn too_large_result_is_an_error() {
    let (mut doc, id) = doc_with_image(pattern(10, 10), (0, 0));
    let scale = f64::from(MAX_DIMENSION) / 5.0;
    assert!(matches!(
        doc.transform_layer(id, &Affine::scale(scale, 1.0), Interpolation::Nearest),
        Err(EditorError::TooLarge { .. })
    ));
    assert!(matches!(
        doc.transform_layer(
            id,
            &Affine::scale(1.5, 1.0).then(Affine::translate(3e9, 0.0)),
            Interpolation::Nearest
        ),
        Err(EditorError::OffsetOverflow)
    ));
    assert!(matches!(
        doc.transform_layer(id, &Affine::translate(3e9, 0.0), Interpolation::Nearest),
        Err(EditorError::OffsetOverflow)
    ));
}

// ---------------------------------------------------------------
// Undo, redo, and versions.
// ---------------------------------------------------------------

#[test]
fn undo_and_redo_restore_exactly() {
    let (mut doc, id) = doc_with_image(pattern(6, 5), (3, -2));
    let mask = Mask::new(
        MaskImage::new_filled(7, 7, 0.25).unwrap(),
        (1, 1),
        MaskFill::White,
    );
    doc.set_mask(id, Some(mask)).unwrap();
    let before = doc.layer(id).unwrap().clone();
    let (pv, mv, rev) = (
        before.pixel_version(),
        before.mask_version(),
        doc.revision(),
    );

    let mut history = History::default();
    let m = Affine::rotate(0.4).then(Affine::translate(2.5, 1.0));
    let edit = doc.transform_layer(id, &m, Interpolation::Bicubic).unwrap();
    history.push("Transform", vec![edit]);
    let after = doc.layer(id).unwrap().clone();
    assert_ne!(after.pixel_version(), pv);
    assert_ne!(after.mask_version(), mv);
    assert_ne!(doc.revision(), rev);
    assert_ne!(after.as_pixel(), before.as_pixel());
    assert_ne!(after.mask(), before.mask());

    assert!(history.undo(&mut doc).unwrap());
    let undone = doc.layer(id).unwrap();
    assert_eq!(undone.as_pixel(), before.as_pixel());
    assert_eq!(undone.mask(), before.mask());

    assert!(history.redo(&mut doc).unwrap());
    let redone = doc.layer(id).unwrap();
    assert_eq!(redone.as_pixel(), after.as_pixel());
    assert_eq!(redone.mask(), after.mask());
}

#[test]
fn edit_without_mask_keeps_no_mask() {
    let (mut doc, id) = doc_with_image(pattern(3, 3), (0, 0));
    let mut history = History::default();
    let edit = doc
        .transform_layer(id, &Affine::scale(1.5, 1.5), Interpolation::Bilinear)
        .unwrap();
    assert!(edit.memory_bytes() >= 9 * 16);
    history.push("Transform", vec![edit]);
    assert!(doc.layer(id).unwrap().mask().is_none());
    history.undo(&mut doc).unwrap();
    let (img, offset) = image_of(&doc, id);
    assert_eq!(img, pattern(3, 3));
    assert_eq!(offset, (0, 0));
}

// ---------------------------------------------------------------
// Timing.
// ---------------------------------------------------------------

/// Run with `cargo test -p expose-editor --release -- --ignored
/// transform_timing --nocapture`.
#[test]
#[ignore = "timing test; run in release mode"]
fn transform_timing() {
    let n = 4096u32;
    let src = WorkingImage::new_filled(n, n, [0.3, 0.5, 0.7, 1.0]).unwrap();
    let mut spec = DocumentSpec::new(n, n);
    spec.background = Background::Transparent;
    let mut doc = Document::new(spec).unwrap();
    let content = LayerContent::Pixel(PixelLayer::new(src, (0, 0)).unwrap());
    let (id, _) = doc.add_layer("Big", content, Placement::root(1)).unwrap();
    let c = f64::from(n) / 2.0;
    let m = Affine::translate(-c, -c)
        .then(Affine::scale(0.8, 0.8))
        .then(Affine::rotate(30f64.to_radians()))
        .then(Affine::translate(c, c));
    let start = std::time::Instant::now();
    doc.transform_layer(id, &m, Interpolation::Bicubic).unwrap();
    println!(
        "rotate 30 + scale 0.8 of {n}x{n}, bicubic: {:?}",
        start.elapsed()
    );
}
