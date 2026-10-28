# 0018 — Editor: layer masks in the UI and the mask view

- **Status:** Accepted
- **Date:** 2026-10-27
- **Decided by:** Claude

## Context

ADR 0016 sets the order of the editor tools: masks UI first, then selections, then brushes. The model already had `set_mask` and `set_mask_enabled`. The UI and the shell contract had no mask operations, and the GPU compositor could not show a mask. Three agents did the work (PRs #61, #62, #63).

## Decisions

### Contract additions (`editor/dto.rs` and `ui/src/lib/editor/api.ts`)

- `LayerInfo.maskEnabled: boolean` (`false` if the layer has no mask). `hasMask` stays.
- Edit `addMask { layerId, fill: "reveal" | "hide" }`: a mask of canvas size at offset (0, 0), all 1.0 with `MaskFill::White` (reveal) or all 0.0 with `MaskFill::Black` (hide). Any layer kind can have a mask. A second add is an error (`MaskExists`, `invalidInput`); the shell does not replace a mask silently.
- Edit `deleteMask { layerId }`: no mask gives `NoMask`.
- Edit `setMaskEnabled { layerId, enabled }`. Step names "Enable Layer Mask" / "Disable Layer Mask".
- Locks: the pixels lock (or `all`) stops add and delete; only the `all` lock stops the enable switch (same rules as the model).
- `editor_render` has an optional last argument `showMaskLayerId`. In the UI it is a field of `RenderArea`.

### Mask view (`GpuCompositor::render_mask_view`)

- Shows the mask values of one layer as an opaque gray image with the same view geometry as `render_view`. The gray value is the sRGB decode of the mask value, so the display encoder shows mask 0.5 as display gray 0.5. A disabled mask still shows its values.
- A kernel builds the gray canvas image from the packed mask upload, then the usual pyramid. The cache key is layer, mask version, offset, fill, and canvas size. `render_view` and `render_mask_view` share the output texture, and the composite view cache stays valid.
- The shell frees the mask view cache (`clear_mask_view_cache`) at the first normal render after a mask view.

### UI

- Layers panel: "Add mask" button (click = reveal all, Alt+click = hide all) and a "Mask" menu (add reveal/hide, enable/disable, delete, Show Mask).
- Mask chip on rows with a mask: plain click selects, Shift+click enables/disables, Alt+click shows the mask. A disabled mask shows a dim chip with a red X; a shown mask has a ring.
- Show mask: `\` key, Alt+click, or the menu. A "Mask: <layer name>" badge shows over the canvas. The view turns off when the selection changes, the layer loses its mask, or the document changes or closes.

## Consequences

- The chip is always white, because `LayerInfo` does not give the mask fill or a thumbnail.
- A frame that is in flight when the layer loses its mask can give one `NoMask` error toast.
- A mask edit rebuilds the full gray canvas and its pyramid while the mask view is on. Partial uploads come with brushes (ADR 0016).
- "Mask from selection" and the paint target switch (pixels or mask) wait for selections and brushes.
