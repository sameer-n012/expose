# 0021 — Editor: selections in the shell and the UI

- **Status:** Accepted
- **Date:** 2026-10-28
- **Decided by:** Claude

## Context

ADR 0019 records the selection model in `expose-editor` and the outline tracer in `expose-core`. This ADR records how the shell and the UI use them (task editor-selections-ui in `TODO.md`, ADR 0016 step 2).

## Decisions

### Selection edits are `EditDto` variants

- New variants: `selectShape { shape, mode, antiAlias, feather }`, `selectAll`, `deselect`, `invertSelection`, `featherSelection { radius }`, `maskFromSelection { layerId }`.
- `SelectionShapeDto` = `rect` / `ellipse` (bounding box) / `polygon { points: [x, y][] }`, in canvas pixels. `SelectionModeId` = `replace` / `add` / `subtract` / `intersect`.
- Each edit is one undo step, as for all other edits. The tools send points, not pixels (ADR 0016).

### `DocumentInfo` tells the UI about the selection

- `hasSelection: boolean` lets the UI disable deselect, invert, and feather (the crate gives `NoSelection` for them).
- `selectionVersion: number` is the crate selection stamp. It changes at each selection change, also at undo and redo.

### The outline is a separate command

- `editor_selection_outline(documentId, step) -> { selectionVersion, step, contours }`. Each contour is a flat `[x0, y0, x1, y1, ...]` list in canvas pixels. Threshold 0.5.
- The UI asks for the outline when `selectionVersion` changes or when the step changes. `step = max(1, floor(1 / scale))`, so the outline has about one point per screen pixel when the view is zoomed out. Pan and zoom in at the same step only change the SVG transform.
- Point cap: 100 000 points. If the outline has more points, the shell doubles the step until it fits, and returns the step that it used.

## Reasons

- An outline in each edit reply would make all edit replies larger, also for edits that do not change the selection. Undo and redo would need the same extra data. A separate command with the version stamp covers all cases with one rule.
- The step from the zoom keeps the point count near the screen resolution, so the SVG stays fast.
- Doubling the step always ends: at a step equal to or larger than the canvas long edge, the grid has only a few samples.

## Consequences

- One extra IPC call after each selection change. Tracing runs on the editor render thread.
- The outline of a feathered selection shows the 0.5 iso-line only.
- The "Add mask" button uses the selection when one exists (click = mask from selection, Alt+click = hide all).

## Open

- `feather_selection` takes about 200 ms at 4096² with a 20 px radius. Speed-up: blur only the edge band.
