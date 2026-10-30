//! Tests of the transform preview ([`GpuCompositor::render_view_transformed`]).

use super::*;

const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

const fn translation(dx: f64, dy: f64) -> [f64; 6] {
    [1.0, 0.0, 0.0, 1.0, dx, dy]
}

/// The view of the full canvas at scale 1 (the straight crop).
fn full_view(doc: &Document) -> ViewSpec {
    let b = doc.bounds();
    view(0.0, 0.0, 1.0, b.width, b.height)
}

/// Renders a transform preview and reads it back.
fn read_preview(
    ctx: &GpuContext,
    comp: &mut GpuCompositor,
    doc: &Document,
    spec: ViewSpec,
    layer: LayerId,
    matrix: [f64; 6],
) -> Vec<[f32; 4]> {
    let preview = LayerTransformPreview { layer, matrix };
    let image =
        pollster::block_on(comp.render_view_transformed(ctx, doc, &spec, &preview)).unwrap();
    let data = pollster::block_on(image.read_rgba_f32(ctx)).unwrap();
    data.chunks_exact(4)
        .map(|p| [p[0], p[1], p[2], p[3]])
        .collect()
}

/// Asserts that two views are bit-equal.
fn assert_same(got: &[[f32; 4]], want: &[[f32; 4]], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: length");
    let diff = got
        .iter()
        .zip(want)
        .position(|(a, b)| a.map(f32::to_bits) != b.map(f32::to_bits));
    if let Some(i) = diff {
        panic!(
            "{what}: first different pixel {i}: got {:?}, want {:?}",
            got[i], want[i]
        );
    }
}

/// A document with a background, the test layer (with a mask), and a
/// layer above it. Returns the document and the ID of the test layer.
fn three_layer_doc(
    rng: &mut Rng,
    offset: (i32, i32),
    mask_offset: (i32, i32),
) -> (Document, LayerId) {
    let mut doc = empty_doc(40, 32, BlendSpace::Perceptual);
    add_pixel(&mut doc, rng, None, (40, 32), (0, 0));
    let id = add_pixel(&mut doc, rng, None, (18, 14), offset);
    doc.set_blend_mode(id, BlendMode::Multiply).unwrap();
    doc.set_opacity(id, 0.8).unwrap();
    set_mask(&mut doc, rng, id, (12, 10), mask_offset, MaskFill::White);
    let top = add_pixel(&mut doc, rng, None, (20, 20), (15, 8));
    doc.set_blend_mode(top, BlendMode::Screen).unwrap();
    (doc, id)
}

#[test]
fn identity_preview_equals_render_view() {
    let Some(ctx) = context() else { return };
    let mut rng = Rng(11);
    // Inside the canvas (cached textures), and partly outside it (full
    // layer and mask uploads).
    for (offset, mask_offset) in [((6, 5), (8, 7)), ((-7, 22), (-3, 20))] {
        let (doc, id) = three_layer_doc(&mut rng, offset, mask_offset);
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let spec = full_view(&doc);
        let want = read_view(&ctx, &mut comp, &doc, spec);
        let got = read_preview(&ctx, &mut comp, &doc, spec, id, IDENTITY);
        assert_same(&got, &want, &format!("identity at {offset:?}"));
        // A fresh compositor (no view composite, no snapshot).
        let mut fresh = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let got = read_preview(&ctx, &mut fresh, &doc, spec, id, IDENTITY);
        assert_same(&got, &want, &format!("identity at {offset:?}, fresh"));
    }
}

