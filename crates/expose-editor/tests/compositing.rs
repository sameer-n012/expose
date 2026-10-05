//! Compositor tests: order, alpha, groups, masks, offsets, adjustment
//! scope, blend space, and region rendering.

// Test helpers outside `#[test]` functions also unwrap.
#![allow(clippy::unwrap_used, clippy::float_cmp)]

use expose_core::buffer::{Rect, Rgba, WorkingImage};
use expose_editor::{
    Adjustment, Background, BlendMode, BlendSpace, Document, DocumentSpec, GroupIsolation,
    HueSaturation, LayerContent, LayerId, Mask, MaskFill, MaskImage, PixelLayer, Placement,
    composite, composite_straight,
};

const EPS: f32 = 1e-5;

fn assert_px(got: Rgba, want: Rgba) {
    assert!(
        got.iter()
            .zip(want.iter())
            .all(|(a, b)| (a - b).abs() < EPS),
        "got {got:?}, expected {want:?}"
    );
}

/// A spec with linear blending. The hand-computed values in these tests use
/// linear math, so they set `BlendSpace::Linear` explicitly (the default is
/// perceptual). `perceptual_blend_space_changes_mixing` tests the default.
fn spec(w: u32, h: u32, bg: Background) -> DocumentSpec {
    DocumentSpec {
        background: bg,
        blend_space: BlendSpace::Linear,
        ..DocumentSpec::new(w, h)
    }
}

/// A solid layer with straight color `c` and alpha `a` (stored premultiplied).
fn solid(w: u32, h: u32, c: [f32; 3], a: f32, offset: (i32, i32)) -> LayerContent {
    let px = [c[0] * a, c[1] * a, c[2] * a, a];
    LayerContent::Pixel(
        PixelLayer::new(WorkingImage::new_filled(w, h, px).unwrap(), offset).unwrap(),
    )
}

fn add_top(doc: &mut Document, parent: Option<LayerId>, name: &str, c: LayerContent) -> LayerId {
    let index = match parent {
        None => doc.layers().len(),
        Some(p) => doc.layer(p).unwrap().children().len(),
    };
    doc.add_layer(name, c, Placement::new(parent, index))
        .unwrap()
        .0
}

fn full(doc: &Document) -> WorkingImage {
    composite(doc, doc.bounds()).unwrap()
}

#[test]
fn layer_order_bottom_to_top() {
    let mut doc = Document::new(spec(2, 2, Background::Black)).unwrap();
    add_top(
        &mut doc,
        None,
        "red",
        solid(2, 2, [1.0, 0.0, 0.0], 1.0, (0, 0)),
    );
    let green = add_top(
        &mut doc,
        None,
        "green",
        solid(2, 2, [0.0, 1.0, 0.0], 1.0, (0, 0)),
    );
    assert_px(full(&doc).pixel(0, 0).unwrap(), [0.0, 1.0, 0.0, 1.0]);
    // Move green below red: red is on top now.
    doc.move_layer(green, Placement::root(1)).unwrap();
    assert_px(full(&doc).pixel(0, 0).unwrap(), [1.0, 0.0, 0.0, 1.0]);
    // Hidden layers are ignored.
    let red = doc.layers()[2].id();
    doc.set_visible(red, false).unwrap();
    assert_px(full(&doc).pixel(0, 0).unwrap(), [0.0, 1.0, 0.0, 1.0]);
}

