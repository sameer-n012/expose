# 0029 — Editor: free transform

- **Status:** Accepted
- **Date:** 2026-10-30
- **Decided by:** Claude (the operator gave Claude the order of the P1 editor tools, session 12)

## Context

P1 item in `FEATURES.md` 3.7: "Free transform (move, scale, rotate) with reference point". The user must see the result live while a handle moves, and the committed pixels must have good quality. Layers already have an offset and any size (`PixelLayer`), and masks have their own offset and an outside fill (`MaskFill`).

The work goes in two rounds:

1. In parallel: crate CPU transform (`expose_editor::transform`), crate GPU preview (`gpu.rs` + shaders), and UI (transform mode, handles, options bar). The two crate agents own disjoint files.
2. Shell (bounds command, preview parameter of `editor_render`, `transformLayer` edit) after both crate PRs are merged.

## Decisions

### Matrix

- An affine matrix `[a, b, c, d, e, f]` (f64) maps a canvas point of the old layer to the canvas point of the new layer: `x' = a·x + c·y + e`, `y' = b·x + d·y + f` (the order of the CSS `matrix()` function). Points are in canvas pixels; pixel centers are at `+0.5`.
- The matrix must be finite and invertible (`|det| >= 1e-6`). The UI builds it from translate, scale, and rotate about the reference point. Skew and perspective are P2.

### CPU transform (crate, commit)

- `Document::layer_content_bounds(layer) -> EditorResult<Option<Rect>>`: the canvas rectangle of the pixels with alpha > 0 of a pixel layer (`None` if all are transparent). The UI draws the transform box on these bounds.
- `Document::transform_layer(layer, &Affine, Interpolation) -> EditorResult<Edit>`: transforms the content of a pixel layer (only the content bounds, not the transparent rest) and its mask (if it has one). One undo edit that holds the old layer image, offset, and mask.
  - The new layer image covers the bounding box of the transformed content bounds (rounded out to whole pixels). It can be partly outside the canvas: pixels outside the canvas stay in the layer (a later move can bring them back). A result larger than `MAX_DIMENSION` / `MAX_PIXELS` is an error.
  - A pure translation by whole pixels changes only the offset (no resample, lossless).
  - Resample in premultiplied alpha. `Interpolation`: `Bicubic` (Catmull-Rom, default), `Bilinear`, `Nearest`. For a scale below 1 in a direction, the kernel is widened by `1 / scale` in that direction, so a strong downscale does not alias.
  - The mask is resampled with the same matrix; outside its transformed area the mask uses its `MaskFill`.
  - Errors: not a pixel layer (`NotAPixelLayer`), the pixels lock or the position lock (`Locked`; check which lock kinds exist), a bad matrix (new `EditorError::InvalidTransform`), an empty layer (`InvalidTransform`).
- Transform of the selected pixels only (a floating selection) is a follow-up. In P1 the whole layer transforms; the selection does not change.

### GPU preview (crate)

- `GpuCompositor::render_view_transformed(ctx, document, view, &LayerTransformPreview { layer, matrix: [f64; 6] })` renders the view as `render_view` does, but draws the given layer (and its mask) through the matrix with bilinear sampling. It uses the below snapshot cache (the layers below the transformed layer composite once per preview), so each preview frame composites only the layer and the layers above.
- The preview does not change the document, the layer texture cache, or the change logs. After the preview, a plain `render_view` gives the old result again without a full upload.

### UI

- Command "Free transform" `Mod+T` on the selected pixel layer (also a toolbar or menu entry). It asks the shell for the content bounds; an empty layer or another layer kind gives a toast.
- Transform mode shows the bounding box, 8 scale handles, the reference point (default: center), and a rotate area outside the corners. Drag inside the box = move; a handle = scale about the reference point (Shift keeps the aspect ratio; Alt scales about the center); outside the box = rotate about the reference point (Shift snaps to 15°). Drag the reference point to move it. Arrow keys move 1 px (Shift: 10 px).
- Options bar: X, Y (reference point), W %, H %, angle, a 3×3 reference point grid, interpolation (bicubic, bilinear, nearest), Commit, Cancel. Enter commits, Escape cancels. Undo during transform mode cancels it.
- While the mode is active, the frames come from `editor_render` with the `transformPreview` parameter. Other tools and layer edits are disabled until commit or cancel.