#[test]
fn integer_translation_equals_moved_layer_and_mask() {
    let Some(ctx) = context() else { return };
    let mut rng = Rng(12);
    let cases = [
        // Inside the canvas.
        ((6, 5), (8, 7), (7, -4)),
        // Partly outside: the translation brings hidden pixels in.
        ((-7, 22), (-3, 20), (9, -12)),
        // Fully outside the canvas before the move.
        ((-30, 2), (-28, 4), (35, 3)),
    ];
    for (offset, mask_offset, (dx, dy)) in cases {
        let (doc, id) = three_layer_doc(&mut rng, offset, mask_offset);
        let mut moved = doc.clone();
        moved.translate_layer(id, dx, dy).unwrap();
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let spec = full_view(&doc);
        let want = read_view(&ctx, &mut comp, &moved, spec);
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let matrix = translation(f64::from(dx), f64::from(dy));
        let got = read_preview(&ctx, &mut comp, &doc, spec, id, matrix);
        assert_same(&got, &want, &format!("translation {dx},{dy} of {offset:?}"));
    }
}

#[test]
fn mask_moves_with_the_layer() {
    let Some(ctx) = context() else { return };
    // An opaque white layer with a mask of one column at 1: the visible
    // column must move with the layer.
    let mut doc = empty_doc(16, 8, BlendSpace::Linear);
    let white = WorkingImage::from_pixels(16, 8, vec![[1.0; 4]; 16 * 8]).unwrap();
    let id = add(
        &mut doc,
        None,
        LayerContent::Pixel(PixelLayer::new(white, (0, 0)).unwrap()),
    );
    let mut data = vec![0.0; 16 * 8];
    for row in 0..8 {
        data[row * 16 + 3] = 1.0;
    }
    let mask = Mask::new(
        MaskImage::from_data(16, 8, data).unwrap(),
        (0, 0),
        MaskFill::Black,
    );
    doc.set_mask(id, Some(mask)).unwrap();
    let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
    let got = read_preview(
        &ctx,
        &mut comp,
        &doc,
        full_view(&doc),
        id,
        translation(5.0, 0.0),
    );
    for (i, p) in got.iter().enumerate() {
        let x = i % 16;
        let want = if x == 8 { 1.0 } else { 0.0 };
        assert_eq!(p[3], want, "alpha at x = {x}");
    }
}

#[test]
fn layer_in_a_group_is_transformed() {
    let Some(ctx) = context() else { return };
    let mut rng = Rng(13);
    for isolation in [GroupIsolation::Isolated, GroupIsolation::PassThrough] {
        let mut doc = empty_doc(32, 24, BlendSpace::Perceptual);
        add_pixel(&mut doc, &mut rng, None, (32, 24), (0, 0));
        let group = add(&mut doc, None, LayerContent::Group(isolation));
        doc.set_opacity(group, 0.7).unwrap();
        add_pixel(&mut doc, &mut rng, Some(group), (20, 10), (2, 3));
        let id = add_pixel(&mut doc, &mut rng, Some(group), (10, 12), (5, 6));
        doc.set_blend_mode(id, BlendMode::Overlay).unwrap();
        set_mask(&mut doc, &mut rng, id, (8, 8), (6, 7), MaskFill::Black);
        add_pixel(&mut doc, &mut rng, None, (8, 8), (20, 10));
        let mut moved = doc.clone();
        moved.translate_layer(id, 4, 2).unwrap();
        let spec = full_view(&doc);
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let want = read_view(&ctx, &mut comp, &moved, spec);
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        // A view composite first, so that the preview uses the snapshot.
        read_view(&ctx, &mut comp, &doc, spec);
        let got = read_preview(&ctx, &mut comp, &doc, spec, id, translation(4.0, 2.0));
        assert_same(&got, &want, &format!("{isolation:?} group"));
        assert_eq!(
            snapshot_index(&comp),
            Some(1),
            "{isolation:?}: below snapshot"
        );
    }
}

