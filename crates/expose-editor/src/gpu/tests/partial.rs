//! Tests of the partial updates: the layer texture cache and the view
//! composite must give the same values as after a full upload and a full
//! composite.

use super::*;
use crate::brush::{BrushSettings, Stroke, StrokePoint, StrokeTarget};
use crate::changes::LOG_CAPACITY;

/// Reads a texture back.
fn read(ctx: &GpuContext, image: &GpuImage) -> Vec<f32> {
    pollster::block_on(image.read_rgba_f32(ctx)).unwrap()
}

/// Asserts that two sample lists are bit-equal.
fn assert_bits(got: &[f32], want: &[f32], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: length");
    let diff = got
        .iter()
        .zip(want)
        .position(|(a, b)| a.to_bits() != b.to_bits());
    assert_eq!(diff, None, "{what}: first different sample");
}

/// Checks the cached pixel and mask textures of layer `id`: a texture with
/// the current version must be bit-equal to a full upload of the same
/// part. A texture with an old version (the last composite did not need
/// it) must have a version that the change log knows, so that the next
/// composite that needs it can update it.
fn assert_parts_are_full_uploads(
    ctx: &GpuContext,
    comp: &GpuCompositor,
    doc: &Document,
    id: LayerId,
    what: &str,
) {
    let entry = comp.cache.entries.get(&id).expect("a cache entry");
    let layer = doc.layer(id).unwrap();
    if let Some(part) = entry
        .pixels
        .as_ref()
        .filter(|p| p.version != layer.pixel_version())
    {
        assert!(layer.pixel_changes_since(part.version).is_known(), "{what}");
    }
    if let Some(part) = entry
        .mask
        .as_ref()
        .filter(|m| m.version != layer.mask_version())
    {
        assert!(layer.mask_changes_since(part.version).is_known(), "{what}");
    }
    if let Some(part) = entry
        .pixels
        .as_ref()
        .filter(|p| p.version == layer.pixel_version())
    {
        let img = layer.as_pixel().unwrap().image();
        let full = pollster::block_on(upload_pixels(ctx, img, part.rect)).unwrap();
        assert_bits(
            &read(ctx, &part.image),
            &read(ctx, &full),
            &format!("{what}: pixels"),
        );
    }
    if let Some(part) = entry
        .mask
        .as_ref()
        .filter(|m| m.version == layer.mask_version())
    {
        let img = &layer.mask().unwrap().image;
        let full = pollster::block_on(upload_mask(ctx, img, part.rect)).unwrap();
        assert_bits(
            &read(ctx, &part.image),
            &read(ctx, &full),
            &format!("{what}: mask"),
        );
    }
}

/// Renders views of `doc` with `comp` and with a new compositor (full
/// uploads, full composite). The view composites and the view outputs at
/// zoom 1 and at zoom 0.3 (pyramid) must be bit-equal.
fn assert_view_is_full(ctx: &GpuContext, comp: &mut GpuCompositor, doc: &Document, what: &str) {
    let mut fresh = pollster::block_on(GpuCompositor::new(ctx)).unwrap();
    for spec in [
        view(0.0, 0.0, 1.0, doc.width(), doc.height()),
        view(-3.0, 2.0, 0.3, 64, 48),
    ] {
        let got = read_view(ctx, comp, doc, spec);
        let want = read_view(ctx, &mut fresh, doc, spec);
        let got: Vec<f32> = got.into_iter().flatten().collect();
        let want: Vec<f32> = want.into_iter().flatten().collect();
        assert_bits(&got, &want, &format!("{what}: view at {}", spec.scale));
    }
    let got = read(ctx, &comp.view.as_ref().expect("a view").composite);
    let want = read(ctx, &fresh.view.as_ref().expect("a view").composite);
    assert_bits(&got, &want, &format!("{what}: view composite"));
}

/// The counters that the tests check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Counts {
    full: u64,
    partial: u64,
    composites: u64,
    patches: u64,
}

fn counts(comp: &GpuCompositor) -> Counts {
    Counts {
        full: comp.layer_uploads(),
        partial: comp.layer_partial_uploads(),
        composites: comp.view_composites(),
        patches: comp.view_patch_composites(),
    }
}

