# 0016 — Editor: masks UI, selections, and brushes

- **Status:** Accepted (operator review done; step 1 done in ADR 0018)
- **Date:** 2026-10-25
- **Decided by:** Claude (draft; the operator can change the scope or the order)

## Context

The editor (ADRs 0012–0015) can open, composite, change layer properties, and save. The document model already has layer masks (`Mask`, `set_mask`, `set_mask_enabled`) and patch writes (`write_pixels`, `write_mask`), with lock checks and history (`History::push_merge`/`end_merge`). The UI has no tool that changes pixels. Three P1 items remain: layer masks in the UI, selections, and brushes. They share two problems:

1. **Transport.** A stroke can make hundreds of changes per second. The UI must not send pixels to the shell, and the shell must not send full frames for each dab.
2. **Partial uploads.** A layer content version change makes the GPU layer cache upload the full layer (about 25 ms for 4096², ADR 0012). A stroke changes only a small area.

## Decisions

### Order of work

1. **Masks UI** (easier): add a mask (reveal all, hide all, from selection when selections exist), delete, enable/disable, "show mask" view, and a target switch (layer pixels or mask) in the layers panel. Uses the existing model operations. New `EditDto` variants only.
2. **Selections** (hard): document selection model, rectangle, ellipse, and polygon lasso tools, modes, feather, outline display.
3. **Brushes** (hard): brush engine, stroke transport, partial uploads, eraser, mask painting.

Each step has a crate task and a shell + UI task with a shared contract in both prompts.

### Selection model

- The selection is a canvas-size single-channel `MaskImage` (0 = not selected, 1 = selected) in `Document`, or `None` (all pixels selected). It is not a layer.
- Selection changes are history steps (one new `EditKind`), so undo restores them.
- Tools: rectangle, ellipse, polygon lasso (points from the UI). Modes: new, add, subtract, intersect. Anti-aliased edges; feather radius (Gaussian blur of the selection mask). Select all, deselect, invert.
- The `.exdoc` file does not store the selection in the first version.
- **Outline display:** the shell computes the outline as polygons (marching squares on the selection at the view scale) and returns it with the change. The UI draws it as an SVG overlay with an animated dash. The UI does not get the mask pixels.
- Uses: brushes and fills multiply their effect by the selection; "mask from selection" makes a layer mask.

### Brush engine

- Lives in `expose-editor` (`brush` module), CPU, deterministic (golden tests). Round dabs with size, hardness, opacity, flow, and spacing (percent of size). Optional pressure to size and opacity.
- The brush color is linear Rec. 2020 (working space, ADR 0002). The UI color picker converts from sRGB.
- Eraser: the same dabs with a destination-out operation. Mask painting writes gray values to the mask.
- **Transport:** the UI sends input points, not pixels: `editor_stroke_begin` (layer, target, brush settings), `editor_stroke_points` (batches of canvas x, y, pressure, about one batch per display frame), `editor_stroke_end`. The render thread places the dabs, writes the changed rectangle with `write_pixels`/`write_mask`, and returns the next view frame as for other edits.
- **History:** one stroke is one history step (`push_merge` from begin to end). The step stores the old pixels of the union rectangle of the stroke only.

### Partial uploads

- Each layer keeps a short list of changed rectangles for its last content versions (for example the last 64 versions). The GPU layer cache uploads only the union of the rectangles between its cached version and the current version. If the list does not reach back to the cached version, the cache uploads the full layer, as now.
- The view cache (ADR 0014) composites only the changed rectangle when the change is a pixel change of one layer.

## Reasons

- Masks UI first gives a visible result fast and uses only existing model code.
- Selections before brushes, because brushes must respect the selection.
- Points over the IPC are small (a few kB per second), and the dab placement must be deterministic and equal for CPU tests and the app, so it runs in Rust.
- Outline polygons are small and draw sharp at any zoom; sending the mask would cost a full-canvas transfer per change.

## Consequences

- New `EditKind` variants (selection) and new `EditDto` variants; the shell has an exhaustive match on `EditorError` and must follow new error variants.
- A GPU brush path is not part of this decision. If CPU dabs are too slow for large brushes at 4096², a later ADR moves dab rendering to the GPU.

## Open

- Pressure input from the webview (pointer events give `pressure`; check on the target tablets).
- Store the selection in `.exdoc` (later version of the format).
- Magic wand and quick selection (P2), selection from color range (P2).