/// The CPU reference of the transform sample: the premultiplied bilinear
/// sample of `image` at `offset`, for each canvas pixel, through the
/// inverse of `matrix` (transparent outside the image).
fn cpu_resample(
    image: &WorkingImage,
    offset: (i32, i32),
    matrix: [f64; 6],
    size: (u32, u32),
) -> WorkingImage {
    let inv = inverse_transform(&matrix).unwrap();
    let texel = |x: i64, y: i64| -> [f64; 4] {
        if x < 0 || y < 0 || x >= i64::from(image.width()) || y >= i64::from(image.height()) {
            return [0.0; 4];
        }
        image.pixel(x as u32, y as u32).unwrap().map(f64::from)
    };
    let mut pixels = Vec::with_capacity(size.0 as usize * size.1 as usize);
    for y in 0..size.1 {
        for x in 0..size.0 {
            let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            let sx = inv[0] * px + inv[2] * py + inv[4] - f64::from(offset.0) - 0.5;
            let sy = inv[1] * px + inv[3] * py + inv[5] - f64::from(offset.1) - 0.5;
            let (x0, y0) = (sx.floor(), sy.floor());
            let (fx, fy) = (sx - x0, sy - y0);
            let (i, j) = (x0 as i64, y0 as i64);
            let mut out = [0.0_f32; 4];
            for (c, o) in out.iter_mut().enumerate() {
                let top = texel(i, j)[c] * (1.0 - fx) + texel(i + 1, j)[c] * fx;
                let bottom = texel(i, j + 1)[c] * (1.0 - fx) + texel(i + 1, j + 1)[c] * fx;
                *o = (top * (1.0 - fy) + bottom * fy) as f32;
            }
            pixels.push(out);
        }
    }
    WorkingImage::from_pixels(size.0, size.1, pixels).unwrap()
}

/// Builds a document with a background and `top` at `offset` (multiply,
/// opacity 0.9). Returns the document and the ID of the top layer.
fn two_layer_doc(
    background: &WorkingImage,
    top: WorkingImage,
    offset: (i32, i32),
) -> (Document, LayerId) {
    let mut doc = empty_doc(
        background.width(),
        background.height(),
        BlendSpace::Perceptual,
    );
    add(
        &mut doc,
        None,
        LayerContent::Pixel(PixelLayer::new(background.clone(), (0, 0)).unwrap()),
    );
    let id = add(
        &mut doc,
        None,
        LayerContent::Pixel(PixelLayer::new(top, offset).unwrap()),
    );
    doc.set_blend_mode(id, BlendMode::Multiply).unwrap();
    doc.set_opacity(id, 0.9).unwrap();
    (doc, id)
}

#[test]
fn rotation_matches_cpu_resample() {
    let Some(ctx) = context() else { return };
    let mut rng = Rng(14);
    let size = (32, 28);
    let background = random_image(&mut rng, size.0, size.1);
    let layer = random_image(&mut rng, 4, 6);
    let offset = (10, 9);
    // Rotation about the layer center (12, 12). Both sizes are even, so a
    // 90 degree rotation maps pixel centers to pixel centers.
    let (cx, cy) = (12.0, 12.0);
    let quarter = [0.0, 1.0, -1.0, 0.0, cx + cy, cy - cx];
    let angle = 30.0_f64.to_radians();
    let (s, c) = angle.sin_cos();
    let thirty = [c, s, -s, c, cx - c * cx + s * cy, cy - s * cx - c * cy];
    let big = random_image(&mut rng, 14, 11);
    // (layer, matrix, tolerance): the 90 degree samples are texel values,
    // so the result is exact. A 30 degree rotation gives values that are
    // not f16 values: the reference layer rounds them on the upload.
    let cases = [
        (layer.clone(), quarter, 0.0),
        (layer, thirty, 2e-3),
        (big, thirty, 2e-3),
    ];
    for (top, matrix, tol) in cases {
        let resampled = cpu_resample(&top, offset, matrix, size);
        let (doc, id) = two_layer_doc(&background, top, offset);
        let (reference, _) = two_layer_doc(&background, resampled, (0, 0));
        let spec = full_view(&doc);
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let want = read_view(&ctx, &mut comp, &reference, spec);
        let got = read_preview(&ctx, &mut comp, &doc, spec, id, matrix);
        // The test is not trivial: the preview differs from the document.
        assert_ne!(got, read_view(&ctx, &mut comp, &doc, spec));
        if tol == 0.0 {
            assert_same(&got, &want, "90 degree rotation");
            continue;
        }
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            for ch in 0..4 {
                let err = (g[ch] - w[ch]).abs() / w[ch].abs().max(1.0);
                assert!(err <= tol, "pixel {i} channel {ch}: got {g:?}, want {w:?}");
            }
        }
    }
}

