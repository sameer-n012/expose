//! Tests of the Gaussian blur and the unsharp mask.

// Tests compare exact values, and make test values from small integers.
#![allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use expose_core::buffer::WorkingImage;

use super::*;
use crate::adjust::{Adjustment, BrightnessContrast};
use crate::document::{Background, DocumentSpec};
use crate::history::History;
use crate::layer::{LayerContent, Mask, MaskFill, PixelLayer, Placement};
use crate::selection::{SelectionMode, SelectionShape};

fn doc_with_image(
    canvas: (u32, u32),
    image: WorkingImage,
    offset: (i32, i32),
    space: BlendSpace,
) -> (Document, LayerId) {
    let mut spec = DocumentSpec::new(canvas.0, canvas.1);
    spec.background = Background::Transparent;
    spec.blend_space = space;
    let mut doc = Document::new(spec).unwrap();
    let content = LayerContent::Pixel(PixelLayer::new(image, offset).unwrap());
    let (id, _) = doc.add_layer("Paint", content, Placement::root(1)).unwrap();
    (doc, id)
}

/// A document of `w` x `h` with one pixel layer of the same size, with
/// the pixels `f(x, y)`.
fn doc_with(
    w: u32,
    h: u32,
    space: BlendSpace,
    f: impl Fn(u32, u32) -> Rgba,
) -> (Document, LayerId) {
    let pixels = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| f(x, y))
        .collect();
    let image = WorkingImage::from_pixels(w, h, pixels).unwrap();
    doc_with_image((w, h), image, (0, 0), space)
}

fn pixels(doc: &Document, id: LayerId) -> Vec<Rgba> {
    doc.layer(id)
        .unwrap()
        .as_pixel()
        .unwrap()
        .image()
        .pixels()
        .to_vec()
}

fn px(doc: &Document, id: LayerId, x: u32, y: u32) -> Rgba {
    let p = doc.layer(id).unwrap().as_pixel().unwrap();
    p.image().pixel(x, y).unwrap()
}

fn bits(doc: &Document, id: LayerId) -> Vec<u32> {
    pixels(doc, id)
        .iter()
        .flatten()
        .map(|v| v.to_bits())
        .collect()
}

fn mask_data(doc: &Document, id: LayerId) -> Vec<f32> {
    doc.layer(id).unwrap().mask().unwrap().image.data().to_vec()
}

const fn blur_spec(radius: f32) -> FilterSpec {
    FilterSpec::GaussianBlur { radius }
}

const fn usm(amount: f32, radius: f32, threshold: f32) -> FilterSpec {
    FilterSpec::UnsharpMask {
        amount,
        radius,
        threshold,
    }
}

