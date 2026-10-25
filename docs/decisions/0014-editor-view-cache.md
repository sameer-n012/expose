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
- **Below snapshot (editor-gpu-followups, PR #53, 2026-10-25):** `render_view` keeps one composite of the top-level layers below the lowest top-level layer that changed since the last view composite. The first step on a layer is a full composite that keeps the state below it; later steps start from the snapshot. The snapshot is used only if the keys of all layers below it are equal now. A key holds the layer ID, pixel and mask versions, all `LayerProps` and `Mask` fields (listed by name, so a new field is a compile error until it is added), the adjustment parameters, and for groups the isolation and the child keys; canvas size and blend space are also checked. A change inside a group puts the snapshot below the top-level group. When the composite reaches the snapshot position, the working buffer becomes the snapshot (no copy). `composite_gpu` does not use the snapshot. Counters: `view_partial_composites()`, `curve_uploads()`.
- **Curve table cache:** each curves layer keeps its table with the curves that made it; the table is uploaded again only when the curves change.

## Consequences

- Benchmark (release, 4096², 10 layers, perceptual): pan only 1.1–2 ms; opacity change, then view 61 ms (10 full-canvas kernel passes); cold composite 294 ms; CPU reference 14.8 s.
- Memory is high: 16 B/px for each cached pixel layer, 4 B/px for each mask, the pooled buffers, the full composite, and the pyramid. The 4096² benchmark document uses about 2.3 GiB of GPU memory. Possible later reductions: `rgba16float` layer textures (no longer an exact match with the CPU), and sparse tiles (editor-storage).
- After the below snapshot (PR #53): repeated slider step 62 → about 29 ms; the first step on a new layer is a full composite (70–79 ms). The snapshot adds at most one canvas-size texture (256 MiB at 4096²; measured 2311 → 2567 MiB). Next possible step: composite and rebuild the pyramid only in the changed area.
- `rgba16float` layer textures were measured (PR #53) and not used: the error against the CPU reference is about 2.6e-4 relative, five times the test tolerance (5e-5). The saving would be 128 MiB per canvas-size layer. A looser tolerance is an operator decision.
- At the right and bottom canvas edge, a pyramid texel can include transparent pixels outside the canvas, so edge pixels can be slightly more transparent at zoom below 1.