#[test]
fn previews_do_not_change_render_view_or_the_cache() {
    let Some(ctx) = context() else { return };
    let mut rng = Rng(15);
    let (doc, id) = three_layer_doc(&mut rng, (-4, 6), (-2, 8));
    let before = doc.clone();
    let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
    let spec = full_view(&doc);
    let want = read_view(&ctx, &mut comp, &doc, spec);
    let uploads = comp.layer_uploads();
    let partial = comp.layer_partial_uploads();
    let composites = comp.view_composites();
    let revision = doc.revision();

    let angle = 0.3_f64;
    let (s, c) = angle.sin_cos();
    let matrices = [
        translation(3.0, 1.0),
        [c, s, -s, c, 4.0, -2.0],
        [1.5, 0.2, -0.1, 0.8, -3.5, 2.25],
    ];
    for matrix in matrices {
        read_preview(&ctx, &mut comp, &doc, spec, id, matrix);
    }
    assert_eq!(comp.transform_preview_composites(), 3);
    assert_eq!(snapshot_index(&comp), Some(1), "below snapshot");
    // A pan with the same matrix does not composite.
    let pan = view(5.0, 3.0, 1.0, 20, 20);
    read_preview(&ctx, &mut comp, &doc, pan, id, matrices[2]);
    assert_eq!(comp.transform_preview_composites(), 3, "a pan composited");
    assert!(comp.cache_memory_bytes() > 0);

    assert_eq!(doc, before, "the document changed");
    assert_eq!(doc.revision(), revision, "the revision changed");
    let got = read_view(&ctx, &mut comp, &doc, spec);
    assert_same(&got, &want, "render_view after the previews");
    assert_eq!(
        comp.layer_uploads(),
        uploads,
        "the previews uploaded layers"
    );
    assert_eq!(comp.layer_partial_uploads(), partial);
    assert_eq!(
        comp.view_composites(),
        composites,
        "render_view composited again"
    );
    assert!(comp.transform_view.is_none() && comp.transform_textures.is_none());

    // A change after the previews: the composite starts from the snapshot
    // that the preview made, and gives the full composite.
    let mut changed = doc.clone();
    changed.set_opacity(id, 0.5).unwrap();
    let got = read_view(&ctx, &mut comp, &changed, spec);
    let mut fresh = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
    let want = read_view(&ctx, &mut fresh, &changed, spec);
    assert_same(&got, &want, "render_view after a change");
    assert_eq!(comp.layer_uploads(), uploads, "the change uploaded layers");
}