#[test]
fn alpha_edge_cases_hand_computed() {
    // Multiply, half-transparent source on an opaque backdrop:
    // co = (1 - 0.5) * 0.5 + 0.5 * (0.5 * 0.4) = 0.35, ao = 1.
    let mut doc = Document::new(spec(1, 1, Background::Color([0.5; 3]))).unwrap();
    let l = add_top(&mut doc, None, "m", solid(1, 1, [0.4; 3], 0.5, (0, 0)));
    doc.set_blend_mode(l, BlendMode::Multiply).unwrap();
    assert_px(full(&doc).pixel(0, 0).unwrap(), [0.35, 0.35, 0.35, 1.0]);

    // Any mode on a transparent backdrop gives the source unchanged.
    let mut doc = Document::new(spec(1, 1, Background::Transparent)).unwrap();
    let l = add_top(&mut doc, None, "m", solid(1, 1, [0.4; 3], 0.5, (0, 0)));
    for mode in BlendMode::ALL {
        if mode == BlendMode::Dissolve {
            continue;
        }
        doc.set_blend_mode(l, mode).unwrap();
        assert_px(full(&doc).pixel(0, 0).unwrap(), [0.2, 0.2, 0.2, 0.5]);
    }

    // Screen, both half-transparent: backdrop 0.3 (a 0.5), source 0.6 (a 0.5).
    // B = 0.72; co = 0.25 * 0.6 + 0.25 * 0.3 + 0.25 * 0.72 = 0.405; ao = 0.75.
    let mut doc = Document::new_empty(spec(1, 1, Background::Transparent)).unwrap();
    add_top(&mut doc, None, "b", solid(1, 1, [0.3; 3], 0.5, (0, 0)));
    let s = add_top(&mut doc, None, "s", solid(1, 1, [0.6; 3], 0.5, (0, 0)));
    doc.set_blend_mode(s, BlendMode::Screen).unwrap();
    assert_px(full(&doc).pixel(0, 0).unwrap(), [0.405, 0.405, 0.405, 0.75]);

    // Fully transparent source changes nothing.
    doc.set_opacity(s, 0.0).unwrap();
    assert_px(full(&doc).pixel(0, 0).unwrap(), [0.15, 0.15, 0.15, 0.5]);
}

#[test]
fn opacity_and_fill_multiply() {
    let mut doc = Document::new(spec(1, 1, Background::Black)).unwrap();
    let l = add_top(&mut doc, None, "w", solid(1, 1, [1.0; 3], 1.0, (0, 0)));
    doc.set_opacity(l, 0.5).unwrap();
    doc.set_fill_opacity(l, 0.5).unwrap();
    assert_px(full(&doc).pixel(0, 0).unwrap(), [0.25, 0.25, 0.25, 1.0]);
}

#[test]
fn non_separable_mode_in_composite() {
    // Luminosity of a gray 0.5 source onto (0.6, 0.2, 0.2), both opaque.
    // See the unit test in `blend`: (0.79492, 0.39492, 0.39492).
    let mut doc = Document::new(spec(1, 1, Background::Color([0.6, 0.2, 0.2]))).unwrap();
    let l = add_top(&mut doc, None, "g", solid(1, 1, [0.5; 3], 1.0, (0, 0)));
    doc.set_blend_mode(l, BlendMode::Luminosity).unwrap();
    assert_px(
        full(&doc).pixel(0, 0).unwrap(),
        [0.79492, 0.39492, 0.39492, 1.0],
    );
}

#[test]
fn perceptual_blend_space_changes_mixing() {
    // White at 50 % over black: perceptual (the default) mixes the encoded
    // values (1 and 0) to 0.5 and decodes to 0.214041; linear gives 0.5.
    assert_eq!(BlendSpace::default(), BlendSpace::Perceptual);
    let default_spec = DocumentSpec {
        background: Background::Black,
        ..DocumentSpec::new(1, 1)
    };
    assert_eq!(default_spec.blend_space, BlendSpace::Perceptual);
    let mut doc = Document::new(default_spec).unwrap();
    let l = add_top(&mut doc, None, "w", solid(1, 1, [1.0; 3], 1.0, (0, 0)));
    doc.set_opacity(l, 0.5).unwrap();
    let v = 0.214_041;
    assert_px(full(&doc).pixel(0, 0).unwrap(), [v, v, v, 1.0]);
    doc.set_blend_space(BlendSpace::Linear).unwrap();
    assert_px(full(&doc).pixel(0, 0).unwrap(), [0.5, 0.5, 0.5, 1.0]);
}

