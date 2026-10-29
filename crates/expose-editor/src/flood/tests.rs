//! Tests of the flood region and the fill.

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
use crate::brush::{BrushSettings, Stroke, StrokePoint, StrokeTarget};
use crate::document::{Background, DocumentSpec};
use crate::history::History;
use crate::layer::{LayerContent, Locks, PixelLayer, Placement};
use crate::selection::SelectionShape;

const A: Rgba = [0.5, 0.5, 0.5, 1.0];
const B: Rgba = [0.0, 0.0, 1.0, 1.0];

fn doc_with_layer(
    canvas: (u32, u32),
    size: (u32, u32),
    fill: Rgba,
    offset: (i32, i32),
) -> (Document, LayerId) {
    let mut spec = DocumentSpec::new(canvas.0, canvas.1);
    spec.background = Background::Transparent;
    let mut doc = Document::new(spec).unwrap();
    let image = WorkingImage::new_filled(size.0, size.1, fill).unwrap();
    let content = LayerContent::Pixel(PixelLayer::new(image, offset).unwrap());
    let (id, _) = doc.add_layer("Paint", content, Placement::root(1)).unwrap();
    (doc, id)
}

/// A document with one layer at (0, 0) of the canvas size with `pixels`
/// (row-major).
fn doc_from(w: u32, h: u32, pixels: &[Rgba]) -> (Document, LayerId) {
    let (mut doc, id) = doc_with_layer((w, h), (w, h), [0.0; 4], (0, 0));
    doc.write_pixels(id, Rect::new(0, 0, w, h), pixels).unwrap();
    (doc, id)
}

fn settings(tolerance: f32, contiguous: bool, anti_alias: bool) -> FloodSettings {
    FloodSettings {
        tolerance,
        contiguous,
        anti_alias,
    }
}