/// A 320 x 240 document: a base layer, a multiply layer partly outside the
/// canvas (the paint target), a layer with a mask, and a curves layer.
/// Returns the document and the IDs of the target and the masked layer.
fn paint_doc(rng: &mut Rng) -> (Document, LayerId, LayerId) {
    let mut doc = empty_doc(320, 240, BlendSpace::Perceptual);
    add_pixel(&mut doc, rng, None, (320, 240), (0, 0));
    let target = add_pixel(&mut doc, rng, None, (200, 150), (-30, 20));
    doc.set_blend_mode(target, BlendMode::Multiply).unwrap();
    let masked = add_pixel(&mut doc, rng, None, (330, 200), (-10, 30));
    doc.set_opacity(masked, 0.7).unwrap();
    // The mask width is not a multiple of 4, and the mask starts left of
    // the canvas: its canvas part starts at mask x = 7 (inside a texel).
    set_mask(&mut doc, rng, masked, (203, 130), (-7, 50), MaskFill::Black);
    add(
        &mut doc,
        None,
        LayerContent::Adjustment(Adjustment::Curves(curves())),
    );
    (doc, target, masked)
}

fn pt(x: f64, y: f64) -> StrokePoint {
    StrokePoint::new(x, y, 1.0)
}

/// The points of batch `i` of a test stroke: a short diagonal line.
fn batch(i: u32) -> Vec<StrokePoint> {
    let x = 40.0 + 12.0 * f64::from(i);
    let y = 60.0 + 7.0 * f64::from(i);
    vec![pt(x, y), pt(x + 6.0, y + 3.0), pt(x + 12.0, y + 7.0)]
}

#[test]
fn brush_batches_and_undo_redo_match_full_updates() {
    let Some(ctx) = context() else { return };
    let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
    let mut rng = Rng(31);
    let (mut doc, target, _) = paint_doc(&mut rng);
    assert_view_is_full(&ctx, &mut comp, &doc, "initial");
    let start = counts(&comp);

    let settings = BrushSettings {
        size: 14.0,
        hardness: 0.6,
        ..BrushSettings::default()
    };
    let target_color = StrokeTarget::Paint {
        color: [0.9, 0.35, 0.1],
    };
    let mut stroke = Stroke::begin(&doc, target, target_color, settings).unwrap();
    let batches = 5;
    for i in 0..batches {
        let changed = stroke.add_points(&mut doc, &batch(i)).unwrap();
        assert!(changed.is_some(), "batch {i} changed no pixel");
        assert_view_is_full(&ctx, &mut comp, &doc, &format!("batch {i}"));
        assert_parts_are_full_uploads(&ctx, &comp, &doc, target, &format!("batch {i}"));
    }
    let after = counts(&comp);
    assert_eq!(
        after.full, start.full,
        "a brush batch uploaded a full layer"
    );
    assert_eq!(after.partial, start.partial + u64::from(batches));
    assert_eq!(after.patches, start.patches + u64::from(batches));

    let edit = stroke.end(&mut doc).unwrap().expect("an edit");
    let mut history = History::default();
    history.push("Paint", vec![edit]);
    for (step, what) in [(false, "undo"), (true, "redo"), (false, "undo 2")] {
        let before = counts(&comp);
        if step {
            assert!(history.redo(&mut doc).unwrap());
        } else {
            assert!(history.undo(&mut doc).unwrap());
        }
        assert_view_is_full(&ctx, &mut comp, &doc, what);
        assert_parts_are_full_uploads(&ctx, &comp, &doc, target, what);
        let now = counts(&comp);
        assert_eq!(now.full, before.full, "{what}: full upload");
        assert_eq!(now.partial, before.partial + 1, "{what}: partial update");
        assert_eq!(now.patches, before.patches + 1, "{what}: view patch");
    }
}