/// The sampled Gaussian at `x` with a normalization over a wide range.
fn reference_kernel(sigma: f64, r: i64) -> Vec<f64> {
    let w: Vec<f64> = (-r..=r)
        .map(|x| (-(x * x) as f64 / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f64 = w.iter().sum();
    w.iter().map(|v| v / sum).collect()
}

/// Blurs one row of values (one channel).
fn blur_row(row: &[f32], sigma: f32) -> Vec<f32> {
    blur::<1>(row.to_vec(), (row.len(), 1), &Plan::new(sigma))
}

// ---------------------------------------------------------------------
// Kernel
// ---------------------------------------------------------------------

#[test]
fn plan_switches_at_exact_max_sigma() {
    assert!(matches!(Plan::new(1.99), Plan::Exact { .. }));
    assert!(matches!(Plan::new(EXACT_MAX_SIGMA), Plan::Boxes(_)));
    // Four passes with the reach `r + 1` each.
    assert_eq!(Plan::new(2.0).margin(), 8);
    assert_eq!(Plan::new(0.1).margin(), 1);
    assert_eq!(Plan::new(1.5).margin(), 5);
}

#[test]
fn box_radii_give_the_gaussian_variance() {
    for sigma in [1.0, 2.0, 2.7, 3.0, 5.0, 13.3, 50.0, 500.0, 1000.0] {
        let k = ExtendedBox::new(sigma);
        assert!((0.0..1.0).contains(&k.alpha), "sigma {sigma}: {k:?}");
        let var = BOX_PASSES as f64 * k.variance();
        let rel = (var - sigma * sigma).abs() / (sigma * sigma);
        assert!(rel < 1e-9, "sigma {sigma}: variance {var}, rel {rel}");
    }
}

#[test]
fn blur_of_a_constant_image_is_unchanged() {
    for sigma in [0.1, 0.7, 1.9, 2.0, 7.5, 60.0] {
        let src = vec![0.375_f32; 40 * 30 * 4];
        let out = blur::<4>(src, (40, 30), &Plan::new(sigma));
        for v in out {
            assert!((v - 0.375).abs() < 1e-6, "sigma {sigma}: {v}");
        }
    }
}

#[test]
fn blur_keeps_the_sum_away_from_edges() {
    for sigma in [0.8, 1.5, 3.0, 9.0] {
        let (w, h) = (101, 101);
        let mut src = vec![0.0_f32; w * h];
        src[50 * w + 50] = 1.0;
        src[48 * w + 52] = 0.5;
        let out = blur::<1>(src, (w, h), &Plan::new(sigma));
        let sum: f64 = out.iter().map(|&v| f64::from(v)).sum();
        assert!((sum - 1.5).abs() < 1e-4, "sigma {sigma}: sum {sum}");
    }
}

#[test]
fn exact_blur_impulse_matches_the_gaussian() {
    let sigma = 1.5_f32;
    let (w, h) = (41, 41);
    let mut src = vec![0.0_f32; w * h];
    src[20 * w + 20] = 1.0;
    let out = blur::<1>(src, (w, h), &Plan::new(sigma));
    let k = reference_kernel(f64::from(sigma), 20);
    let peak = k[20] * k[20];
    for y in 0..h {
        for x in 0..w {
            let expected = k[x] * k[y];
            let got = f64::from(out[y * w + x]);
            // The kernel ends at 3 sigma, so the weights are about 0.3 %
            // larger (per axis) than the reference.
            assert!(
                (got - expected).abs() < 0.01 * peak,
                "({x}, {y}): {got} vs {expected}"
            );
        }
    }
}

#[test]
fn box_blur_impulse_is_close_to_exact() {
    for sigma in [2.0_f32, 2.5, 3.3, 4.0, 6.0, 8.0, 25.0, 100.0, 1000.0] {
        let r = (6.0 * sigma).ceil() as usize;
        let n = 2 * r + 1;
        let mut row = vec![0.0_f32; n];
        row[r] = 1.0;
        let out = blur_row(&row, sigma);
        let k = reference_kernel(f64::from(sigma), r as i64);
        let peak = k[r];
        let max = out
            .iter()
            .zip(&k)
            .map(|(&a, &b)| (f64::from(a) - b).abs())
            .fold(0.0, f64::max);
        assert!(max < 0.04 * peak, "sigma {sigma}: {max} vs peak {peak}");
    }
}

#[test]
fn box_blur_step_is_close_to_exact() {
    for sigma in [2.0_f32, 2.5, 3.0, 4.5, 6.0, 8.0, 12.0, 30.0, 200.0] {
        let r = (6.0 * sigma).ceil() as usize;
        let n = 4 * r;
        let row: Vec<f32> = (0..n).map(|i| if i < n / 2 { 0.0 } else { 1.0 }).collect();
        let out = blur_row(&row, sigma);
        let k = reference_kernel(f64::from(sigma), r as i64);
        // Reference: the step convolved with the kernel.
        let mut max = 0.0_f64;
        for x in r..n - r {
            let expected: f64 = k
                .iter()
                .enumerate()
                .map(|(j, w)| w * f64::from(row[x + j - r]))
                .sum();
            max = max.max((f64::from(out[x]) - expected).abs());
        }
        assert!(max < 0.008, "sigma {sigma}: {max}");
    }
}

#[test]
fn blur_clamps_to_the_edge() {
    for sigma in [1.2_f32, 6.0] {
        let row: Vec<f32> = (0..20).map(|i| (i as f32 * 0.37).sin()).collect();
        // The same row with 100 copies of each edge value.
        let mut padded = vec![row[0]; 100];
        padded.extend_from_slice(&row);
        padded.extend(std::iter::repeat_n(row[19], 100));
        let out = blur_row(&row, sigma);
        let out_padded = blur_row(&padded, sigma);
        for (i, &v) in out.iter().enumerate() {
            assert!(
                (v - out_padded[100 + i]).abs() < 1e-5,
                "sigma {sigma} x {i}"
            );
        }
    }
}

#[test]
fn transfer_tables_are_close_to_exact() {
    let t = Transfer::new(BlendSpace::Perceptual);
    let mut max_enc = 0.0_f64;
    let mut max_dec = 0.0_f64;
    for i in 0..200_000 {
        // Values from -0.1 to about 20, dense near 0.
        let shift = if i % 7 == 0 { 0.1 } else { 0.0 };
        let v = (i as f32 / 200_000.0).powi(3) * 20.0 - shift;
        let e = t.encode([v; 3])[0];
        let ee = TransferFunction::Srgb.encode(v);
        let d = t.decode_one(v);
        let de = TransferFunction::Srgb.decode(v);
        let rel = |a: f32, b: f32| f64::from((a - b).abs()) / f64::from(b.abs().max(1e-6));
        max_enc = max_enc.max(rel(e, ee));
        max_dec = max_dec.max(rel(d, de));
    }
    assert!(max_enc < 1e-6, "encode {max_enc}");
    assert!(max_dec < 1e-6, "decode {max_dec}");
    // Linear space: no change.
    let l = Transfer::new(BlendSpace::Linear);
    assert_eq!(l.encode([0.3, -1.0, 7.0]), [0.3, -1.0, 7.0]);
    assert_eq!(l.decode_one(0.42), 0.42);
}

#[test]
fn blur_is_deterministic() {
    let src: Vec<f32> = (0..64 * 64)
        .map(|i| ((i * 7919) % 97) as f32 / 97.0)
        .collect();
    for sigma in [1.0, 10.0] {
        let a = blur::<1>(src.clone(), (64, 64), &Plan::new(sigma));
        let b = blur::<1>(src.clone(), (64, 64), &Plan::new(sigma));
        assert_eq!(a, b);
    }
}

// ---------------------------------------------------------------------
// Gaussian blur on layers
// ---------------------------------------------------------------------

#[test]
fn blur_has_no_dark_fringe_at_a_transparent_edge() {
    let (mut doc, id) = doc_with(20, 4, BlendSpace::Perceptual, |x, _| {
        if x < 10 {
            [1.0, 0.0, 0.0, 1.0]
        } else {
            [0.0; 4]
        }
    });
    let edit = doc
        .apply_filter(id, FilterTarget::Pixels, &blur_spec(3.0))
        .unwrap();
    assert!(edit.is_some());
    for p in pixels(&doc, id) {
        if p[3] > 1e-4 {
            let (c, _) = unpremultiply(p);
            assert!((c[0] - 1.0).abs() < 1e-4 && c[1].abs() < 1e-6, "{p:?}");
        }
    }
    // The alpha falls across the edge.
    assert!(px(&doc, id, 9, 0)[3] < 1.0 && px(&doc, id, 10, 0)[3] > 0.0);
}

#[test]
fn apply_changes_versions_and_revision() {
    let (mut doc, id) = doc_with(16, 16, BlendSpace::Perceptual, |x, _| {
        [x as f32 / 16.0, 0.5, 0.25, 1.0]
    });
    let version = doc.layer(id).unwrap().pixel_version();
    let revision = doc.revision();
    let mut s = FilterSession::begin(&doc, id, FilterTarget::Pixels).unwrap();
    let rect = s.apply(&mut doc, &blur_spec(1.0)).unwrap().unwrap();
    assert_eq!(rect, Rect::new(0, 0, 16, 16));
    assert_ne!(doc.layer(id).unwrap().pixel_version(), version);
    assert_ne!(doc.revision(), revision);
    let changes = doc.layer(id).unwrap().pixel_changes_since(version);
    assert_eq!(changes.area(), Some(Rect::new(0, 0, 16, 16)));
    // The same spec again: no change.
    let revision = doc.revision();
    assert_eq!(s.apply(&mut doc, &blur_spec(1.0)).unwrap(), None);
    assert_eq!(doc.revision(), revision);
    assert!(s.commit(&mut doc).unwrap().is_some());
}

#[test]
fn apply_twice_equals_apply_once_with_the_second_spec() {
    let make = || {
        doc_with(24, 24, BlendSpace::Perceptual, |x, y| {
            let v = ((x * 3 + y * 5) % 7) as f32 / 7.0;
            [v * 0.8, v, 0.3, 0.8]
        })
    };
    let (mut a, ia) = make();
    let mut s = FilterSession::begin(&a, ia, FilterTarget::Pixels).unwrap();
    s.apply(&mut a, &blur_spec(4.0)).unwrap();
    s.apply(&mut a, &usm(1.5, 1.0, 0.0)).unwrap();
    s.commit(&mut a).unwrap();
    let (mut b, ib) = make();
    b.apply_filter(ib, FilterTarget::Pixels, &usm(1.5, 1.0, 0.0))
        .unwrap();
    assert_eq!(bits(&a, ia), bits(&b, ib));
}

#[test]
fn cancel_restores_bit_exact() {
    let (mut doc, id) = doc_with(20, 20, BlendSpace::Perceptual, |x, y| {
        [(x ^ y) as f32 / 31.0, 0.1, 0.9, 0.6]
    });
    let before = bits(&doc, id);
    let version = doc.layer(id).unwrap().pixel_version();
    let mut s = FilterSession::begin(&doc, id, FilterTarget::Pixels).unwrap();
    s.apply(&mut doc, &blur_spec(2.5)).unwrap();
    assert_ne!(bits(&doc, id), before);
    s.cancel(&mut doc).unwrap();
    assert_eq!(bits(&doc, id), before);
    assert_ne!(doc.layer(id).unwrap().pixel_version(), version);
}

#[test]
fn commit_and_undo_restore() {
    let (mut doc, id) = doc_with(20, 20, BlendSpace::Perceptual, |x, _| {
        if x < 10 {
            [0.2, 0.2, 0.2, 1.0]
        } else {
            [0.8, 0.8, 0.8, 1.0]
        }
    });
    let before = bits(&doc, id);
    let edit = doc
        .apply_filter(id, FilterTarget::Pixels, &blur_spec(1.0))
        .unwrap()
        .unwrap();
    let after = bits(&doc, id);
    assert_ne!(after, before);
    let mut history = History::default();
    history.push("Gaussian Blur", vec![edit]);
    assert!(history.undo(&mut doc).unwrap());
    assert_eq!(bits(&doc, id), before);
    assert!(history.redo(&mut doc).unwrap());
    assert_eq!(bits(&doc, id), after);
}

#[test]
fn constant_layer_gives_no_edit() {
    let (mut doc, id) = doc_with(10, 10, BlendSpace::Perceptual, |_, _| [0.5, 0.5, 0.5, 1.0]);
    // A blur of a constant image can change the last bit, so compare with
    // the unsharp mask with a threshold.
    let edit = doc
        .apply_filter(id, FilterTarget::Pixels, &usm(1.0, 2.0, 1.0))
        .unwrap();
    assert!(edit.is_none());
}

#[test]
fn selection_limits_the_change() {
    let (mut doc, id) = doc_with(30, 30, BlendSpace::Perceptual, |x, y| {
        let v = if (x + y) % 2 == 0 { 0.9 } else { 0.1 };
        [v, v, v, 1.0]
    });
    let before = pixels(&doc, id);
    let shape = SelectionShape::Rect {
        x: 10.0,
        y: 5.0,
        width: 8.0,
        height: 6.0,
    };
    doc.select_shape(&shape, SelectionMode::Replace, false, 0.0)
        .unwrap();
    let mut s = FilterSession::begin(&doc, id, FilterTarget::Pixels).unwrap();
    let rect = s.apply(&mut doc, &blur_spec(1.0)).unwrap().unwrap();
    assert_eq!(rect, Rect::new(10, 5, 8, 6));
    let after = pixels(&doc, id);
    for y in 0..30 {
        for x in 0..30 {
            let i = (y * 30 + x) as usize;
            let inside = (10..18).contains(&x) && (5..11).contains(&y);
            if inside {
                assert_ne!(after[i], before[i], "({x}, {y})");
            } else {
                assert_eq!(after[i], before[i], "({x}, {y})");
            }
        }
    }
    // The pixels at the edge of the selection read the pixels outside it:
    // the result is the same as a blur of the full layer.
    let (mut full, fid) = doc_with(30, 30, BlendSpace::Perceptual, |x, y| {
        let v = if (x + y) % 2 == 0 { 0.9 } else { 0.1 };
        [v, v, v, 1.0]
    });
    full.apply_filter(fid, FilterTarget::Pixels, &blur_spec(1.0))
        .unwrap();
    assert_eq!(px(&doc, id, 10, 5), px(&full, fid, 10, 5));
    assert_eq!(px(&doc, id, 17, 10), px(&full, fid, 17, 10));
}

#[test]
fn partial_selection_lerps() {
    let (mut doc, id) = doc_with(10, 1, BlendSpace::Linear, |x, _| {
        let v = if x < 5 { 0.0 } else { 1.0 };
        [v, v, v, 1.0]
    });
    let (mut full, fid) = doc_with(10, 1, BlendSpace::Linear, |x, _| {
        let v = if x < 5 { 0.0 } else { 1.0 };
        [v, v, v, 1.0]
    });
    let shape = SelectionShape::Rect {
        x: 0.0,
        y: 0.0,
        width: 10.0,
        height: 1.0,
    };
    doc.select_shape(&shape, SelectionMode::Replace, false, 0.0)
        .unwrap();
    // Half of each selection value: set a new selection with a feather is
    // more complex, so read the stored values and scale them.
    let sel = doc.selection().unwrap().data().to_vec();
    assert!(sel.iter().all(|&v| v == 1.0));
    let mut s = FilterSession::begin(&doc, id, FilterTarget::Pixels).unwrap();
    s.selection = Some(vec![0.5; 10]);
    s.apply(&mut doc, &blur_spec(1.0)).unwrap();
    full.apply_filter(fid, FilterTarget::Pixels, &blur_spec(1.0))
        .unwrap();
    let a = px(&doc, id, 4, 0)[0];
    let f = px(&full, fid, 4, 0)[0];
    assert!((a - 0.5 * f).abs() < 1e-6, "{a} vs {f}");
}

#[test]
fn selection_outside_the_layer_changes_nothing() {
    let image = WorkingImage::new_filled(10, 10, [0.5, 0.5, 0.5, 1.0]).unwrap();
    let (mut doc, id) = doc_with_image((40, 40), image, (0, 0), BlendSpace::Perceptual);
    let shape = SelectionShape::Rect {
        x: 20.0,
        y: 20.0,
        width: 5.0,
        height: 5.0,
    };
    doc.select_shape(&shape, SelectionMode::Replace, false, 0.0)
        .unwrap();
    let revision = doc.revision();
    let mut s = FilterSession::begin(&doc, id, FilterTarget::Pixels).unwrap();
    assert_eq!(s.apply(&mut doc, &blur_spec(3.0)).unwrap(), None);
    assert_eq!(s.commit(&mut doc).unwrap(), None);
    assert_eq!(doc.revision(), revision);
}

#[test]
fn values_outside_the_canvas_do_not_change() {
    // A layer of 20 x 10 at x = -5 on a canvas of 10 x 10.
    let pixels_in: Vec<Rgba> = (0..200)
        .map(|i| {
            let v = if (i % 20) % 2 == 0 { 1.0 } else { 0.0 };
            [v, v, v, 1.0]
        })
        .collect();
    let image = WorkingImage::from_pixels(20, 10, pixels_in.clone()).unwrap();
    let (mut doc, id) = doc_with_image((10, 10), image, (-5, 0), BlendSpace::Perceptual);
    let mut s = FilterSession::begin(&doc, id, FilterTarget::Pixels).unwrap();
    let rect = s.apply(&mut doc, &blur_spec(1.0)).unwrap().unwrap();
    assert_eq!(rect, Rect::new(0, 0, 10, 10));
    let after = pixels(&doc, id);
    for (i, (a, b)) in after.iter().zip(&pixels_in).enumerate() {
        let x = i % 20;
        if (5..15).contains(&x) {
            assert_ne!(a, b, "x {x}");
        } else {
            assert_eq!(a, b, "x {x}");
        }
    }
}

#[test]
fn transparency_lock_keeps_alpha() {
    let (mut doc, id) = doc_with(20, 2, BlendSpace::Perceptual, |x, _| match x {
        0..=9 => [0.9, 0.1, 0.1, 1.0],
        10..=14 => [0.05, 0.05, 0.25, 0.5],
        _ => [0.0; 4],
    });
    let alpha: Vec<f32> = pixels(&doc, id).iter().map(|p| p[3]).collect();
    let locks = Locks {
        transparency: true,
        ..Locks::NONE
    };
    doc.set_locks(id, locks).unwrap();
    doc.apply_filter(id, FilterTarget::Pixels, &blur_spec(2.0))
        .unwrap()
        .unwrap();
    let after = pixels(&doc, id);
    for (p, a) in after.iter().zip(&alpha) {
        assert_eq!(p[3], *a);
    }
    assert_eq!(after[19], [0.0; 4]);
    // The color changed near the edge.
    assert!(after[9][2] > 0.1);
}

// ---------------------------------------------------------------------
// Unsharp mask
// ---------------------------------------------------------------------

#[test]
fn unsharp_overshoots_on_a_step_edge_and_keeps_alpha() {
    // Straight colors 0.2 and 0.8 with the alpha 0.75 (premultiplied).
    let (mut doc, id) = doc_with(20, 3, BlendSpace::Perceptual, |x, _| {
        if x < 10 {
            [0.15, 0.15, 0.15, 0.75]
        } else {
            [0.6, 0.6, 0.6, 0.75]
        }
    });
    doc.apply_filter(id, FilterTarget::Pixels, &usm(1.0, 1.5, 0.0))
        .unwrap()
        .unwrap();
    let dark = unpremultiply(px(&doc, id, 9, 1)).0[0];
    let light = unpremultiply(px(&doc, id, 10, 1)).0[0];
    assert!(dark < 0.19, "{dark}");
    assert!(light > 0.81, "{light}");
    // Far from the edge, no change (the threshold is 0, so only the
    // rounding of the blur changes the value).
    let far = unpremultiply(px(&doc, id, 0, 1)).0[0];
    assert!((far - 0.2).abs() < 1e-5, "{far}");
    for p in pixels(&doc, id) {
        assert_eq!(p[3], 0.75);
    }
}

#[test]
fn unsharp_in_linear_space_follows_the_rule() {
    let row = [0.2_f32, 0.2, 0.2, 0.8, 0.8, 0.8];
    let (mut doc, id) = doc_with(6, 1, BlendSpace::Linear, |x, _| {
        let v = row[x as usize];
        [v, v, v, 1.0]
    });
    doc.apply_filter(id, FilterTarget::Pixels, &usm(2.0, 1.0, 0.0))
        .unwrap();
    let b = blur_row(&row, 1.0);
    for x in 0..6 {
        let d = row[x] - b[x];
        let expected = if d.abs() * 255.0 > 0.0 {
            row[x] + 2.0 * d
        } else {
            row[x]
        };
        let got = px(&doc, id, x as u32, 0)[0];
        assert!((got - expected).abs() < 1e-6, "x {x}: {got} vs {expected}");
    }
}

#[test]
fn unsharp_threshold_suppresses_small_detail() {
    // Detail of 0.5 levels; an edge of 100 levels.
    let (mut doc, id) = doc_with(30, 2, BlendSpace::Linear, |x, _| {
        let v = if x < 20 {
            0.5 + if x % 2 == 0 {
                0.25 / 255.0
            } else {
                -0.25 / 255.0
            }
        } else {
            0.5 + 100.0 / 255.0
        };
        [v, v, v, 1.0]
    });
    let before = pixels(&doc, id);
    doc.apply_filter(id, FilterTarget::Pixels, &usm(1.0, 1.0, 4.0))
        .unwrap()
        .unwrap();
    let after = pixels(&doc, id);
    for x in 0..17 {
        assert_eq!(after[x], before[x], "x {x}");
    }
    assert_ne!(after[19], before[19]);
    assert_ne!(after[20], before[20]);
}

#[test]
fn unsharp_keeps_transparent_pixels() {
    let (mut doc, id) = doc_with(10, 1, BlendSpace::Perceptual, |x, _| {
        if x < 5 {
            [0.6, 0.3, 0.1, 1.0]
        } else {
            [0.0; 4]
        }
    });
    doc.apply_filter(id, FilterTarget::Pixels, &usm(3.0, 2.0, 0.0))
        .unwrap();
    let after = pixels(&doc, id);
    for p in &after[5..] {
        assert_eq!(*p, [0.0; 4]);
    }
    // The alpha-weighted blur ignores the transparent pixels, so the
    // opaque pixels of one color do not change.
    for p in &after[..5] {
        for (a, b) in p.iter().zip([0.6, 0.3, 0.1, 1.0]) {
            assert!((a - b).abs() < 1e-5, "{p:?}");
        }
    }
}

// ---------------------------------------------------------------------
// Mask target
// ---------------------------------------------------------------------

fn doc_with_mask(values: &[f32]) -> (Document, LayerId) {
    let w = values.len() as u32;
    let (mut doc, id) = doc_with(w, 1, BlendSpace::Perceptual, |_, _| [0.2, 0.4, 0.6, 1.0]);
    let image = MaskImage::from_data(w, 1, values.to_vec()).unwrap();
    doc.set_mask(id, Some(Mask::new(image, (0, 0), MaskFill::White)))
        .unwrap();
    (doc, id)
}

#[test]
fn mask_blur_changes_the_mask_only() {
    let values: Vec<f32> = (0..12).map(|x| if x < 6 { 0.0 } else { 1.0 }).collect();
    let (mut doc, id) = doc_with_mask(&values);
    let pixels_before = bits(&doc, id);
    let version = doc.layer(id).unwrap().mask_version();
    let pixel_version = doc.layer(id).unwrap().pixel_version();
    let edit = doc
        .apply_filter(id, FilterTarget::Mask, &blur_spec(1.0))
        .unwrap()
        .unwrap();
    let after = mask_data(&doc, id);
    let expected = blur_row(&values, 1.0);
    assert_eq!(after, expected);
    assert_eq!(bits(&doc, id), pixels_before);
    assert_ne!(doc.layer(id).unwrap().mask_version(), version);
    assert_eq!(doc.layer(id).unwrap().pixel_version(), pixel_version);
    let mut history = History::default();
    history.push("Blur", vec![edit]);
    history.undo(&mut doc).unwrap();
    assert_eq!(mask_data(&doc, id), values);
}

#[test]
fn mask_unsharp_clamps() {
    let values: Vec<f32> = (0..12).map(|x| if x < 6 { 0.05 } else { 0.95 }).collect();
    let (mut doc, id) = doc_with_mask(&values);
    doc.apply_filter(id, FilterTarget::Mask, &usm(5.0, 1.0, 0.0))
        .unwrap();
    let after = mask_data(&doc, id);
    assert_eq!(after[5], 0.0);
    assert_eq!(after[6], 1.0);
    assert!(after.iter().all(|v| (0.0..=1.0).contains(v)));
}

#[test]
fn mask_target_on_an_adjustment_layer() {
    let (mut doc, _) = doc_with(8, 1, BlendSpace::Perceptual, |_, _| [0.5; 4]);
    let content =
        LayerContent::Adjustment(Adjustment::BrightnessContrast(BrightnessContrast::default()));
    let (adj, _) = doc.add_layer("Adj", content, Placement::root(2)).unwrap();
    assert_eq!(
        FilterSession::begin(&doc, adj, FilterTarget::Pixels).unwrap_err(),
        EditorError::NotAPixelLayer(adj)
    );
    assert_eq!(
        FilterSession::begin(&doc, adj, FilterTarget::Mask).unwrap_err(),
        EditorError::NoMask(adj)
    );
    let values: Vec<f32> = (0..8).map(|x| if x < 4 { 0.0 } else { 1.0 }).collect();
    let image = MaskImage::from_data(8, 1, values).unwrap();
    doc.set_mask(adj, Some(Mask::new(image, (0, 0), MaskFill::White)))
        .unwrap();
    assert!(
        doc.apply_filter(adj, FilterTarget::Mask, &blur_spec(1.0))
            .unwrap()
            .is_some()
    );
}

// ---------------------------------------------------------------------
// Errors and conflicts
// ---------------------------------------------------------------------

#[test]
fn bad_values_are_refused() {
    let (mut doc, id) = doc_with(4, 4, BlendSpace::Perceptual, |_, _| [0.5; 4]);
    let revision = doc.revision();
    let cases = [
        (blur_spec(0.05), "radius"),
        (blur_spec(1001.0), "radius"),
        (usm(0.0, 1.0, 0.0), "amount"),
        (usm(6.0, 1.0, 0.0), "amount"),
        (usm(1.0, 1.0, 256.0), "threshold"),
        (usm(1.0, 1.0, -1.0), "threshold"),
    ];
    for (spec, name) in cases {
        match doc.apply_filter(id, FilterTarget::Pixels, &spec) {
            Err(EditorError::OutOfRange { name: n, .. }) => assert_eq!(n, name),
            other => panic!("{spec:?}: {other:?}"),
        }
    }
    assert_eq!(
        doc.apply_filter(id, FilterTarget::Pixels, &blur_spec(f32::NAN)),
        Err(EditorError::NonFinite { name: "radius" })
    );
    assert_eq!(
        doc.apply_filter(id, FilterTarget::Pixels, &usm(f32::INFINITY, 1.0, 0.0)),
        Err(EditorError::NonFinite { name: "amount" })
    );
    assert_eq!(doc.revision(), revision);
}

#[test]
fn locks_are_checked() {
    let (mut doc, id) = doc_with(4, 4, BlendSpace::Perceptual, |_, _| [0.5; 4]);
    let missing = LayerId(9999);
    assert!(matches!(
        FilterSession::begin(&doc, missing, FilterTarget::Pixels),
        Err(EditorError::LayerNotFound(_))
    ));
    let locks = Locks {
        pixels: true,
        ..Locks::NONE
    };
    doc.set_locks(id, locks).unwrap();
    assert_eq!(
        FilterSession::begin(&doc, id, FilterTarget::Pixels).unwrap_err(),
        EditorError::Locked {
            id,
            lock: LockKind::Pixels
        }
    );
    doc.set_locks(id, Locks::NONE).unwrap();
    // The `all` lock on a parent group.
    let group_content = LayerContent::Group(crate::layer::GroupIsolation::default());
    let (group, _) = doc
        .add_layer("Group", group_content, Placement::root(2))
        .unwrap();
    doc.move_layer(id, Placement::new(Some(group), 0)).unwrap();
    let all = Locks {
        all: true,
        ..Locks::NONE
    };
    doc.set_locks(group, all).unwrap();
    assert_eq!(
        FilterSession::begin(&doc, id, FilterTarget::Pixels).unwrap_err(),
        EditorError::Locked {
            id,
            lock: LockKind::All
        }
    );
}

#[test]
fn changes_outside_the_session_give_a_conflict() {
    let (mut doc, id) = doc_with(8, 8, BlendSpace::Perceptual, |x, _| {
        [x as f32 / 8.0, 0.5, 0.5, 1.0]
    });
    let mut s = FilterSession::begin(&doc, id, FilterTarget::Pixels).unwrap();
    s.apply(&mut doc, &blur_spec(1.0)).unwrap();
    doc.write_pixels(id, Rect::new(0, 0, 1, 1), &[[1.0; 4]])
        .unwrap();
    let written = bits(&doc, id);
    assert_eq!(
        s.apply(&mut doc, &blur_spec(2.0)),
        Err(EditorError::StrokeConflict(id))
    );
    assert_eq!(bits(&doc, id), written);
    assert_eq!(s.cancel(&mut doc), Err(EditorError::StrokeConflict(id)));

    let s = FilterSession::begin(&doc, id, FilterTarget::Pixels).unwrap();
    let locks = Locks {
        transparency: true,
        ..Locks::NONE
    };
    doc.set_locks(id, locks).unwrap();
    assert_eq!(s.commit(&mut doc), Err(EditorError::StrokeConflict(id)));

    let s = FilterSession::begin(&doc, id, FilterTarget::Pixels).unwrap();
    doc.translate_layer(id, 1, 0).unwrap();
    assert_eq!(s.commit(&mut doc), Err(EditorError::StrokeConflict(id)));
}

#[test]
fn session_accessors_and_debug() {
    let (doc, id) = doc_with(64, 64, BlendSpace::Perceptual, |_, _| [0.5; 4]);
    let s = FilterSession::begin(&doc, id, FilterTarget::Pixels).unwrap();
    assert_eq!(s.layer(), id);
    assert_eq!(s.target(), FilterTarget::Pixels);
    let text = format!("{s:?}");
    assert!(text.len() < 400, "{text}");
    assert!(text.contains("original_len: 16384"));
}

// ---------------------------------------------------------------------
// Timing (run with `cargo test --release -p expose-editor -- --ignored
// --nocapture filter::tests::timing`)
// ---------------------------------------------------------------------

fn timing_doc() -> (Document, LayerId) {
    doc_with(4096, 4096, BlendSpace::Perceptual, |x, y| {
        let v = ((x * 13 + y * 7) % 251) as f32 / 251.0;
        [v, v * 0.5, 1.0 - v, 1.0]
    })
}

fn time(spec: FilterSpec) {
    let (mut doc, id) = timing_doc();
    let mut s = FilterSession::begin(&doc, id, FilterTarget::Pixels).unwrap();
    let start = std::time::Instant::now();
    s.apply(&mut doc, &spec).unwrap();
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    println!("{spec:?} at 4096 x 4096: {ms:.0} ms");
}

#[test]
#[ignore = "timing; run in release"]
fn timing_blur_r2() {
    time(blur_spec(2.0));
}

#[test]
#[ignore = "timing; run in release"]
fn timing_blur_r1_9() {
    time(blur_spec(1.9));
}

#[test]
#[ignore = "timing; run in release"]
fn timing_blur_r50() {
    time(blur_spec(50.0));
}

#[test]
#[ignore = "timing; run in release"]
fn timing_blur_r500() {
    time(blur_spec(500.0));
}

#[test]
#[ignore = "timing; run in release"]
fn timing_unsharp_r1() {
    time(usm(1.0, 1.0, 0.0));
}
