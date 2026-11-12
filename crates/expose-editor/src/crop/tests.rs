//! Tests of the canvas crop.

// Tests compare exact values, and make test values from small integers.
#![allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use expose_core::buffer::Rgba;

use super::*;
use crate::adjust::{Adjustment, BrightnessContrast};
use crate::composite::composite;
use crate::document::{Background, DocumentSpec, MAX_DIMENSION};
use crate::history::History;
use crate::layer::{GroupIsolation, LayerContent, Locks, MaskFill, Placement};

fn empty_doc(w: u32, h: u32) -> Document {
    let mut spec = DocumentSpec::new(w, h);
    spec.background = Background::Transparent;
    Document::new_empty(spec).unwrap()
}

fn add_pixels(doc: &mut Document, image: WorkingImage, offset: (i32, i32)) -> LayerId {
    let content = LayerContent::Pixel(PixelLayer::new(image, offset).unwrap());
    let at = Placement::root(doc.layers().len());
    doc.add_layer("Paint", content, at).unwrap().0
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

/// A mask with a different value at each pixel.
fn mask_pattern(w: u32, h: u32, offset: (i32, i32), fill: MaskFill) -> Mask {
    let data = (0..w * h).map(|i| (i % 7) as f32 / 7.0).collect();
    Mask::new(MaskImage::from_data(w, h, data).unwrap(), offset, fill)
}

fn spec(center: (f64, f64), width: u32, height: u32, angle: f64, delete: bool) -> CropSpec {
    CropSpec {
        center,
        width,
        height,
        angle,
        delete_cropped: delete,
        interpolation: Interpolation::Bilinear,
    }
}

fn pixel_layer(doc: &Document, id: LayerId) -> (WorkingImage, (i32, i32)) {
    let p = doc.layer(id).unwrap().as_pixel().unwrap();
    (p.image().clone(), p.offset())
}

fn bits(image: &WorkingImage) -> Vec<u32> {
    image
        .pixels()
        .iter()
        .flatten()
        .map(|v| v.to_bits())
        .collect()
}

/// The composite goes through the blending space, so it is equal to the
/// layer values only within float tolerance.
fn near(a: Rgba, b: Rgba) -> bool {
    a.iter()
        .zip(b)
        .all(|(x, y)| (x - y).abs() <= 1e-5 * y.abs().max(1.0))
}

fn assert_images_near(a: &WorkingImage, b: &WorkingImage) {
    assert_eq!((a.width(), a.height()), (b.width(), b.height()));
    let bad = a
        .pixels()
        .iter()
        .zip(b.pixels())
        .position(|(p, q)| !near(*p, *q));
    assert!(bad.is_none(), "first difference at index {bad:?}");
}

// ---------------------------------------------------------------
// Move-only path.
// ---------------------------------------------------------------

#[test]
fn whole_pixel_crop_changes_only_offsets() {
    let mut doc = empty_doc(64, 64);
    let id = add_pixels(&mut doc, pattern(64, 64), (0, 0));
    doc.set_mask(id, Some(mask_pattern(10, 10, (2, 3), MaskFill::White)))
        .unwrap();
    let layer = doc.layer(id).unwrap();
    let (pv, mv) = (layer.pixel_version(), layer.mask_version());
    let rev = doc.revision();

    doc.crop_canvas(&spec((30.0, 20.0), 20, 10, 0.0, false))
        .unwrap();
    assert_eq!((doc.width(), doc.height()), (20, 10));
    assert_ne!(doc.revision(), rev);
    let (image, offset) = pixel_layer(&doc, id);
    assert_eq!(offset, (-20, -15));
    assert_eq!(bits(&image), bits(&pattern(64, 64)));
    let layer = doc.layer(id).unwrap();
    assert_eq!(layer.pixel_version(), pv);
    assert_eq!(layer.mask_version(), mv);
    assert_eq!(layer.mask().unwrap().offset, (-18, -12));

    // The composite is the cropped part of the pattern (mask disabled).
    doc.set_mask_enabled(id, false).unwrap();
    let out = composite(&doc, doc.bounds()).unwrap();
    let expected = pattern(64, 64).crop(Rect::new(20, 15, 20, 10)).unwrap();
    assert_images_near(&out, &expected);
}

#[test]
fn whole_pixel_crop_with_delete_trims_buffers() {
    let mut doc = empty_doc(64, 64);
    let id = add_pixels(&mut doc, pattern(64, 64), (0, 0));
    doc.set_mask(id, Some(mask_pattern(30, 30, (10, 10), MaskFill::Black)))
        .unwrap();
    let pv = doc.layer(id).unwrap().pixel_version();
    doc.crop_canvas(&spec((30.0, 20.0), 20, 10, 0.0, true))
        .unwrap();
    let (image, offset) = pixel_layer(&doc, id);
    assert_eq!(offset, (0, 0));
    let expected = pattern(64, 64).crop(Rect::new(20, 15, 20, 10)).unwrap();
    assert_eq!(bits(&image), bits(&expected));
    assert_ne!(doc.layer(id).unwrap().pixel_version(), pv);

    // The mask (canvas 10..40) after the shift is -10..20 x -5..25; the
    // part on the canvas is 0..20 x 0..10.
    let mask = doc.layer(id).unwrap().mask().unwrap();
    assert_eq!(mask.offset, (0, 0));
    assert_eq!((mask.image.width(), mask.image.height()), (20, 10));
    let old = mask_pattern(30, 30, (10, 10), MaskFill::Black);
    for y in 0..10 {
        for x in 0..20 {
            let canvas = (i64::from(x) + 20, i64::from(y) + 15);
            assert_eq!(
                mask.image.get(x, y).unwrap(),
                old.value_at(canvas.0, canvas.1)
            );
        }
    }
}

#[test]
fn hidden_pixels_come_back_after_a_move() {
    let mut doc = empty_doc(64, 64);
    let id = add_pixels(&mut doc, pattern(64, 64), (0, 0));
    doc.crop_canvas(&spec((30.0, 20.0), 20, 10, 0.0, false))
        .unwrap();
    doc.translate_layer(id, 5, 0).unwrap();
    let out = composite(&doc, doc.bounds()).unwrap();
    // New (0, 0) shows old canvas (15, 15).
    assert!(near(
        out.pixel(0, 0).unwrap(),
        pattern(64, 64).pixel(15, 15).unwrap()
    ));

    // With delete, the area is transparent after the same move.
    let mut doc = empty_doc(64, 64);
    let id = add_pixels(&mut doc, pattern(64, 64), (0, 0));
    doc.crop_canvas(&spec((30.0, 20.0), 20, 10, 0.0, true))
        .unwrap();
    doc.translate_layer(id, 5, 0).unwrap();
    let out = composite(&doc, doc.bounds()).unwrap();
    assert_eq!(out.pixel(0, 0).unwrap(), [0.0; 4]);
}

#[test]
fn crop_larger_than_canvas_adds_transparent_area() {
    for delete in [false, true] {
        let mut doc = empty_doc(8, 8);
        let id = add_pixels(&mut doc, pattern(8, 8), (0, 0));
        let pv = doc.layer(id).unwrap().pixel_version();
        doc.crop_canvas(&spec((4.0, 4.0), 16, 16, 0.0, delete))
            .unwrap();
        let (image, offset) = pixel_layer(&doc, id);
        assert_eq!(offset, (4, 4));
        assert_eq!(bits(&image), bits(&pattern(8, 8)));
        // The buffer is fully on the new canvas: no trim.
        assert_eq!(doc.layer(id).unwrap().pixel_version(), pv);
        let out = composite(&doc, doc.bounds()).unwrap();
        assert_eq!(out.pixel(0, 0).unwrap(), [0.0; 4]);
        assert!(near(
            out.pixel(4, 4).unwrap(),
            pattern(8, 8).pixel(0, 0).unwrap()
        ));
    }
}

#[test]
fn layer_outside_the_new_canvas_becomes_one_pixel_with_delete() {
    let mut doc = empty_doc(64, 64);
    let id = add_pixels(&mut doc, pattern(4, 4), (50, 50));
    doc.set_mask(id, Some(mask_pattern(4, 4, (50, 50), MaskFill::Black)))
        .unwrap();
    doc.crop_canvas(&spec((5.0, 5.0), 10, 10, 0.0, true))
        .unwrap();
    let (image, offset) = pixel_layer(&doc, id);
    assert_eq!(offset, (0, 0));
    assert_eq!((image.width(), image.height()), (1, 1));
    assert_eq!(image.pixel(0, 0).unwrap(), [0.0; 4]);
    let mask = doc.layer(id).unwrap().mask().unwrap();
    assert_eq!((mask.image.width(), mask.image.height()), (1, 1));
    assert_eq!(mask.image.get(0, 0), Some(0.0));
    assert_eq!(mask.fill, MaskFill::Black);
}

// ---------------------------------------------------------------
// Resample path.
// ---------------------------------------------------------------

/// A 2 px wide line through `(32, 32)` with the direction `angle`
/// (degrees, clockwise on the screen).
fn line_image(angle: f64) -> WorkingImage {
    let (s, c) = angle.to_radians().sin_cos();
    let pixels = (0..64 * 64)
        .map(|i| {
            let dx = f64::from(i % 64) + 0.5 - 32.0;
            let dy = f64::from(i / 64) + 0.5 - 32.0;
            // A soft edge (alpha ramps over 1 px), so that the line has
            // little aliasing. The alpha integral across the line is 2.
            let a = (1.5 - (-s * dx + c * dy).abs()).clamp(0.0, 1.0) as f32;
            [a, a, a, a]
        })
        .collect();
    WorkingImage::from_pixels(64, 64, pixels).unwrap()
}

#[test]
fn straightened_crop_makes_a_turned_line_level() {
    for angle in [10.0, -7.5] {
        for delete in [false, true] {
            let mut doc = empty_doc(64, 64);
            add_pixels(&mut doc, line_image(angle), (0, 0));
            let pv = doc.layers()[0].pixel_version();
            doc.crop_canvas(&spec((32.0, 32.0), 40, 20, angle, delete))
                .unwrap();
            assert_ne!(doc.layers()[0].pixel_version(), pv);
            let out = composite(&doc, doc.bounds()).unwrap();
            // The center of one column can be off by a small part of a
            // pixel (sampling). The mean of all columns is near the exact
            // value.
            let mut centers = Vec::new();
            for u in 3..37 {
                let (mut sum, mut weight) = (0.0, 0.0);
                for v in 0..20 {
                    let a = f64::from(out.pixel(u, v).unwrap()[3]);
                    sum += a * (f64::from(v) + 0.5);
                    weight += a;
                    if !(7..13).contains(&v) {
                        assert!(a < 0.05, "angle {angle}: alpha {a} at {u},{v}");
                    }
                }
                // The line is 2 px wide: the summed alpha is about 2.
                assert!((weight - 2.0).abs() < 0.3, "weight {weight} at {u}");
                let center = sum / weight;
                assert!((center - 10.0).abs() < 0.1, "center {center} at {u}");
                centers.push(center);
            }
            let mean = centers.iter().sum::<f64>() / centers.len() as f64;
            assert!((mean - 10.0).abs() < 0.02, "mean center {mean}");
            let (image, _) = pixel_layer(&doc, doc.layers()[0].id());
            if delete {
                assert!(image.width() <= 40 && image.height() <= 20);
            } else {
                // The full turned content stays: more than the canvas.
                assert!(image.width() > 40 && image.height() > 20);
            }
        }
    }
}

#[test]
fn masks_on_all_layer_kinds_move_with_the_crop() {
    let mut doc = empty_doc(64, 64);
    let px = add_pixels(&mut doc, pattern(64, 64), (0, 0));
    let content =
        LayerContent::Adjustment(Adjustment::BrightnessContrast(BrightnessContrast::default()));
    let (adj, _) = doc.add_layer("Adj", content, Placement::root(1)).unwrap();
    let (group, _) = doc
        .add_layer(
            "Group",
            LayerContent::Group(GroupIsolation::Isolated),
            Placement::root(2),
        )
        .unwrap();
    for (i, id) in [px, adj, group].into_iter().enumerate() {
        let at = (2 + i as i32, 3);
        doc.set_mask(id, Some(mask_pattern(10, 10, at, MaskFill::White)))
            .unwrap();
    }

    // Move only: each mask offset moves by (-5, -6), the values stay.
    let mut moved = doc.clone();
    moved
        .crop_canvas(&spec((15.0, 16.0), 20, 20, 0.0, false))
        .unwrap();
    for (i, id) in [px, adj, group].into_iter().enumerate() {
        let m = moved.layer(id).unwrap().mask().unwrap();
        assert_eq!(m.offset, (2 + i as i32 - 5, 3 - 6));
        assert_eq!(m.image, mask_pattern(10, 10, (0, 0), MaskFill::White).image);
    }

    // Turned: each mask gets new values and a new mask version.
    let versions: Vec<u64> = [px, adj, group]
        .iter()
        .map(|id| doc.layer(*id).unwrap().mask_version())
        .collect();
    doc.crop_canvas(&spec((15.0, 16.0), 20, 20, 5.0, false))
        .unwrap();
    for (id, v) in [px, adj, group].into_iter().zip(versions) {
        let layer = doc.layer(id).unwrap();
        assert_ne!(layer.mask_version(), v);
        let m = layer.mask().unwrap();
        // The turned 10 x 10 buffer is larger than 10 x 10.
        assert!(m.image.width() > 10 && m.image.height() > 10);
        assert!(m.image.data().iter().all(|v| (0.0..=1.0).contains(v)));
    }
}

#[test]
fn empty_pixel_layer_only_moves() {
    let mut doc = empty_doc(64, 64);
    let id = add_pixels(&mut doc, WorkingImage::new(64, 64).unwrap(), (0, 0));
    let pv = doc.layer(id).unwrap().pixel_version();
    doc.crop_canvas(&spec((32.0, 32.0), 40, 20, 10.0, false))
        .unwrap();
    let (image, offset) = pixel_layer(&doc, id);
    assert_eq!((image.width(), image.height()), (64, 64));
    // The center (32, 32) maps to the new center (20, 10).
    assert_eq!(offset, (20 - 32, 10 - 32));
    assert_eq!(doc.layer(id).unwrap().pixel_version(), pv);

    // With delete, the buffer is trimmed to the canvas.
    let mut doc = empty_doc(64, 64);
    let id = add_pixels(&mut doc, WorkingImage::new(64, 64).unwrap(), (0, 0));
    doc.crop_canvas(&spec((32.0, 32.0), 40, 20, 10.0, true))
        .unwrap();
    let (image, offset) = pixel_layer(&doc, id);
    assert_eq!(offset, (0, 0));
    assert_eq!((image.width(), image.height()), (40, 20));
}

#[test]
fn fractional_center_resamples() {
    let mut doc = empty_doc(16, 16);
    let id = add_pixels(&mut doc, pattern(16, 16), (0, 0));
    let pv = doc.layer(id).unwrap().pixel_version();
    doc.crop_canvas(&spec((8.5, 8.0), 8, 8, 0.0, true)).unwrap();
    assert_ne!(doc.layer(id).unwrap().pixel_version(), pv);
    let out = composite(&doc, doc.bounds()).unwrap();
    // New pixel (0, 0) samples old point (5, 4.5): the mean of the old
    // pixels (4, 4) and (5, 4).
    let p = pattern(16, 16);
    let (a, b) = (p.pixel(4, 4).unwrap(), p.pixel(5, 4).unwrap());
    let got = out.pixel(0, 0).unwrap();
    for c in 0..4 {
        assert!((got[c] - f32::midpoint(a[c], b[c])).abs() < 1e-4);
    }
}

// ---------------------------------------------------------------
// Selection, locks, undo.
// ---------------------------------------------------------------

#[test]
fn crop_clears_the_selection_and_reselect_state() {
    let mut doc = empty_doc(64, 64);
    add_pixels(&mut doc, pattern(64, 64), (0, 0));
    doc.select_all().unwrap();
    doc.deselect().unwrap();
    assert!(doc.can_reselect());
    doc.select_all().unwrap();
    let old_selection = doc.selection().cloned();
    let version = doc.selection_version();

    let mut history = History::default();
    let edit = doc
        .crop_canvas(&spec((30.0, 20.0), 20, 10, 0.0, false))
        .unwrap();
    history.push("Crop", vec![edit]);
    assert!(doc.selection().is_none());
    assert_ne!(doc.selection_version(), version);
    assert!(!doc.can_reselect());

    assert!(history.undo(&mut doc).unwrap());
    assert_eq!(doc.selection().cloned(), old_selection);
    assert!(history.redo(&mut doc).unwrap());
    assert!(doc.selection().is_none());
}

#[test]
fn crop_ignores_layer_locks() {
    let mut doc = empty_doc(64, 64);
    let id = add_pixels(&mut doc, pattern(64, 64), (0, 0));
    let all = Locks {
        all: true,
        ..Locks::NONE
    };
    doc.set_locks(id, all).unwrap();
    doc.crop_canvas(&spec((30.0, 20.0), 20, 10, 3.0, false))
        .unwrap();
    assert_eq!((doc.width(), doc.height()), (20, 10));
}

#[test]
fn undo_and_redo_are_bit_exact() {
    for (angle, delete) in [(0.0, false), (0.0, true), (6.0, false), (-4.0, true)] {
        let mut doc = empty_doc(64, 64);
        let id = add_pixels(&mut doc, pattern(64, 64), (-3, 2));
        doc.set_mask(id, Some(mask_pattern(20, 20, (5, 5), MaskFill::Black)))
            .unwrap();
        let empty = add_pixels(&mut doc, WorkingImage::new(8, 8).unwrap(), (60, 60));
        let content =
            LayerContent::Adjustment(Adjustment::BrightnessContrast(BrightnessContrast::default()));
        let (adj, _) = doc.add_layer("Adj", content, Placement::root(2)).unwrap();
        doc.set_mask(adj, Some(mask_pattern(12, 9, (40, 1), MaskFill::White)))
            .unwrap();
        doc.select_all().unwrap();
        let before = doc.clone();

        let mut history = History::default();
        let edit = doc
            .crop_canvas(&spec((31.0, 22.0), 30, 24, angle, delete))
            .unwrap();
        history.push("Crop", vec![edit]);
        let after = doc.clone();
        assert!(history.undo(&mut doc).unwrap());
        assert_eq!(doc, before);
        assert_eq!(
            bits(&pixel_layer(&doc, id).0),
            bits(&pixel_layer(&before, id).0)
        );
        assert!(history.redo(&mut doc).unwrap());
        assert_eq!(doc, after);
        assert_eq!(
            bits(&pixel_layer(&doc, empty).0),
            bits(&pixel_layer(&after, empty).0)
        );
        assert!(history.memory_bytes() > 0);
    }
}

// ---------------------------------------------------------------
// Errors.
// ---------------------------------------------------------------

#[test]
fn bad_specs_are_refused_and_change_nothing() {
    let mut doc = empty_doc(64, 64);
    add_pixels(&mut doc, pattern(64, 64), (0, 0));
    let before = doc.clone();
    let rev = doc.revision();
    let cases = [
        spec((f64::NAN, 0.0), 10, 10, 0.0, false),
        spec((0.0, f64::INFINITY), 10, 10, 0.0, false),
        spec((0.0, MAX_CROP_COORD * 2.0), 10, 10, 0.0, false),
        spec((0.0, 0.0), 10, 10, 45.5, false),
        spec((0.0, 0.0), 10, 10, f64::NAN, false),
        spec((0.0, 0.0), 0, 10, 0.0, false),
        spec((0.0, 0.0), MAX_DIMENSION + 1, 10, 0.0, false),
    ];
    let expected = [
        |e: &EditorError| matches!(e, EditorError::NonFinite { .. }),
        |e: &EditorError| matches!(e, EditorError::NonFinite { .. }),
        |e: &EditorError| matches!(e, EditorError::OutOfRange { .. }),
        |e: &EditorError| matches!(e, EditorError::OutOfRange { .. }),
        |e: &EditorError| matches!(e, EditorError::NonFinite { .. }),
        |e: &EditorError| matches!(e, EditorError::Core(_)),
        |e: &EditorError| matches!(e, EditorError::TooLarge { .. }),
    ];
    for (s, check) in cases.iter().zip(expected) {
        let err = doc.crop_canvas(s).unwrap_err();
        assert!(check(&err), "{s:?}: {err:?}");
        assert_eq!(doc, before);
        assert_eq!(doc.revision(), rev);
    }
    // The limits are inclusive.
    doc.crop_canvas(&spec((32.0, 32.0), 64, 64, MAX_CROP_ANGLE, false))
        .unwrap();
}

#[test]
fn offset_overflow_is_refused() {
    let mut doc = empty_doc(64, 64);
    add_pixels(&mut doc, pattern(4, 4), (i32::MAX - 10, 0));
    let before = doc.clone();
    let err = doc
        .crop_canvas(&spec((-1000.0, 32.0), 64, 64, 0.0, false))
        .unwrap_err();
    assert_eq!(err, EditorError::OffsetOverflow);
    assert_eq!(doc, before);
}

#[test]
fn turned_crop_of_a_too_large_layer_is_refused_without_delete() {
    // A MAX_DIMENSION x 2 layer turned by 45 degrees needs about
    // 21000 x 21000 pixels, more than MAX_PIXELS. With delete, only the
    // canvas part is made.
    let mut doc = empty_doc(64, 64);
    add_pixels(
        &mut doc,
        WorkingImage::new_filled(MAX_DIMENSION, 2, [1.0; 4]).unwrap(),
        (-15_000, 31),
    );
    let err = doc
        .crop_canvas(&spec((32.0, 32.0), 64, 64, 45.0, false))
        .unwrap_err();
    assert!(matches!(err, EditorError::TooLarge { .. }), "{err:?}");
    doc.crop_canvas(&spec((32.0, 32.0), 64, 64, 45.0, true))
        .unwrap();
    let (image, _) = pixel_layer(&doc, doc.layers()[0].id());
    assert_eq!((image.width(), image.height()), (64, 64));
}

// ---------------------------------------------------------------
// Timing.
// ---------------------------------------------------------------

/// Run with `cargo test -p expose-editor --release -- --ignored
/// crop_timing --nocapture`.
#[test]
#[ignore = "timing test, run in release mode"]
fn crop_timing_4096() {
    let size = 4096;
    let mut doc = empty_doc(size, size);
    add_pixels(&mut doc, pattern(size, size), (0, 0));
    for (interp, delete) in [
        (Interpolation::Bicubic, true),
        (Interpolation::Bicubic, false),
        (Interpolation::Bilinear, true),
    ] {
        let mut d = doc.clone();
        let mut s = spec((2048.0, 2048.0), 3800, 3800, 3.0, delete);
        s.interpolation = interp;
        let start = std::time::Instant::now();
        d.crop_canvas(&s).unwrap();
        println!(
            "crop 4096^2 -> 3800^2, 3 deg, {interp:?}, delete {delete}: {:?}",
            start.elapsed()
        );
    }
}
