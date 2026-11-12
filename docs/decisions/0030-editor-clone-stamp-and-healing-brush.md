# 0030 — Editor: clone stamp and healing brush

- **Status:** Accepted
- **Date:** 2026-11-12
- **Decided by:** Claude

## Context

`FEATURES.md` has two P1 editor tools that are not done:

- Clone stamp (aligned / non-aligned, sample current / all layers).
- Healing brush (source-based, texture / tone blend).

Both tools paint with a brush and copy pixels from a source position. The brush engine (ADR 0022) already has dab placement, coverage, tiles, undo, selection, and locks. This ADR records how the two tools use it.

## Decisions

### Crate (`expose_editor::brush`)

- The two tools are new stroke kinds of the existing `Stroke`. They use the same dab placement, dab shape, coverage rule (`c = min(c + (1 − c)·k, opacity)`), tiles, undo edit (`PixelTiles`), selection, and conflict check.
- New public types:

  ```rust
  /// Where a clone or heal stroke reads its source pixels.
  pub enum CloneSource {
      /// The stroke layer itself, as it was at the start of the stroke.
      Layer,
      /// A canvas-sized image (premultiplied linear Rec. 2020), for
      /// example the composite of all visible layers.
      Image(std::sync::Arc<WorkingImage>),
  }

  /// What a clone or heal stroke does.
  #[derive(Clone, Copy, PartialEq, Eq)]
  pub enum CloneKind { Clone, Heal }

  pub struct CloneSpec {
      pub kind: CloneKind,
      /// Source position = destination position + offset (canvas pixels).
      pub offset: (i32, i32),
      pub source: CloneSource,
  }

  impl Stroke {
      pub fn begin_clone(doc: &Document, layer: LayerId, spec: CloneSpec,
                         settings: BrushSettings) -> EditorResult<Self>;
  }

  /// Largest absolute value of a clone offset component (canvas pixels).
  pub const MAX_CLONE_OFFSET: i32 = 100_000;
  ```

- **Whole-pixel offset.** The offset is an integer, so a clone is an exact copy (no resampling blur). The UI rounds the pointer positions.
- **Source snapshot.** The stroke reads the source as it was at the start of the stroke. With `CloneSource::Layer`, a source pixel comes from the original values of its stroke tile if the tile exists, else from the layer buffer (it did not change). Thus the stroke never copies its own output, and no full copy of the layer is necessary.
- **Source outside the source area.** A destination pixel whose source pixel is outside the source (outside the layer buffer, or outside the canvas for `Image`) does not change.
- **Clone result.** `result = lerp(orig, src, c)` (premultiplied). With the transparency lock: `rgb = lerp(orig_rgb, src_straight · orig_a, c)`, alpha stays.
- **Heal result.** During the stroke (`add_points`), a heal stroke shows the clone result. `end` runs the heal pass on the touched area and then makes the undo edit:
  - `R` = the pixels with coverage `c > 0` and a source pixel. `S` = source, `D` = destination tone: the layer original for `CloneSource::Layer`, the source image at the same position (no offset) for `CloneSource::Image` (so that a heal on an empty layer over a photo matches the photo).
  - The heal works per channel on straight color encoded with the document `BlendSpace` and on linear alpha.
  - Membrane `M`: the harmonic function in `R` (`ΔM = 0`) with `M = D − S` on the pixels next to `R`. Result: `H = S + M`, then decode, clamp alpha to `0..=1` and color to `≥ 0`, premultiply, and `result = lerp(orig, H, c)` (transparency lock as for clone).
  - Solver: deterministic multigrid (or a coarse-to-fine solve), not plain Jacobi. Target: at most 300 ms for a region of 1 MP (release).
  - Boundary pixels without a source pixel, and pixels outside the layer buffer, use a zero-gradient (Neumann) condition.
- Errors: `NotAPixelLayer` for a layer that is not a pixel layer; `Locked` for the pixels lock; `InvalidBrush` for a blend mode other than Normal; `OutOfRange` for an offset larger than `MAX_CLONE_OFFSET`; `InvalidBrush` if an `Image` source does not have the canvas size. No mask target (the tools change layer pixels only). No new `EditorError` variant.

