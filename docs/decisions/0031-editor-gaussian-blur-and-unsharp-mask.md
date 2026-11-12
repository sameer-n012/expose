# 0031 — Editor: Gaussian blur and unsharp mask filters

- **Status:** Accepted
- **Date:** 2026-11-12
- **Decided by:** Claude

## Context

`FEATURES.md` 3.8 has two P1 filters: Gaussian blur and unsharp mask. Non-destructive filters (on embedded layers) are P2. So the P1 filters change the pixels of one layer (or its mask), with one undo step and a live preview in a dialog.

## Decisions

### Crate (`expose_editor::filter`, new file)

```rust
pub const MIN_FILTER_RADIUS: f32 = 0.1;    // pixels (the Gaussian sigma)
pub const MAX_FILTER_RADIUS: f32 = 1000.0;
pub const MAX_UNSHARP_AMOUNT: f32 = 5.0;   // 500 %
pub const MAX_UNSHARP_THRESHOLD: f32 = 255.0; // levels

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterSpec {
    GaussianBlur { radius: f32 },
    /// `amount` 0.01..=5 (1 = 100 %), `threshold` 0..=255 levels.
    UnsharpMask { amount: f32, radius: f32, threshold: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterTarget { Pixels, Mask }

/// A filter preview on one layer. Like a brush stroke: begin, apply
/// (again and again, always from the original), then commit or cancel.
pub struct FilterSession { .. }
impl FilterSession {
    pub fn begin(doc: &Document, layer: LayerId, target: FilterTarget) -> EditorResult<Self>;
    /// Filters the original content with `spec` and writes the result into
    /// the layer. Returns the changed canvas rectangle.
    pub fn apply(&mut self, doc: &mut Document, spec: &FilterSpec) -> EditorResult<Option<Rect>>;
    /// One undo edit (`PixelPatch` / `MaskPatch` of the changed area).
    pub fn commit(self, doc: &mut Document) -> EditorResult<Option<Edit>>;
    pub fn cancel(self, doc: &mut Document) -> EditorResult<()>;
}
impl Document {
    /// `begin` + `apply` + `commit` in one call.
    pub fn apply_filter(&mut self, layer: LayerId, target: FilterTarget, spec: &FilterSpec)
        -> EditorResult<Option<Edit>>;
}
```

- **Radius** is the Gaussian sigma in pixels. The kernel reaches to about 3 sigma.
- **Gaussian blur** works on premultiplied linear values (light mixes correctly, no dark fringes at transparent edges). Pixels outside the layer buffer repeat the edge pixel (clamp to edge), so a canvas-sized photo layer does not fade at its edges. The mask target blurs the mask values in the same way.
- **Unsharp mask** works on straight color encoded with the document `BlendSpace` (perceptual by default): `d = x − blur(x)`; where `|d| · 255 > threshold`, `y = x + amount · d`; alpha does not change. Then decode and premultiply with the original alpha. With the mask target, the same rule works on the mask values (clamped to `0..=1`).
- **Speed:** an O(1)-per-pixel method for large radii (for example three extended box passes or a recursive filter), an exact sampled kernel for small radii. Target: at most 400 ms at 4096² for any radius (release). The output is deterministic.
- **Selection:** `result = lerp(orig, filtered, selection)`. Only the bounding box of the selection changes; the filter reads the pixels around it.
- **Locks and errors:** pixels lock (or `all`, also on a parent group) → `Locked`; transparency lock → the alpha stays, only the color changes; pixel target on a layer that is not a pixel layer → `NotAPixelLayer`; mask target without a mask → `NoMask`; bad values → `OutOfRange` / `NonFinite`. A layer change outside the session → `StrokeConflict` (the same check as a stroke). No new `EditorError` variant.
- The session uses the per-layer change log, so the GPU cache patches only the changed area.

### Shell