#[test]
fn offsets_outside_canvas() {
    let mut doc = Document::new(spec(4, 4, Background::Black)).unwrap();
    // A 3x3 layer at (-2, -1): it covers canvas columns 0 and rows 0..2.
    add_top(&mut doc, None, "w", solid(3, 3, [1.0; 3], 1.0, (-2, -1)));
    // A layer fully outside the canvas has no effect.
    add_top(
        &mut doc,
        None,
        "far",
        solid(2, 2, [0.0, 1.0, 0.0], 1.0, (10, 10)),
    );
    let img = full(&doc);
    assert_px(img.pixel(0, 0).unwrap(), [1.0, 1.0, 1.0, 1.0]);
    assert_px(img.pixel(0, 1).unwrap(), [1.0, 1.0, 1.0, 1.0]);
    assert_px(img.pixel(0, 2).unwrap(), [0.0, 0.0, 0.0, 1.0]);
    assert_px(img.pixel(1, 0).unwrap(), [0.0, 0.0, 0.0, 1.0]);
    assert_px(img.pixel(3, 3).unwrap(), [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn masks_with_offset_fill_and_enable() {
    let mut doc = Document::new(spec(4, 1, Background::Black)).unwrap();
    let l = add_top(&mut doc, None, "w", solid(4, 1, [1.0; 3], 1.0, (0, 0)));
    // A 2x1 mask at x = 1 with values (0.25, 1.0); outside it is black.
    let img = MaskImage::from_data(2, 1, vec![0.25, 1.0]).unwrap();
    doc.set_mask(l, Some(Mask::new(img, (1, 0), MaskFill::Black)))
        .unwrap();
    let out = full(&doc);
    assert_px(out.pixel(0, 0).unwrap(), [0.0, 0.0, 0.0, 1.0]);
    assert_px(out.pixel(1, 0).unwrap(), [0.25, 0.25, 0.25, 1.0]);
    assert_px(out.pixel(2, 0).unwrap(), [1.0, 1.0, 1.0, 1.0]);
    assert_px(out.pixel(3, 0).unwrap(), [0.0, 0.0, 0.0, 1.0]);
    // Mask paint, then disable.
    doc.write_mask(l, Rect::new(0, 0, 1, 1), &[0.5]).unwrap();
    assert_px(full(&doc).pixel(1, 0).unwrap(), [0.5, 0.5, 0.5, 1.0]);
    doc.set_mask_enabled(l, false).unwrap();
    assert_px(full(&doc).pixel(0, 0).unwrap(), [1.0, 1.0, 1.0, 1.0]);
    // A move moves the mask too.
    doc.set_mask_enabled(l, true).unwrap();
    doc.translate_layer(l, 1, 0).unwrap();
    let out = full(&doc);
    assert_px(out.pixel(2, 0).unwrap(), [0.5, 0.5, 0.5, 1.0]);
    assert_px(out.pixel(3, 0).unwrap(), [1.0, 1.0, 1.0, 1.0]);
}

fn desaturate() -> LayerContent {
    LayerContent::Adjustment(Adjustment::HueSaturation(HueSaturation {
        hue: 0.0,
        saturation: -1.0,
        lightness: 0.0,
    }))
}

fn is_gray(p: Rgba) -> bool {
    (p[0] - p[1]).abs() < 1e-4 && (p[1] - p[2]).abs() < 1e-4
}

#[test]
fn adjustment_layer_scope() {
    // Left column red at the root, right column red inside a group. A
    // desaturate adjustment sits inside the group above its red layer.
    let build = |iso: GroupIsolation| {
        let mut doc = Document::new(spec(2, 1, Background::Transparent)).unwrap();
        add_top(
            &mut doc,
            None,
            "left",
            solid(1, 1, [1.0, 0.0, 0.0], 1.0, (0, 0)),
        );
        let (g, _) = doc
            .add_layer("G", LayerContent::Group(iso), Placement::root(2))
            .unwrap();
        add_top(
            &mut doc,
            Some(g),
            "right",
            solid(1, 1, [1.0, 0.0, 0.0], 1.0, (1, 0)),
        );
        add_top(&mut doc, Some(g), "adj", desaturate());
        doc
    };
    // Isolated: only the layers in the group change.
    let out = full(&build(GroupIsolation::Isolated));
    assert!(!is_gray(out.pixel(0, 0).unwrap()));
    assert!(is_gray(out.pixel(1, 0).unwrap()));
    // Pass-through: the layers below the group change too.
    let out = full(&build(GroupIsolation::PassThrough));
    assert!(is_gray(out.pixel(0, 0).unwrap()));
    assert!(is_gray(out.pixel(1, 0).unwrap()));
    // An adjustment does not change transparent pixels.
    let mut doc = Document::new(spec(1, 1, Background::Transparent)).unwrap();
    add_top(&mut doc, None, "adj", desaturate());
    assert_px(full(&doc).pixel(0, 0).unwrap(), [0.0; 4]);
    // An adjustment above the layer at 50 % opacity: half way to gray.
    let mut doc = Document::new(spec(1, 1, Background::Color([1.0, 0.0, 0.0]))).unwrap();
    let adj = add_top(&mut doc, None, "adj", desaturate());
    let gray = full(&doc).pixel(0, 0).unwrap();
    doc.set_opacity(adj, 0.5).unwrap();
    let half = full(&doc).pixel(0, 0).unwrap();
    assert!((half[0] - f32::midpoint(1.0, gray[0])).abs() < 1e-5);
}

#[test]
fn group_isolation_and_group_properties() {
    // Backdrop gray 0.5; in a group: a multiply layer of 0.5.
    let build = |iso: GroupIsolation| {
        let mut doc = Document::new(spec(1, 1, Background::Color([0.5; 3]))).unwrap();
        let (g, _) = doc
            .add_layer("G", LayerContent::Group(iso), Placement::root(1))
            .unwrap();
        let l = add_top(&mut doc, Some(g), "m", solid(1, 1, [0.5; 3], 1.0, (0, 0)));
        doc.set_blend_mode(l, BlendMode::Multiply).unwrap();
        (doc, g)
    };
    // Pass-through: multiply onto the backdrop: 0.25.
    let (doc, _) = build(GroupIsolation::PassThrough);
    assert_px(full(&doc).pixel(0, 0).unwrap(), [0.25, 0.25, 0.25, 1.0]);
    // Isolated: multiply onto transparent gives 0.5; the group is normal: 0.5.
    let (mut doc, g) = build(GroupIsolation::Isolated);
    assert_px(full(&doc).pixel(0, 0).unwrap(), [0.5, 0.5, 0.5, 1.0]);
    // Group blend mode multiply: 0.25 again.
    doc.set_blend_mode(g, BlendMode::Multiply).unwrap();
    assert_px(full(&doc).pixel(0, 0).unwrap(), [0.25, 0.25, 0.25, 1.0]);
    // Group opacity 0.5 (isolated): 0.5 + 0.5 * (0.25 - 0.5) = 0.375.
    doc.set_opacity(g, 0.5).unwrap();
    assert_px(full(&doc).pixel(0, 0).unwrap(), [0.375, 0.375, 0.375, 1.0]);
    // Pass-through group opacity 0.5: mix of 0.5 and 0.25 = 0.375.
    let (mut doc, g) = build(GroupIsolation::PassThrough);
    doc.set_opacity(g, 0.5).unwrap();
    assert_px(full(&doc).pixel(0, 0).unwrap(), [0.375, 0.375, 0.375, 1.0]);
    // Hidden group: backdrop only.
    doc.set_visible(g, false).unwrap();
    assert_px(full(&doc).pixel(0, 0).unwrap(), [0.5, 0.5, 0.5, 1.0]);
}

#[test]
fn dissolve_is_binary_and_deterministic() {
    let mut doc = Document::new(spec(32, 32, Background::Black)).unwrap();
    let l = add_top(&mut doc, None, "w", solid(32, 32, [1.0; 3], 1.0, (0, 0)));
    doc.set_blend_mode(l, BlendMode::Dissolve).unwrap();
    doc.set_opacity(l, 0.5).unwrap();
    let a = full(&doc);
    assert_eq!(a, full(&doc));
    let mut shown = 0;
    for p in a.pixels() {
        assert!(p[0] == 0.0 || p[0] == 1.0, "dissolve pixel {p:?}");
        if p[0] == 1.0 {
            shown += 1;
        }
    }
    assert!((400..624).contains(&shown), "shown = {shown} of 1024");
}

/// Builds a document that uses all compositor features.
fn rich_document() -> Document {
    let mut doc = Document::new(spec(23, 17, Background::Color([0.2, 0.3, 0.4]))).unwrap();
    let mut grad = WorkingImage::new(15, 11).unwrap();
    for y in 0..11 {
        for x in 0..15 {
            #[allow(clippy::cast_precision_loss)]
            let (fx, fy) = (x as f32 / 14.0, y as f32 / 10.0);
            let a = 0.3 + 0.7 * fx;
            *grad.pixel_mut(x, y).unwrap() = [fx * 2.0 * a, fy * a, (1.0 - fx) * a, a];
        }
    }
    let g1 = add_top(
        &mut doc,
        None,
        "grad",
        LayerContent::Pixel(PixelLayer::new(grad, (-3, 4)).unwrap()),
    );
    doc.set_blend_mode(g1, BlendMode::Overlay).unwrap();
    let (grp, _) = doc
        .add_layer(
            "G",
            LayerContent::Group(GroupIsolation::Isolated),
            Placement::root(2),
        )
        .unwrap();
    let a = add_top(
        &mut doc,
        Some(grp),
        "a",
        solid(10, 10, [0.9, 0.1, 0.5], 0.8, (8, 2)),
    );
    doc.set_blend_mode(a, BlendMode::Dissolve).unwrap();
    doc.set_opacity(a, 0.7).unwrap();
    add_top(&mut doc, Some(grp), "adj", desaturate());
    doc.set_blend_mode(grp, BlendMode::Color).unwrap();
    let mut mdata = Vec::new();
    for i in 0..(12 * 9) {
        #[allow(clippy::cast_precision_loss)]
        mdata.push((i % 7) as f32 / 6.0);
    }
    let m = MaskImage::from_data(12, 9, mdata).unwrap();
    doc.set_mask(grp, Some(Mask::new(m, (5, 5), MaskFill::White)))
        .unwrap();
    let s = add_top(
        &mut doc,
        None,
        "s",
        solid(30, 5, [3.0, 0.5, 0.2], 1.0, (-4, 12)),
    );
    doc.set_blend_mode(s, BlendMode::Screen).unwrap();
    doc.set_fill_opacity(s, 0.6).unwrap();
    doc
}

#[test]
fn region_rendering_equals_crop_of_full_rendering() {
    for space in [BlendSpace::Linear, BlendSpace::Perceptual] {
        let mut doc = rich_document();
        doc.set_blend_space(space).unwrap();
        let whole = full(&doc);
        let rects = [
            Rect::new(0, 0, 23, 17),
            Rect::new(0, 0, 1, 1),
            Rect::new(22, 16, 1, 1),
            Rect::new(3, 4, 8, 8),
            Rect::new(10, 0, 13, 5),
            Rect::new(0, 9, 23, 8),
            Rect::new(7, 3, 1, 14),
        ];
        for r in rects {
            let part = composite(&doc, r).unwrap();
            assert_eq!(part, whole.crop(r).unwrap(), "region {r:?} ({space:?})");
        }
        // Tiles cover the canvas exactly.
        for ty in (0..17).step_by(8) {
            for tx in (0..23).step_by(8) {
                let r = Rect::new(tx, ty, 8.min(23 - tx), 8.min(17 - ty));
                assert_eq!(composite(&doc, r).unwrap(), whole.crop(r).unwrap());
            }
        }
    }
}

#[test]
fn invalid_regions_are_errors() {
    let doc = Document::new(DocumentSpec::new(4, 4)).unwrap();
    assert!(composite(&doc, Rect::new(0, 0, 0, 1)).is_err());
    assert!(composite(&doc, Rect::new(2, 2, 3, 1)).is_err());
}

#[test]
fn straight_alpha_output() {
    let mut doc = Document::new(spec(1, 1, Background::Transparent)).unwrap();
    add_top(
        &mut doc,
        None,
        "h",
        solid(1, 1, [0.8, 0.4, 0.2], 0.5, (0, 0)),
    );
    let out = composite_straight(&doc, doc.bounds()).unwrap();
    assert_px(out.pixel(0, 0).unwrap(), [0.8, 0.4, 0.2, 0.5]);
}