### Shell

- `StrokeTargetDto` gets two variants:

  ```jsonc
  { "type": "clone", "offset": [dx, dy], "sampleAllLayers": false }
  { "type": "heal",  "offset": [dx, dy], "sampleAllLayers": true }
  ```

  `offset` values must be whole numbers in ±100 000, else `invalidInput`. The same `editor_stroke_begin/points/end/cancel` commands carry the strokes. History step names: "Clone Stamp", "Healing Brush".
- `sampleAllLayers: true` gives `CloneSource::Image(composite)`. The session caches the composite (`Arc<WorkingImage>`) by document revision, so a second stroke without a change between them does not composite again.

### UI

- Tools `cloneStamp` (`S`) and `healingBrush` (`J`). The options bar: size, hardness, opacity, flow, spacing, pressure (the brush controls, without Mode), plus "Aligned" (default on) and "Sample: Current layer / All layers" (default current layer).
- Alt+click sets the source point (canvas pixels) for the active tab. Without a source point, a stroke gives a toast "Alt+click to set the source point".
- Offset: at the first stroke after a new source point, `offset = round(source − strokeStart)`. With "Aligned", later strokes use the same offset. Without "Aligned", each stroke computes the offset again from the source point.
- Overlay: a crosshair at the source position (`pointer + offset` while the pointer is on the canvas and an offset is known; else at the source point). It turns with the rotated view.
- With the mask as the paint target, a stroke gives a toast and does not start.

## Reasons

- One stroke engine keeps coverage, undo, locks, and selection the same for all brush tools.
- A heal pass at the end of the stroke gives an exact boundary for the whole stroke. A heal per dab would show seams between dabs.
- A whole-pixel offset gives exact copies and makes the tests simple.
- The source snapshot stops repeated patterns where a stroke crosses its own source area.

## Consequences

- A heal stroke shows the clone result while it paints and changes to the healed result when it ends. `Stroke::end` of a heal stroke changes the layer again (new pixel version, revision, and change log entry), so the caller renders after it.
- Crate (PR #104): paint dabs now run row by row through a small rule interface. A clone stroke reads one source row before it writes that row, and every changed pixel is in a tile that holds its original value. Thus each read sees the layer as it was at the stroke start.
- Heal solver (`heal.rs`): conjugate gradients with one multigrid V-cycle as preconditioner (2×2 blocks, red-black smoothing, a dense solve at ≤ 256 cells, coarse correction scaled by 1.6), tolerance 1e-4 (largest difference against 1e-7: 4e-4). Single-threaded with a fixed order, so the result is deterministic. A connected part of the stroke without a known edge value stays a plain clone.
- Heal time for a 1 MP region (release): about 348 ms, above the 300 ms target (solver iterations 180 ms, final encode/decode write 75 ms). Possible speed-ups: an encode lookup table, a lighter first level, threads.
- The heal pass works on the bounding box of the stroke, so a long diagonal stroke on a large canvas uses memory for the full box.
- "All layers" needs a composite at the stroke start. The shell (PR #105) builds it on the engine thread with `GpuCompositor::composite_gpu_image` and keeps it in the session (`CompositeCache { revision, image }`) while the revision does not change. The cache holds about 256 MiB at 4096² until the next all-layers stroke after a change, or the close of the document. Shell types: `StrokeKind::{Brush, Clone}`, `EditSession::stroke_begin_kind` (the composite build is a closure, so the tests run without a GPU), `StrokeTargetDto::to_kind`.
- UI (PR #106): both tools share one set of brush settings (`editor.clone`, defaults size 40, hardness 0.5, spacing 0.15) plus `cloneAligned` and `cloneSampleAll`, shared by all tabs. The source point and the offset are per tab (`EditorTab.cloneState`). Without "Aligned", the offset clears after each stroke. Offset logic: `lib/editor/clone.ts` (`strokeOffset`, `withSource`, `sourceMarker`).

## Open

- Clone source panel (several sources, scale, rotation, overlay preview) and the spot healing brush are P2.
- A live heal preview during the stroke (heal per batch) is a follow-up if the jump at the end is disturbing.