fn region(doc: &Document, id: LayerId, x: u32, y: u32, s: FloodSettings) -> Vec<f32> {
    let sample = FloodSample::layer(doc, id).unwrap();
    flood_region(&sample, x, y, &s).unwrap().data().to_vec()
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

fn full(doc: &Document, value: f32) -> MaskImage {
    MaskImage::new_filled(doc.width(), doc.height(), value).unwrap()
}

fn fill(color: [f32; 3], opacity: f32, blend_mode: BlendMode) -> FillSettings {
    FillSettings {
        color,
        opacity,
        blend_mode,
    }
}

#[test]
fn settings_defaults_and_validation() {
    let s = FloodSettings::default();
    assert_eq!(s, settings(32.0, true, true));
    assert!(s.validate().is_ok());
    assert!(settings(255.0, true, true).validate().is_ok());
    assert!(matches!(
        settings(-1.0, true, true).validate(),
        Err(EditorError::OutOfRange { .. })
    ));
    assert!(matches!(
        settings(256.0, true, true).validate(),
        Err(EditorError::OutOfRange { .. })
    ));
    assert!(matches!(
        settings(f32::NAN, true, true).validate(),
        Err(EditorError::NonFinite { .. })
    ));
}

#[test]
fn keys_use_the_srgb_encoding() {
    let white = [1.0, 1.0, 1.0, 1.0];
    let faint_white = [0.4, 0.4, 0.4, 0.4];
    let (doc, id) = doc_from(3, 1, &[white, faint_white, [0.0; 4]]);
    let s = FloodSample::layer(&doc, id).unwrap();
    assert_eq!(s.key(0, 0), [255; 4]);
    // Straight white with alpha 0.4: 255 * 1 * 0.4 = 102.
    assert_eq!(s.key(1, 0), [102; 4]);
    assert_eq!(s.key(2, 0), [0; 4]);
    // Linear 0.5 encodes to about 0.7354 in sRGB: 187.5 -> 188.
    let (doc, id) = doc_from(1, 1, &[A]);
    let s = FloodSample::layer(&doc, id).unwrap();
    assert_eq!(s.key(0, 0), [188, 188, 188, 255]);
}

#[test]
fn tolerance_zero_and_max() {
    let c = [0.52, 0.5, 0.5, 1.0];
    let (doc, id) = doc_from(4, 1, &[A, A, c, A]);
    let sample = FloodSample::layer(&doc, id).unwrap();
    let ka = sample.key(0, 0);
    let kc = sample.key(2, 0);
    let diff = ka
        .iter()
        .zip(&kc)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(diff > 1);
    let d = f32::from(diff);
    assert_eq!(
        region(&doc, id, 0, 0, settings(0.0, true, false)),
        [1.0, 1.0, 0.0, 0.0]
    );
    assert_eq!(
        region(&doc, id, 0, 0, settings(d - 1.0, true, false)),
        [1.0, 1.0, 0.0, 0.0]
    );
    assert_eq!(region(&doc, id, 0, 0, settings(d, true, false)), [1.0; 4]);
    // The fraction of the tolerance has no effect (integer keys).
    assert_eq!(
        region(&doc, id, 0, 0, settings(d - 0.5, true, false)),
        [1.0, 1.0, 0.0, 0.0]
    );
    let (doc, id) = doc_from(3, 1, &[[0.0; 4], [1.0; 4], B]);
    assert_eq!(
        region(&doc, id, 0, 0, settings(255.0, true, false)),
        [1.0; 3]
    );
}

#[test]
fn contiguous_and_global() {
    let (doc, id) = doc_from(5, 1, &[A, B, A, B, A]);
    assert_eq!(
        region(&doc, id, 0, 0, settings(0.0, true, false)),
        [1.0, 0.0, 0.0, 0.0, 0.0]
    );
    assert_eq!(
        region(&doc, id, 2, 0, settings(0.0, false, false)),
        [1.0, 0.0, 1.0, 0.0, 1.0]
    );
    assert_eq!(
        region(&doc, id, 1, 0, settings(0.0, false, false)),
        [0.0, 1.0, 0.0, 1.0, 0.0]
    );
}

#[test]
fn diagonal_pixels_are_not_connected() {
    let (doc, id) = doc_from(3, 3, &[A, B, B, B, A, B, B, B, A]);
    let r = region(&doc, id, 0, 0, settings(0.0, true, false));
    assert_eq!(r, [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    let r = region(&doc, id, 1, 1, settings(0.0, false, false));
    assert_eq!(r, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn contiguous_fill_follows_a_spiral() {
    // A path of A that turns back and forth: the scanline fill must find
    // all of it.
    #[rustfmt::skip]
    let map = [
        1, 1, 1, 1, 1,
        0, 0, 0, 0, 1,
        1, 1, 1, 0, 1,
        1, 0, 1, 1, 1,
        1, 0, 0, 0, 0,
    ];
    let pixels: Vec<Rgba> = map.iter().map(|&m| if m == 1 { A } else { B }).collect();
    let (doc, id) = doc_from(5, 5, &pixels);
    let r = region(&doc, id, 0, 0, settings(0.0, true, false));
    let expected: Vec<f32> = map.iter().map(|&m| m as f32).collect();
    assert_eq!(r, expected);
    // The same from the other end.
    let r = region(&doc, id, 0, 4, settings(0.0, true, false));
    assert_eq!(r, expected);
}

#[test]
fn anti_alias_ring_values() {
    let mut pixels = vec![B; 49];
    for y in 2..5 {
        for x in 2..5 {
            pixels[y * 7 + x] = A;
        }
    }
    let (doc, id) = doc_from(7, 7, &pixels);
    let r = region(&doc, id, 3, 3, settings(0.0, true, true));
    let at = |x: usize, y: usize| r[y * 7 + x];
    for y in 2..5 {
        for x in 2..5 {
            assert_eq!(at(x, y), 1.0);
        }
    }
    assert_eq!(at(1, 1), 1.0 / 9.0);
    assert_eq!(at(1, 2), 2.0 / 9.0);
    assert_eq!(at(1, 3), 3.0 / 9.0);
    assert_eq!(at(3, 1), 3.0 / 9.0);
    assert_eq!(at(5, 5), 1.0 / 9.0);
    assert_eq!(at(5, 4), 2.0 / 9.0);
    // Only the 1 px ring gets values.
    for i in 0..7 {
        assert_eq!(at(0, i), 0.0);
        assert_eq!(at(6, i), 0.0);
        assert_eq!(at(i, 0), 0.0);
        assert_eq!(at(i, 6), 0.0);
    }
    // At the canvas edge, pixels outside the canvas count as 0.
    let (doc, id) = doc_from(3, 1, &[A, B, B]);
    let r = region(&doc, id, 0, 0, settings(0.0, true, true));
    assert_eq!(r, [1.0, 1.0 / 9.0, 0.0]);
    // The same region without anti-alias is binary.
    let r = region(&doc, id, 0, 0, settings(0.0, true, false));
    assert_eq!(r, [1.0, 0.0, 0.0]);
}

#[test]
fn transparent_pixels_match_each_other() {
    // Different color values with alpha 0 all give the key 0.
    let pixels = [[0.0; 4], [0.3, 0.0, 0.0, 0.0], A, [0.0, -1.0, 0.2, 0.0]];
    let (doc, id) = doc_from(4, 1, &pixels);
    let r = region(&doc, id, 0, 0, settings(0.0, false, false));
    assert_eq!(r, [1.0, 1.0, 0.0, 1.0]);
}

#[test]
fn layer_sample_with_offset() {
    let red = [1.0, 0.0, 0.0, 1.0];
    let (doc, id) = doc_with_layer((8, 8), (4, 4), red, (-2, 3));
    let s = FloodSample::layer(&doc, id).unwrap();
    assert_eq!((s.width(), s.height()), (8, 8));
    let k = s.key(0, 3);
    assert_ne!(k, [0; 4]);
    for (x, y) in [(0, 3), (1, 3), (0, 6), (1, 6)] {
        assert_eq!(s.key(x, y), k);
    }
    for (x, y) in [(2, 3), (0, 2), (0, 7), (7, 7)] {
        assert_eq!(s.key(x, y), [0; 4]);
    }
    // A layer that is larger than the canvas.
    let (doc, id) = doc_with_layer((4, 4), (10, 10), red, (-3, -3));
    let s = FloodSample::layer(&doc, id).unwrap();
    assert!((0..4).all(|y| (0..4).all(|x| s.key(x, y) == k)));
    // A layer outside the canvas.
    let (doc, id) = doc_with_layer((4, 4), (2, 2), red, (10, 0));
    let s = FloodSample::layer(&doc, id).unwrap();
    assert!((0..4).all(|y| (0..4).all(|x| s.key(x, y) == [0; 4])));
}

#[test]
fn sample_errors() {
    let (mut doc, id) = doc_with_layer((4, 4), (4, 4), A, (0, 0));
    let missing = LayerId(9999);
    assert_eq!(
        FloodSample::layer(&doc, missing).unwrap_err(),
        EditorError::LayerNotFound(missing)
    );
    let content =
        LayerContent::Adjustment(Adjustment::BrightnessContrast(BrightnessContrast::default()));
    let (adj, _) = doc.add_layer("Adj", content, Placement::root(2)).unwrap();
    assert_eq!(
        FloodSample::layer(&doc, adj).unwrap_err(),
        EditorError::NotAPixelLayer(adj)
    );
    let wrong = WorkingImage::new(4, 3).unwrap();
    assert!(matches!(
        FloodSample::from_composite_image(&doc, &wrong),
        Err(EditorError::InvalidRect { .. })
    ));
    let s = FloodSample::layer(&doc, id).unwrap();
    for (x, y) in [(4, 0), (0, 4)] {
        assert!(matches!(
            flood_region(&s, x, y, &FloodSettings::default()),
            Err(EditorError::OutOfRange { .. })
        ));
    }
}

#[test]
fn composite_sample_equals_layer_sample_for_one_opaque_layer() {
    let (doc, id) = doc_from(3, 1, &[A, B, [0.2, 0.1, 0.05, 1.0]]);
    let c = FloodSample::composite(&doc).unwrap();
    let l = FloodSample::layer(&doc, id).unwrap();
    assert_eq!(c, l);
}

#[test]
fn select_region_modes_and_history() {
    let (mut doc, id) = doc_from(4, 1, &[A, A, B, B]);
    let left = flood_region(
        &FloodSample::layer(&doc, id).unwrap(),
        0,
        0,
        &settings(0.0, true, false),
    )
    .unwrap();
    let right = MaskImage::from_data(4, 1, vec![0.0, 1.0, 1.0, 0.5]).unwrap();
    let sel = |d: &Document| d.selection().unwrap().data().to_vec();

    // Without a selection, subtract and intersect give an empty selection.
    for mode in [SelectionMode::Subtract, SelectionMode::Intersect] {
        let mut d = doc.clone();
        d.select_region(&left, mode).unwrap();
        assert_eq!(sel(&d), [0.0; 4]);
    }

    let mut history = History::default();
    let mut states = vec![doc.clone()];
    let steps: [(&MaskImage, SelectionMode, [f32; 4]); 4] = [
        (&left, SelectionMode::Replace, [1.0, 1.0, 0.0, 0.0]),
        (&right, SelectionMode::Add, [1.0, 1.0, 1.0, 0.5]),
        (&right, SelectionMode::Subtract, [1.0, 0.0, 0.0, 0.25]),
        (&left, SelectionMode::Intersect, [1.0, 0.0, 0.0, 0.0]),
    ];
    let revision = doc.revision();
    for (region, mode, expected) in steps {
        let edit = doc.select_region(region, mode).unwrap();
        history.push("Magic wand", vec![edit]);
        assert_eq!(sel(&doc), expected);
        states.push(doc.clone());
    }
    assert_eq!(doc.revision(), revision);
    for i in (0..4).rev() {
        assert!(history.undo(&mut doc).unwrap());
        assert_eq!(doc, states[i], "undo to {i}");
    }
    for (i, state) in states.iter().enumerate().skip(1) {
        assert!(history.redo(&mut doc).unwrap());
        assert_eq!(&doc, state, "redo to {i}");
    }

    let wrong = MaskImage::new_filled(3, 1, 1.0).unwrap();
    assert!(matches!(
        doc.select_region(&wrong, SelectionMode::Replace),
        Err(EditorError::InvalidRect { .. })
    ));
    let nan = MaskImage::from_data(4, 1, vec![f32::NAN, 0.0, 0.0, 0.0]).unwrap();
    assert!(matches!(
        doc.select_region(&nan, SelectionMode::Replace),
        Err(EditorError::NonFinite { .. })
    ));
}

#[test]
fn fill_equals_brush_paint() {
    let orig = [0.2, 0.3, 0.1, 0.6];
    let color = [1.0, 0.5, 0.0];
    for mode in [BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen] {
        for keep_alpha in [false, true] {
            let locks = Locks {
                transparency: keep_alpha,
                ..Locks::NONE
            };
            let (mut a, id_a) = doc_with_layer((16, 16), (16, 16), orig, (0, 0));
            a.set_locks(id_a, locks).unwrap();
            let region = full(&a, 1.0);
            a.fill_region(id_a, &region, &fill(color, 0.7, mode))
                .unwrap()
                .unwrap();

            let (mut b, id_b) = doc_with_layer((16, 16), (16, 16), orig, (0, 0));
            b.set_locks(id_b, locks).unwrap();
            let brush = BrushSettings {
                size: 100.0,
                opacity: 0.7,
                blend_mode: mode,
                ..BrushSettings::default()
            };
            let mut s = Stroke::begin(&b, id_b, StrokeTarget::Paint { color }, brush).unwrap();
            s.add_points(&mut b, &[StrokePoint::new(8.0, 8.0, 1.0)])
                .unwrap();
            s.end(&mut b).unwrap();
            assert_eq!(bits(&a, id_a), bits(&b, id_b), "{mode:?} {keep_alpha}");
        }
    }
}

#[test]
fn fill_normal_math_on_one_pixel() {
    let o = [0.2, 0.3, 0.1, 0.6];
    let (mut doc, id) = doc_with_layer((2, 1), (2, 1), o, (0, 0));
    let region = MaskImage::from_data(2, 1, vec![1.0, 0.5]).unwrap();
    let (_, rect) = doc
        .fill_region(id, &region, &fill([1.0, 0.0, 0.0], 0.5, BlendMode::Normal))
        .unwrap()
        .unwrap();
    assert_eq!(rect, Rect::new(0, 0, 2, 1));
    let expect = |c: f32| {
        let k = 1.0 - c;
        [c + o[0] * k, o[1] * k, o[2] * k, c + o[3] * k]
    };
    assert_eq!(px(&doc, id, 0, 0), expect(0.5));
    assert_eq!(px(&doc, id, 1, 0), expect(0.25));
}

#[test]
fn fill_uses_the_selection() {
    let o = [0.0, 0.0, 0.0, 1.0];
    let (mut doc, id) = doc_with_layer((4, 1), (4, 1), o, (0, 0));
    let shape = SelectionShape::Rect {
        x: 0.0,
        y: 0.0,
        width: 2.5,
        height: 1.0,
    };
    doc.select_shape(&shape, SelectionMode::Replace, true, 0.0)
        .unwrap();
    let region = full(&doc, 1.0);
    let (_, rect) = doc
        .fill_region(id, &region, &fill([1.0, 1.0, 1.0], 1.0, BlendMode::Normal))
        .unwrap()
        .unwrap();
    assert_eq!(rect, Rect::new(0, 0, 3, 1));
    assert_eq!(px(&doc, id, 0, 0), [1.0; 4]);
    assert_eq!(px(&doc, id, 1, 0), [1.0; 4]);
    assert_eq!(px(&doc, id, 2, 0), [0.5, 0.5, 0.5, 1.0]);
    assert_eq!(px(&doc, id, 3, 0), o);
}

#[test]
fn fill_with_transparency_lock_keeps_alpha() {
    let (mut doc, id) = doc_from(2, 1, &[[0.25, 0.25, 0.25, 0.5], [0.0; 4]]);
    let locks = Locks {
        transparency: true,
        ..Locks::NONE
    };
    doc.set_locks(id, locks).unwrap();
    let region = full(&doc, 1.0);
    let (_, rect) = doc
        .fill_region(id, &region, &fill([1.0, 0.0, 0.0], 1.0, BlendMode::Normal))
        .unwrap()
        .unwrap();
    assert_eq!(rect, Rect::new(0, 0, 1, 1));
    assert_eq!(px(&doc, id, 0, 0), [0.5, 0.0, 0.0, 0.5]);
    assert_eq!(px(&doc, id, 1, 0), [0.0; 4]);
}

#[test]
fn fill_errors() {
    let (mut doc, id) = doc_with_layer((4, 4), (4, 4), A, (0, 0));
    let region = full(&doc, 1.0);
    let red = fill([1.0, 0.0, 0.0], 1.0, BlendMode::Normal);
    let missing = LayerId(9999);
    assert_eq!(
        doc.fill_region(missing, &region, &red).unwrap_err(),
        EditorError::LayerNotFound(missing)
    );
    let content =
        LayerContent::Adjustment(Adjustment::BrightnessContrast(BrightnessContrast::default()));
    let (adj, _) = doc.add_layer("Adj", content, Placement::root(2)).unwrap();
    assert_eq!(
        doc.fill_region(adj, &region, &red).unwrap_err(),
        EditorError::NotAPixelLayer(adj)
    );
    assert!(matches!(
        doc.fill_region(
            id,
            &region,
            &fill([1.0, 0.0, 0.0], 1.0, BlendMode::Dissolve)
        ),
        Err(EditorError::InvalidFill(_))
    ));
    assert!(matches!(
        doc.fill_region(
            id,
            &region,
            &fill([f32::NAN, 0.0, 0.0], 1.0, BlendMode::Normal)
        ),
        Err(EditorError::NonFinite { .. })
    ));
    assert!(matches!(
        doc.fill_region(id, &region, &fill([1.0, 0.0, 0.0], 1.5, BlendMode::Normal)),
        Err(EditorError::OutOfRange { .. })
    ));
    let wrong = MaskImage::new_filled(4, 5, 1.0).unwrap();
    assert!(matches!(
        doc.fill_region(id, &wrong, &red),
        Err(EditorError::InvalidRect { .. })
    ));
    let locks = Locks {
        pixels: true,
        ..Locks::NONE
    };
    doc.set_locks(id, locks).unwrap();
    assert_eq!(
        doc.fill_region(id, &region, &red).unwrap_err(),
        EditorError::Locked {
            id,
            lock: LockKind::Pixels
        }
    );
    assert_eq!(px(&doc, id, 0, 0), A);
}

#[test]
fn fill_does_not_grow_the_layer() {
    let (mut doc, id) = doc_with_layer((8, 8), (4, 4), [0.0; 4], (6, -2));
    let region = full(&doc, 1.0);
    let (_, rect) = doc
        .fill_region(id, &region, &fill([1.0, 0.0, 0.0], 1.0, BlendMode::Normal))
        .unwrap()
        .unwrap();
    // The layer covers canvas x 6..8, y 0..2 (layer x 0..2, y 2..4).
    assert_eq!(rect, Rect::new(6, 0, 2, 2));
    let p = doc.layer(id).unwrap().as_pixel().unwrap();
    assert_eq!((p.image().width(), p.image().height()), (4, 4));
    assert_eq!(p.offset(), (6, -2));
    assert_eq!(px(&doc, id, 0, 2), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(px(&doc, id, 1, 3), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(px(&doc, id, 2, 2), [0.0; 4]);
    assert_eq!(px(&doc, id, 0, 1), [0.0; 4]);
    // A layer outside the canvas does not change.
    let (mut doc, id) = doc_with_layer((8, 8), (4, 4), [0.0; 4], (20, 0));
    let region = full(&doc, 1.0);
    let result = doc
        .fill_region(id, &region, &fill([1.0, 0.0, 0.0], 1.0, BlendMode::Normal))
        .unwrap();
    assert!(result.is_none());
}

#[test]
fn undo_restores_the_exact_values() {
    // A layer of more than one tile, with a pattern of values.
    let (w, h) = (150, 140);
    let pixels: Vec<Rgba> = (0..w * h)
        .map(|i| {
            let v = (i % 17) as f32 / 17.0;
            [v * 0.5, 0.1, v * 0.25, 0.5 + v * 0.5]
        })
        .collect();
    let (mut doc, id) = doc_from(w, h, &pixels);
    // A region in the middle tiles only.
    let mut data = vec![0.0; (w * h) as usize];
    for y in 70..100 {
        for x in 65..130 {
            data[(y * w + x) as usize] = 0.75;
        }
    }
    let region = MaskImage::from_data(w, h, data).unwrap();
    let before = bits(&doc, id);
    let version = doc.layer(id).unwrap().pixel_version();
    let revision = doc.revision();
    let (edit, rect) = doc
        .fill_region(id, &region, &fill([0.9, 0.2, 0.4], 0.8, BlendMode::Overlay))
        .unwrap()
        .unwrap();
    assert_eq!(rect, Rect::new(65, 70, 65, 30));
    assert_ne!(doc.layer(id).unwrap().pixel_version(), version);
    assert_ne!(doc.revision(), revision);
    let after = bits(&doc, id);
    assert_ne!(after, before);
    // The edit holds only the touched tiles (64 px): x 64..150, y 64..128.
    assert!(edit.memory_bytes() < 3 * 2 * 64 * 64 * 16 + 4096);
    let mut history = History::default();
    history.push("Paint bucket", vec![edit]);
    assert!(history.undo(&mut doc).unwrap());
    assert_eq!(bits(&doc, id), before);
    assert!(history.redo(&mut doc).unwrap());
    assert_eq!(bits(&doc, id), after);
}

#[test]
fn no_change_fill_returns_none() {
    let red = [1.0, 0.0, 0.0, 1.0];
    let (mut doc, id) = doc_with_layer((4, 4), (4, 4), red, (0, 0));
    let revision = doc.revision();
    let paint = fill([1.0, 0.0, 0.0], 1.0, BlendMode::Normal);
    assert!(
        doc.fill_region(id, &full(&doc, 1.0), &paint)
            .unwrap()
            .is_none()
    );
    let blue = fill([0.0, 0.0, 1.0], 1.0, BlendMode::Normal);
    assert!(
        doc.fill_region(id, &full(&doc, 0.0), &blue)
            .unwrap()
            .is_none()
    );
    let none = fill([0.0, 0.0, 1.0], 0.0, BlendMode::Normal);
    assert!(
        doc.fill_region(id, &full(&doc, 1.0), &none)
            .unwrap()
            .is_none()
    );
    assert_eq!(doc.revision(), revision);
}

/// Timing at 4096 x 4096 (run with `--release --ignored --nocapture`).
#[test]
#[ignore = "timing only"]
fn timing_4096() {
    use std::time::Instant;
    let n = 4096;
    let pixels: Vec<Rgba> = (0..n * n)
        .map(|i| {
            let (x, y) = (i % n, i / n);
            // Large areas of two colors, with a noisy stripe.
            if (x / 512 + y / 512) % 2 == 0 {
                A
            } else {
                [(i % 7) as f32 / 7.0, 0.2, 0.3, 1.0]
            }
        })
        .collect();
    let (mut doc, id) = doc_from(n, n, &pixels);
    let t = Instant::now();
    let sample = FloodSample::layer(&doc, id).unwrap();
    let t_sample = t.elapsed();
    let t = Instant::now();
    let r = flood_region(&sample, 0, 0, &settings(10.0, true, true)).unwrap();
    let t_contiguous = t.elapsed();
    let t = Instant::now();
    let _ = flood_region(&sample, 0, 0, &settings(10.0, false, true)).unwrap();
    let t_global = t.elapsed();
    let t = Instant::now();
    let _ = doc
        .fill_region(id, &r, &fill([1.0, 0.0, 0.0], 1.0, BlendMode::Normal))
        .unwrap();
    let t_fill = t.elapsed();
    println!(
        "sample {t_sample:?}, contiguous {t_contiguous:?}, global {t_global:?}, fill {t_fill:?}"
    );
}