- Commands (one active filter session per document):
  - `editor_filter_preview(documentId, layerId, target: "pixels" | "mask", filter) -> DocumentInfo`: starts a session (if none, or for another layer or target: the old one is cancelled first) and applies `filter`.
  - `editor_filter_commit(documentId) -> DocumentInfo`: one history step, "Gaussian Blur" or "Unsharp Mask" (the last applied filter).
  - `editor_filter_cancel(documentId) -> DocumentInfo`: restores the layer, no step.
  - `filter` JSON: `{ "type": "gaussianBlur", "radius": 2.0 }` or `{ "type": "unsharpMask", "amount": 1.5, "radius": 1.0, "threshold": 0 }` (`amount` is a fraction: 1.5 = 150 %).
  - Any other edit, stroke, undo, redo, history jump, save, or close **cancels** an active filter session first (a filter changes pixels only after OK).
- A preview call that comes while an older one waits replaces it (only the newest preview runs).

### UI

- Menu entries in the Editor ("Filter" menu or the layer menu): "Gaussian Blur…" and "Unsharp Mask…". A dialog with sliders and number fields (blur: radius; unsharp mask: amount 1–500 %, radius, threshold 0–255), a "Preview" check box (default on), OK, Cancel. Escape = Cancel, Enter = OK.
- The target is the current paint target (layer pixels, or the mask if the mask chip is selected). Not available for an adjustment or group layer without the mask target (the menu entries are disabled).
- The dialog sends a preview after a slider change (one request in flight, the newest value wins) and asks for a new frame after each reply. With "Preview" off, it cancels the session; OK then applies the filter with one preview + commit.
- The dialog stays at the side so the canvas stays visible; zoom and pan work while it is open; other tools do not.

## Reasons

- A session like a brush stroke keeps the live preview, the undo edit, and the conflict check in one place in the crate.
- Blur in linear light is the physically correct mix; sharpening in the perceptual space gives even halos in dark and light areas, as users expect from the threshold in levels.

## Consequences

- A filter on a full 4096² layer stores the original area for the session (up to 256 MiB) and the same size in the undo edit.
- Preview speed depends on the CPU filter time. A GPU filter path is a follow-up if the time is too long.
- Crate (PR #109): exact sampled kernel (cut at 3 sigma) below `EXACT_MAX_SIGMA` = 2.0; from 2.0, `BOX_PASSES` = 4 extended box passes (fractional end weights match the Gaussian variance exactly, so the blur changes smoothly with the slider; within 0.8 % of the step height and 4 % of the impulse peak). Running sums in `f64`. Vertical passes stream over rows. Clamp to edge through edge copies before the passes.
- The unsharp mask blur is alpha-weighted (`blur(a·x) / blur(a)`), so transparent neighbours do not pull colors to black. Encode and decode use lookup tables (relative error < 1e-6).
- The session copies the selection at `begin`. `apply` returns the area changed since the previous `apply`.
- Time at 4096² RGBA (release, one thread): blur 1.9 → 304 ms, 2 → 429 ms, 50 → 443 ms, 500 → 615 ms; unsharp mask radius 1 → 592 ms. This is above the 400 ms target. The preview runs only the newest value, so the dialog stays usable; SIMD or a GPU path is a follow-up.
- Shell (PR #110): `EditSession::{filter_preview, filter_commit, cancel_filter, has_filter}`; `edit`, `undo`, `redo`, `history_jump`, stroke begin, and the save snapshot cancel a filter first; a filter preview commits an active stroke first. A queued preview for the same document is replaced in place (the old reply gets the existing `superseded` error). A bad value on the first apply cancels the session. A commit with no change records no step. New `EditorEngineError::FilterNotActive` (`invalidInput`). A preview runs on the engine thread, so no frame renders during it (0.3–0.6 s at 4096²).
- UI (PR #111): Filter menu in the selection toolbar; `FilterDialogFrame.svelte` (lower right of the stage), `FilterField.svelte` (log-scale radius); `lib/editor/filter.ts` (`FilterClient`, `filterDto`); opening a dialog selects the hand tool; free transform and crop are blocked while it is open; a tab, layer, or paint-target change cancels it. The menu is disabled for the `pixels` or `all` lock (also for the mask target, as for a mask stroke).
- `filter::MAX_FILTER_RADIUS` (f32, 1000) is not re-exported at the crate root, because `transform::MAX_FILTER_RADIUS` (f64, 32) has the name.

## Open

- Non-destructive filters, other blurs, and the filter gallery are P2/P3.
