# 0032 — Editor: crop tool

- **Status:** Accepted
- **Date:** 2026-11-12
- **Decided by:** Claude

## Context

`FEATURES.md` 3.7: "Crop tool (with straighten; delete or hide cropped pixels)" (P1). A crop changes the canvas size of the whole document. With a straighten angle, it also turns all layers.

## Decisions

### Crate (`expose_editor::crop`, new file)

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropSpec {
    /// Center of the crop box in old canvas pixels.
    pub center: (f64, f64),
    /// New canvas size in pixels (size limits as for a document).
    pub width: u32,
    pub height: u32,
    /// Turn of the crop box in degrees, -45..=45 (positive = clockwise on
    /// screen). The content turns by -angle, so the box becomes level.
    pub angle: f64,
    /// `true`: pixels outside the new canvas are removed. `false`: they stay
    /// in the layer buffers outside the canvas (hidden), and a later move
    /// can show them again.
    pub delete_cropped: bool,
    pub interpolation: Interpolation,
}
impl Document {
    pub fn crop_canvas(&mut self, spec: &CropSpec) -> EditorResult<Edit>;
}
```

- Mapping: the center of new pixel `(u, v)` comes from the old canvas point `center + R(angle) · (u + 0.5 − w/2, v + 0.5 − h/2)`.
- **Angle 0 and whole-pixel box** (`center − size/2` is whole): only layer and mask offsets change (bit-exact, cheap undo). With `delete_cropped`, the buffers are trimmed to the new canvas (layers that become empty keep a 1×1 transparent buffer or the smallest valid size).
- **Angle ≠ 0 (or a fractional box):** each pixel layer and each mask (also masks of adjustment and group layers) is resampled with the same affine matrix as the free transform (reuse `transform.rs` code; empty layers only get new offsets). With `delete_cropped: false`, the full turned content is kept; with `true`, it is trimmed to the canvas.
- The crop box can reach outside the old canvas; the new area is transparent.
- Canvas operations ignore layer locks (they change the document, not one layer).
- Selection: cleared (deselect); the stored reselect state is cleared too.
- Undo: one `Edit` that restores the old canvas size, the selection, and each changed layer. For an offset-only crop, it holds only the offsets; for resampled or trimmed layers, the old images and masks.
- Errors: `OutOfRange` / `NonFinite` / `TooLarge` / `OffsetOverflow` as fitting. No new `EditorError` variant.

### Shell

- `EditDto::CropCanvas { centerX, centerY, width, height, angle, deleteCropped, interpolation }` through `editor_edit`, history step "Crop". After the edit, `DocumentInfo` has the new `width` / `height`; the compositor and view caches must handle a size change (check `GpuCompositor` and the view cache).

### UI

- Tool `crop` (key `C`). On activation, the crop box = the full canvas. 8 handles, drag inside moves, drag outside turns (straighten; Shift snaps to whole degrees), aspect presets (free, original, 1:1, 4:3, 3:2, 16:9) with a swap button, W/H fields, "Delete cropped pixels" check box (default on), an angle field. Rule-of-thirds overlay. Enter / Commit applies, Escape / Cancel resets.
- The box is drawn in canvas coordinates, also in a rotated view. The canvas outside the box is dimmed.
- After a crop, the view fits the new canvas.
- At angle 0 the UI rounds the box to whole pixels.

## Reasons

- One document-level edit keeps undo simple and exact. The offset-only path makes the common crop fast and lossless.
- Reuse of the free transform resampler gives one interpolation implementation.

## Consequences

- A straightened crop of many large layers takes time. Crate (PR #108), one 4096² layer cropped to 3800² at 3° (release, one thread): bicubic 1.37 s with delete, 1.62 s without; bilinear 1.07 s. The UI shows "Cropping…" and disables the controls during the edit.
- Crate details: `CropSpec::validate`, `CropSpec::new_to_old`, `MAX_CROP_COORD` = 1e6. The offset-only path runs when the old-to-new map is a whole-pixel translation (the free-transform test). Pixel layers resample only their visible content; masks resample the whole buffer and keep their fill value outside. With `delete_cropped`, each target is clipped to the canvas before the resample (so a large layer at 45° does not give `TooLarge`). A buffer with no part on the new canvas becomes 1×1 (transparent, or the mask fill). Offset-only changes keep the layer versions (the GPU layer key compares offsets). Undo restores the selection but not the reselect state.
- Shell (PR #110): `EditDto::CropCanvas` (reuses `InterpolationId`), step "Crop". The engine keeps `OpenDoc.canvas_size` and calls `GpuCompositor::clear_cache()` when the size differs at the start of a render, a save snapshot, or a stroke begin (this covers crop, undo, redo, and history jump). The session drops its flood and composite caches after a crop.
- UI (PR #112): `lib/editor/crop.ts` geometry, `CropOptions.svelte`; Escape resets a changed box, then leaves the tool; Alt+drag resizes about the center; arrows move 1 px (Shift 10 px); at angle 0 all four edges are whole pixels.

## Open

- Perspective crop and content-aware fill of the new area are P2.