#[test]
fn invalid_previews_are_rejected() {
    let Some(ctx) = context() else { return };
    let mut rng = Rng(16);
    let (mut doc, id) = three_layer_doc(&mut rng, (2, 2), (2, 2));
    let adjustment = add(
        &mut doc,
        None,
        LayerContent::Adjustment(Adjustment::Levels(levels())),
    );
    let group = add(
        &mut doc,
        None,
        LayerContent::Group(GroupIsolation::Isolated),
    );
    let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
    let spec = full_view(&doc);
    let mut run = |layer: LayerId, matrix: [f64; 6]| {
        let preview = LayerTransformPreview { layer, matrix };
        pollster::block_on(comp.render_view_transformed(&ctx, &doc, &spec, &preview))
            .map(|_| ())
            .unwrap_err()
    };
    for matrix in [
        [f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 1.0, f64::INFINITY, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [1.0, 2.0, 2.0, 4.0, 0.0, 0.0],
        [1e-4, 0.0, 0.0, 1e-4, 0.0, 0.0],
        [1.0, 0.0, 0.0, 1.0, 1e300, 0.0],
    ] {
        assert!(
            matches!(run(id, matrix), EditorError::InvalidView(_)),
            "{matrix:?}"
        );
    }
    assert!(matches!(
        run(adjustment, IDENTITY),
        EditorError::NotAPixelLayer(l) if l == adjustment
    ));
    assert!(matches!(
        run(group, IDENTITY),
        EditorError::NotAPixelLayer(l) if l == group
    ));
    let unknown = LayerId(9_999);
    assert!(matches!(
        run(unknown, IDENTITY),
        EditorError::LayerNotFound(l) if l == unknown
    ));
    assert_eq!(comp.transform_preview_composites(), 0);
}

/// Benchmark: the transform preview of the middle layer of a 4096 x 4096
/// document with 10 layers. Run with
/// `cargo test -p expose-editor --release -- --ignored --nocapture bench_`.
#[test]
#[ignore = "benchmark; run in release mode"]
fn bench_transform_preview_4096() {
    use std::time::Instant;

    let Some(ctx) = context() else { return };
    let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
    let mut rng = Rng(17);
    let size = 4096;
    let mut doc = empty_doc(size, size, BlendSpace::Perceptual);
    let modes = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::Normal,
        BlendMode::Color,
        BlendMode::Multiply,
    ];
    let mut ids = Vec::new();
    for mode in modes {
        let id = add_pixel(&mut doc, &mut rng, None, (size, size), (0, 0));
        doc.set_blend_mode(id, mode).unwrap();
        doc.set_opacity(id, 0.8).unwrap();
        ids.push(id);
    }
    add(
        &mut doc,
        None,
        LayerContent::Adjustment(Adjustment::Curves(curves())),
    );
    add(
        &mut doc,
        None,
        LayerContent::Adjustment(Adjustment::HueSaturation(HueSaturation {
            hue: 20.0,
            saturation: 0.2,
            lightness: 0.0,
        })),
    );
    // The middle layer (index 5 of 10), with a mask.
    let id = ids[5];
    set_mask(
        &mut doc,
        &mut rng,
        id,
        (size, size),
        (0, 0),
        MaskFill::White,
    );
    let spec = view(0.0, 0.0, 0.3, 2560, 1600);
    pollster::block_on(comp.render_view(&ctx, &doc, &spec)).unwrap();
    ctx.wait_idle().unwrap();
    let uploads = comp.layer_uploads();

    let frames = 12;
    let mut times = Vec::new();
    let half = f64::from(size) / 2.0;
    for i in 0..frames {
        let angle = (f64::from(i) * 1.5).to_radians();
        let (s, c) = angle.sin_cos();
        let matrix = [
            c,
            s,
            -s,
            c,
            half - c * half + s * half + f64::from(i),
            half - s * half - c * half,
        ];
        let preview = LayerTransformPreview { layer: id, matrix };
        let t = Instant::now();
        pollster::block_on(comp.render_view_transformed(&ctx, &doc, &spec, &preview)).unwrap();
        ctx.wait_idle().unwrap();
        times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    assert_eq!(comp.layer_uploads(), uploads, "the preview uploaded layers");
    assert_eq!(snapshot_index(&comp), Some(5), "below snapshot");
    let rest = &times[1..];
    let mean = rest.iter().sum::<f64>() / rest.len() as f64;
    let min = rest.iter().copied().fold(f64::MAX, f64::min);
    eprintln!(
        "bench transform preview 4096x4096, 10 layers, middle layer, view 2560x1600 at 0.3: \
         first frame {:.1} ms; frames 2..{frames}: mean {mean:.1} ms, min {min:.1} ms",
        times[0]
    );
}
