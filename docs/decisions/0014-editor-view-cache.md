# 0014 — Editor versions, texture cache, and view rendering

- **Status:** Accepted
- **Date:** 2026-10-09
- **Decided by:** Claude (implementation details of ADR 0013)

## Context

The GPU compositor (ADR 0012) uploaded each layer on each call (about 245 ms at 4096²). The editor canvas (ADR 0013) needs fast frames for pan, zoom, and slider drags.

## Decisions

- **Versions:** one atomic counter for the process gives all stamps, so no stamp is ever used twice. `Document::revision()` gets a new stamp on each successful edit, undo, and redo. `Layer::pixel_version()` and `mask_version()` change on pixel and mask writes, a new mask, and their undo. Property changes, translate, and insert/remove keep them. `Document`/`Layer` equality ignores the stamps. Because stamps are unique in the process, one compositor can serve many documents.
- **Layer texture cache:** in `GpuCompositor`, keyed by `LayerId`, tagged with the version and the stored part. The cache stores the part of the layer inside the canvas (or inside the region, if the canvas part is larger than the device limit). At each composite, the compositor removes entries of deleted layers and entries with an old version. All compositor calls take `&mut self`.
- **View:** `render_view(ctx, document, &ViewSpec { origin_x, origin_y, scale, width, height })` returns linear Rec. 2020 `rgba32float` with straight alpha, at the view size. Pixels outside the canvas are transparent. The full composite is kept on the GPU and computed again only when `revision()` changes. Scale `>= 1`: nearest neighbor. Scale `< 1`: a pyramid of 2×2 means, then an exact area-weighted box of up to 3×3 texels on the level where one output pixel covers 1 to 2 texels.
- **History merge:** `History::push_merge(name, edits, key)` adds the edits to the last entry when the key is the same and the cursor is at the end. `push`, undo, redo, `jump_to`, `clear`, and `end_merge` end a merge.

## Consequences

- Benchmark (release, 4096², 10 layers, perceptual): pan only 1.1–2 ms; opacity change, then view 61 ms (10 full-canvas kernel passes); cold composite 294 ms; CPU reference 14.8 s.
- Memory is high: 16 B/px for each cached pixel layer, 4 B/px for each mask, the pooled buffers, the full composite, and the pyramid. The 4096² benchmark document uses about 2.3 GiB of GPU memory. Possible later reductions: `rgba16float` layer textures (no longer an exact match with the CPU), and sparse tiles (editor-storage).
- Faster slider steps later: composite only the changed area, or cache the composite of the layers below the changed layer.
- At the right and bottom canvas edge, a pyramid texel can include transparent pixels outside the canvas, so edge pixels can be slightly more transparent at zoom below 1.