#[test]
fn mask_strokes_match_full_updates() {
    let Some(ctx) = context() else { return };
    let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
    let mut rng = Rng(32);
    let (mut doc, _, masked) = paint_doc(&mut rng);
    assert_view_is_full(&ctx, &mut comp, &doc, "initial");
    let start = counts(&comp);

    let settings = BrushSettings {
        size: 9.0,
        hardness: 0.3,
        ..BrushSettings::default()
    };
    let mut stroke =
        Stroke::begin(&doc, masked, StrokeTarget::Mask { value: 0.15 }, settings).unwrap();
    // The first batch is at the left canvas edge (the first mask texels of
    // the canvas part); the next ones at odd positions inside.
    let points = [
        vec![pt(1.0, 70.0), pt(3.0, 90.0)],
        vec![pt(53.0, 101.0), pt(61.0, 99.0)],
        vec![pt(141.0, 120.0)],
        vec![pt(190.0, 175.0), pt(193.0, 178.0)],
    ];
    for (i, batch) in points.iter().enumerate() {
        stroke
            .add_points(&mut doc, batch)
            .unwrap()
            .expect("a change");
        let what = format!("mask batch {i}");
        assert_view_is_full(&ctx, &mut comp, &doc, &what);
        assert_parts_are_full_uploads(&ctx, &comp, &doc, masked, &what);
    }
    let edit = stroke.end(&mut doc).unwrap().expect("an edit");
    let mut history = History::default();
    history.push("Mask", vec![edit]);
    assert!(history.undo(&mut doc).unwrap());
    assert_view_is_full(&ctx, &mut comp, &doc, "mask undo");
    assert_parts_are_full_uploads(&ctx, &comp, &doc, masked, "mask undo");
    assert!(history.redo(&mut doc).unwrap());
    assert_view_is_full(&ctx, &mut comp, &doc, "mask redo");
    assert_parts_are_full_uploads(&ctx, &comp, &doc, masked, "mask redo");

    let now = counts(&comp);
    let steps = points.len() as u64 + 2;
    assert_eq!(now.full, start.full, "a mask change uploaded the full mask");
    assert_eq!(now.partial, start.partial + steps);
    assert_eq!(now.patches, start.patches + steps);
}

#[test]
fn log_overflow_falls_back_to_full_updates() {
    let Some(ctx) = context() else { return };
    let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
    let mut rng = Rng(33);
    let (mut doc, target, _) = paint_doc(&mut rng);
    assert_view_is_full(&ctx, &mut comp, &doc, "initial");

    // Within the log capacity: one partial update.
    let before = counts(&comp);
    for i in 0..LOG_CAPACITY as u32 {
        let px = [0.01 * (i % 50) as f32, 0.2, 0.3, 0.9];
        doc.write_pixels(target, Rect::new(30 + i % 40, 10 + i / 40, 1, 1), &[px])
            .unwrap();
    }
    assert_view_is_full(&ctx, &mut comp, &doc, "full log");
    assert_parts_are_full_uploads(&ctx, &comp, &doc, target, "full log");
    let now = counts(&comp);
    assert_eq!(now.full, before.full);
    assert_eq!(now.partial, before.partial + 1);
    assert_eq!(now.patches, before.patches + 1);

    // One change more than the capacity: the cached version is no longer
    // in the log. Full upload and full composite.
    let before = now;
    for i in 0..=LOG_CAPACITY as u32 {
        let px = [0.5, 0.01 * (i % 50) as f32, 0.3, 0.8];
        doc.write_pixels(target, Rect::new(60 + i % 40, 40 + i / 40, 1, 1), &[px])
            .unwrap();
    }
    assert_view_is_full(&ctx, &mut comp, &doc, "overflow");
    assert_parts_are_full_uploads(&ctx, &comp, &doc, target, "overflow");
    let now = counts(&comp);
    assert_eq!(now.full, before.full + 1, "overflow: full upload");
    assert_eq!(now.partial, before.partial);
    assert_eq!(now.composites, before.composites + 1);
    assert_eq!(now.patches, before.patches, "overflow: no view patch");
}

