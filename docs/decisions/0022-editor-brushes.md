# 0022 — Editor: brush engine and stroke transport

- **Status:** Accepted
- **Date:** 2026-10-28
- **Decided by:** Claude

## Context

ADR 0016 step 3 is brushes. This ADR records the decisions of the brush engine in `expose-editor` (PR #71), the stroke commands of the shell, and the brush tools of the UI.

## Decisions

### Brush engine (`expose_editor::brush`)

- `Stroke::{begin, add_points, end, cancel}` with `BrushSettings` (size 1..=5000 px, hardness, opacity, flow, spacing 0.01..=10 of the size, pressure to size and to opacity) and `StrokeTarget::{Paint { color }, Erase, Mask { value }}`.
- Dab placement is deterministic. The distance remainder carries over between point batches, so the batch split does not change the pixels. The smallest dab step is 0.5 px.
- Dab shape: `clamp(r + 0.5 − d, 0, 1) · (1 − smoothstep((d − h·r) / (r − h·r))) · min(1, 2r)`.
- Stroke buffer model: a coverage value per pixel grows with `c += (1 − c) · flow · dab`, and opacity caps it. The result is always computed from the original content, so opacity limits the total effect of one stroke. The selection value multiplies each dab.
- Paint = color OVER original (premultiplied). With the transparency lock, only the color changes. Erase = destination-out (an error with the transparency lock). Mask = mix of the old value and the target value.
- The stroke keeps the original content and the coverage in 64 px tiles, made when a dab first touches them. `end` gives one undo edit (`PixelTiles` / `MaskTiles`) with only the tiles that changed. Example: a 4096² test stroke stores 21 MiB, not 256 MiB.
- If the layer changes outside the stroke (version, offset, size, locks), the stroke fails with `EditorError::StrokeConflict` (shell: `invalidInput`).

### Stroke transport (shell and UI)

- Commands: `editor_stroke_begin(documentId, layerId, target, brush) -> strokeId`, `editor_stroke_points(documentId, strokeId, points)` (flat `[x, y, pressure, ...]`, 1..=3000 points per call), `editor_stroke_end -> DocumentInfo` (one history step), `editor_stroke_cancel -> DocumentInfo`.
- One active stroke per document. Any other edit, undo, redo, save, or close commits the active stroke first.
- The UI sends one points call at a time and batches the points that come while it runs. After each reply it asks for a new frame.
- **Change from ADR 0016:** the paint color in the command is gamma-encoded sRGB (0..1), and the shell converts it to linear Rec. 2020. The color math then has one implementation (Rust), not a copy in TypeScript.
- Paint target: the UI selects the layer pixels or the layer mask (click on the mask chip). On the mask, the brush paints the luminance of the foreground color and the eraser paints the luminance of the background color.

## Reasons

- The stroke buffer model gives the expected result for overlapping dabs: a 50 % opacity stroke never goes above 50 %, however often it crosses itself.
- Tiles keep memory and undo data in proportion to the painted area.
- A stroke ID lets the UI ignore late replies of a stroke that is already committed.

## Consequences

- CPU dabs: 9–11 ms per batch of 16 dabs of 300 px at 4096² (release). A GPU dab path needs a new ADR if this is too slow.
- Each points batch changes the layer version. Without partial uploads, the GPU cache uploads the full layer each time; the partial upload task (ADR 0016 "Partial uploads") removes this cost.

## Open

- Pressure on real tablets in the webview (ADR 0016).
- Brush tips other than round, smoothing, and blend modes for the brush (P2).