### Shell contract (round 2)

```ts
// new command
editor_layer_bounds(documentId, layerId) -> { x: number; y: number; width: number; height: number } | null
// new optional parameter of editor_render
transformPreview?: { layerId: number; matrix: [number, number, number, number, number, number] } | null
// new EditDto variant
| { type: 'transformLayer'; layerId: number;
    matrix: [number, number, number, number, number, number];
    interpolation: 'bicubic' | 'bilinear' | 'nearest' }
```

- History name: "Free Transform". A not-pixel layer, a bad matrix, or an empty layer gives `invalidInput`; locks give `locked`.

### Implementation notes (round 1)

- CPU (PR #99): the core `Rect` is unsigned, so bounds use the new `CanvasRect` (i64 x, y). New undo record `EditKind::PixelLayer` (whole image, offset, optional mask). The transparency lock does not block a transform; the pixels, position, and all locks do. The downscale filter widens per source axis by the length of that axis after the transform, up to a radius of `MAX_FILTER_RADIUS` (32) source pixels: this is exact for axis scales and scale-then-rotate, and shear can alias a little. 4096², rotate 30° + scale 0.8, bicubic, release: about 1.2 s.
- GPU (PR #100): `blend_layer.wgsl` has a transform mode (`params.flags.w == 1`); the normal path gives the same bits as before. The preview keeps its own composite and pyramid, keyed by revision, size, layer, and matrix bits. A layer inside a group composites the whole top-level group each frame (correct, slower). A layer partly outside the canvas and larger than the device texture limit gives `EditorError::Gpu`. 4096², 10 layers, 2560×1600 view: first frame 152 ms, next frames 29.1 ms, no uploads.
- UI (PR #101): per-tab state in `EditorTab` (`TransformSession`), math in `lib/editor/transform.ts`. A tool change, another layer, or a tab switch cancels the mode with no edit; undo, redo, and history jump only cancel it. While the mode is active it takes all left-button events; middle button and Space still pan.
- The preview is bilinear and the commit is bicubic by default, so the committed pixels can be slightly sharper than the last preview frame.

### Implementation notes (round 2)

- Shell (PR #102): `editor_layer_bounds` (`Job::LayerBounds`, `editor/bounds.rs`; does not commit an active stroke), `TransformPreviewDto` on `editor_render`, `EditDto::TransformLayer` with `InterpolationId`. A render with both `showMaskLayerId` and `transformPreview` gives the new `EditorEngineError::InvalidRender` (`invalidInput`).
- The commit runs inline on the engine render thread. From reading the code: frame requests wait during it and are not rejected, and only the newest frame per document renders after it. This behavior was not timed or tested with frames sent during a 4096² commit.

## Reasons

- A GPU preview through the matrix keeps each frame fast (the below snapshot is the slider path of ADR 0012) and agrees with the commit within the difference of bilinear and bicubic sampling.
- Transforming only the content bounds keeps the new layer small: a canvas-size layer with a small object does not grow to the rotated canvas size.
- The whole-pixel translation path makes a move lossless, so many moves do not blur the layer.

## Consequences

- New `EditorError::InvalidTransform`: the shell match needs an arm (the orchestrator adds it in the crate branch).
- Layers can now hold pixels outside the canvas after a transform. Save, export, and composite already use the layer offset and size.

## Open

- Transform of a selection (floating pixels), skew, distort, perspective, warp (P2).
- Transform of a group or of several layers.