#[test]
fn other_changes_use_full_composites() {
    let Some(ctx) = context() else { return };
    let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
    let mut rng = Rng(34);
    let (mut doc, target, masked) = paint_doc(&mut rng);
    assert_view_is_full(&ctx, &mut comp, &doc, "initial");

    // Two layers changed: two partial uploads, one full composite.
    let before = counts(&comp);
    doc.write_pixels(target, Rect::new(5, 5, 2, 1), &[[0.1, 0.2, 0.3, 0.4]; 2])
        .unwrap();
    doc.write_pixels(masked, Rect::new(9, 3, 1, 2), &[[0.4, 0.3, 0.2, 0.5]; 2])
        .unwrap();
    assert_view_is_full(&ctx, &mut comp, &doc, "two layers");
    let now = counts(&comp);
    assert_eq!(now.partial, before.partial + 2);
    assert_eq!(now.patches, before.patches);
    assert_eq!(now.composites, before.composites + 1);

    // A pixel write and a property change of the same layer: full.
    let before = now;
    doc.write_pixels(target, Rect::new(8, 8, 1, 1), &[[0.1, 0.2, 0.3, 0.4]])
        .unwrap();
    doc.set_opacity(target, 0.5).unwrap();
    assert_view_is_full(&ctx, &mut comp, &doc, "write and opacity");
    assert_eq!(counts(&comp).patches, before.patches);

    // A change outside the canvas: no composite and no upload. The cached
    // part keeps its old version until a composite needs it.
    let before = counts(&comp);
    doc.write_pixels(
        target,
        Rect::new(0, 0, 10, 10),
        &[[0.7, 0.1, 0.1, 0.9]; 100],
    )
    .unwrap();
    assert_view_is_full(&ctx, &mut comp, &doc, "outside the canvas");
    let now = counts(&comp);
    assert_eq!(now.composites, before.composites);
    assert_eq!((now.full, now.partial), (before.full, before.partial));

    // A change larger than half of the canvas: full composite.
    let before = now;
    let base = doc.layers()[0].id();
    let big = Rect::new(0, 0, 300, 200);
    doc.write_pixels(base, big, &vec![[0.2, 0.2, 0.2, 0.6]; 300 * 200])
        .unwrap();
    assert_view_is_full(&ctx, &mut comp, &doc, "large change");
    let now = counts(&comp);
    assert_eq!(now.composites, before.composites + 1);
    assert_eq!(now.patches, before.patches);

    // With the partial updates off, a write uploads the full layer.
    comp.cache.no_partial = true;
    let before = now;
    doc.write_pixels(target, Rect::new(50, 50, 1, 1), &[[0.3, 0.3, 0.3, 1.0]])
        .unwrap();
    assert_view_is_full(&ctx, &mut comp, &doc, "partial off");
    let now = counts(&comp);
    assert_eq!(now.full, before.full + 1);
    assert_eq!(now.partial, before.partial);
    assert_eq!(now.patches, before.patches);
}

/// The stroke records the changed rectangle of each batch in the change
/// log of the layer, in layer coordinates. No GPU is necessary.
#[test]
fn brush_batches_record_their_rects() {
    let mut rng = Rng(35);
    let (mut doc, target, _) = paint_doc(&mut rng);
    let v0 = doc.layer(target).unwrap().pixel_version();
    let mut stroke =
        Stroke::begin(&doc, target, StrokeTarget::Erase, BrushSettings::default()).unwrap();
    let canvas = stroke.add_points(&mut doc, &batch(0)).unwrap().unwrap();
    let layer = doc.layer(target).unwrap();
    let local = layer.pixel_changes_since(v0).area().unwrap();
    let (ox, oy) = layer.as_pixel().unwrap().offset();
    // The target is inside the canvas at the stroke, so the canvas rect is
    // the layer rect moved by the offset.
    assert_eq!(
        (
            i64::from(local.x) + i64::from(ox),
            i64::from(local.y) + i64::from(oy)
        ),
        (i64::from(canvas.x), i64::from(canvas.y))
    );
    assert_eq!((local.width, local.height), (canvas.width, canvas.height));
    // A cancel clears the log.
    stroke.cancel(&mut doc).unwrap();
    assert_eq!(
        doc.layer(target).unwrap().pixel_changes_since(v0),
        Changes::Unknown
    );
}

