# 0019 — Editor: selection model and outline tracing

- **Status:** Accepted
- **Date:** 2026-10-27
- **Decided by:** Claude

## Context

ADR 0016 step 2 is selections. This ADR records the crate part: the selection in `expose-editor` (PR #66) and the outline tracer in `expose-core` (PR #64). The shell and the UI come in a later task (editor-selections-ui in `TODO.md`).

## Decisions

### Selection model (`expose_editor::selection`)

- `Document::selection() -> Option<&MaskImage>`: a canvas-size mask (0 = not selected, 1 = selected), or `None` = no selection (all pixels act as selected). An operation that removes all selected pixels keeps an all-zero mask; only `deselect` gives `None`.
- `select_shape(shape, mode, anti_alias, feather_radius)` with `SelectionShape::{Rect, Ellipse, Polygon}` in canvas coordinates. Polygons use the nonzero rule (a self-crossing lasso has no holes). Limits: `MAX_POLYGON_POINTS` = 100 000, `MAX_SELECTION_COORD` = 1e7, `MAX_FEATHER_RADIUS` = 250.
- Anti-alias: exact area for rectangles; 16 sub-rows per pixel row with exact covered length for ellipses and polygons. Without anti-alias, a pixel is selected if its center is inside.
- Modes: replace; add = max(a, b); subtract = a·(1−b); intersect = a·b. With no selection, the old selection counts as all zero.
- Feather: Gaussian, sigma = radius / 3, separable, edge values repeat at the canvas border.
- `invert_selection`, `feather_selection`, and `deselect` with no selection give the new error `EditorError::NoSelection` (the shell maps it to `invalidInput`), so the UI can disable these commands.
- `mask_from_selection(id)`: a canvas-size mask at (0, 0) from the selection (all 1.0 with no selection). It replaces an existing mask. The pixels lock stops it.
- History: each selection step stores the old selection as one background value plus the smallest rectangle that holds all other values (about 64 bytes for select all / none; 64 MiB for a fully feathered 4096² selection). `Edit::memory_bytes` counts it.
- Selection steps change `selection_version()` (a process-wide stamp) and not `revision()`, so the GPU composite cache stays valid. `mask_from_selection` changes the revision (it is a mask change).
- `.exdoc` does not store the selection.

### Outline tracing (`expose_core::buffer::trace_contours`)

- `trace_contours(image: &GrayImage, threshold, step, max_points) -> CoreResult<Vec<Contour>>`: interpolated marching squares. Values outside the image count as 0, so all contours are closed and stay inside the image. The first point is not repeated at the end.
- Saddle cells: the inside corners connect if the cell mean is at least the threshold.
- `step > 1`: each sample is the maximum of its `step` × `step` block, so thin lines do not vanish.
- Orientation: the inside is on the left of the walk direction on screen (outer outlines counter-clockwise on screen, holes clockwise).
- More than `max_points` points gives `CoreError::ContourTooLarge` (no truncated outline); the caller can retry with a larger step.

## Measurements (release, this machine)

- 4096² anti-aliased ellipse: 18 ms; with a 20 px feather: 200 ms (feather about 180 ms).
- `trace_contours` on a 4096² ellipse at step 1: 8.45 ms (12 000 points).

## Consequences

- Interior corners of a rectangle outline are cut by half a pixel (8 points for a rectangle inside the canvas). A pixel-edge tracer is a later option if the operator wants it.
- The feather is slow for interactive use at large sizes. Speed-up: blur only the band around the edges.