/// Benchmark: a 4096 x 4096 document with 10 layers (the layer stack of
/// `bench_composite_4096_perceptual`). The frame time of `render_view`
/// (2560 x 1600 at zoom 0.5) after one brush batch of 16 dabs of 300 px,
/// with and without the partial updates. Run with
/// `cargo test -p expose-editor --release -- --ignored --nocapture bench_`.
#[test]
#[ignore = "benchmark; run manually in release mode"]
fn bench_brush_batch_4096() {
    use std::time::Instant;

    let Some(ctx) = context() else { return };
    let mut rng = Rng(7);
    let size = 4096;
    let mut doc = empty_doc(size, size, BlendSpace::Perceptual);
    for mode in [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::Color,
    ] {
        let id = add_pixel(&mut doc, &mut rng, None, (size, size), (0, 0));
        doc.set_blend_mode(id, mode).unwrap();
        doc.set_opacity(id, 0.8).unwrap();
    }
    let half = add_pixel(&mut doc, &mut rng, None, (2048, 2048), (1000, 1500));
    set_mask(
        &mut doc,
        &mut rng,
        half,
        (2048, 2048),
        (1000, 1500),
        MaskFill::Black,
    );
    let dodge = add_pixel(&mut doc, &mut rng, None, (2048, 2048), (-500, 2600));
    doc.set_blend_mode(dodge, BlendMode::LinearDodge).unwrap();
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
    let spec = ViewSpec {
        origin_x: 0.0,
        origin_y: 100.0,
        scale: 0.5,
        width: 2560,
        height: 1600,
    };
    // 16 dabs for each batch: the spacing is 15 px, the path 240 px.
    let settings = BrushSettings {
        size: 300.0,
        hardness: 0.5,
        spacing: 0.05,
        ..BrushSettings::default()
    };
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
    let batches: u32 = 8;

    // Returns the frame time after the first batch (it can make new
    // textures), the min and mean of the next ones, and the mean batch
    // time on the CPU.
    let run = |doc: &Document, layer_index: usize, partial: bool| {
        let mut doc = doc.clone();
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        comp.cache.no_partial = !partial;
        let id = doc.layers()[layer_index].id();
        let paint = StrokeTarget::Paint {
            color: [0.8, 0.3, 0.2],
        };
        let mut stroke = Stroke::begin(&doc, id, paint, settings).unwrap();
        let (x0, y) = (900.0, 1800.0);
        stroke.add_points(&mut doc, &[pt(x0, y)]).unwrap();
        // The first frame uploads all layers and composites all.
        pollster::block_on(comp.render_view(&ctx, &doc, &spec)).unwrap();
        ctx.wait_idle().unwrap();
        let (mut first, mut min, mut sum, mut cpu) = (0.0, f64::MAX, 0.0, 0.0);
        for i in 1..=batches {
            let x = x0 + 240.0 * f64::from(i);
            let t = Instant::now();
            stroke.add_points(&mut doc, &[pt(x, y)]).unwrap();
            cpu += ms(t);
            let t = Instant::now();
            pollster::block_on(comp.render_view(&ctx, &doc, &spec)).unwrap();
            ctx.wait_idle().unwrap();
            let frame = ms(t);
            if i == 1 {
                first = frame;
            } else {
                min = min.min(frame);
                sum += frame;
            }
        }
        if partial {
            assert_eq!(comp.view_patch_composites(), u64::from(batches));
        }
        (
            first,
            min,
            sum / f64::from(batches - 1),
            cpu / f64::from(batches),
        )
    };

    for layer_index in [0, 5] {
        let (full_first, full_min, full_mean, cpu) = run(&doc, layer_index, false);
        let (part_first, part_min, part_mean, _) = run(&doc, layer_index, true);
        eprintln!(
            "bench brush batch 4096x4096, 10 layers, paint on layer {layer_index}: \
             CPU batch {cpu:.1} ms; frame without partial updates: first {full_first:.1} \
             ms, next min {full_min:.1} / mean {full_mean:.1} ms; with partial updates: \
             first {part_first:.1} ms, next min {part_min:.1} / mean {part_mean:.1} ms"
        );
    }

    // The parts of a patch frame that touch the full canvas: the patch
    // kernel (copy of a full texture) and the pyramid build.
    let small = pollster::block_on(GpuImage::new(&ctx, 540, 300, FORMAT)).unwrap();
    let params = PatchParams {
        rect: [1000, 1000, 540, 300],
    };
    let mut copy_ms = [f64::MAX; 2];
    for (slot, format) in copy_ms.iter_mut().zip([FORMAT, LAYER_FORMAT]) {
        let a = pollster::block_on(GpuImage::new(&ctx, size, size, format)).unwrap();
        let b = pollster::block_on(GpuImage::new(&ctx, size, size, format)).unwrap();
        for _ in 0..5 {
            let t = Instant::now();
            pollster::block_on(ctx.run_kernel_multi(&PATCH_KERNEL, &params, &[&a, &small], &b))
                .unwrap();
            ctx.wait_idle().unwrap();
            *slot = slot.min(ms(t));
        }
    }
    let mut pyramid_ms = f64::MAX;
    let mut mips = Vec::new();
    let mut image = pollster::block_on(GpuImage::new(&ctx, size, size, FORMAT)).unwrap();
    for _ in 0..5 {
        let t = Instant::now();
        let cache = pollster::block_on(build_pyramid(&ctx, 0, image, mips)).unwrap();
        ctx.wait_idle().unwrap();
        pyramid_ms = pyramid_ms.min(ms(t));
        image = cache.composite;
        mips = cache.mips;
    }
    eprintln!(
        "bench patch parts 4096x4096: patch kernel rgba32float {:.1} ms, rgba16float \
         {:.1} ms; pyramid build {pyramid_ms:.1} ms",
        copy_ms[0], copy_ms[1]
    );
}
