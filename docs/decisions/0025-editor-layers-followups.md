# 0025 — Editor: mask thumbnails, stale show-mask errors, lock and constant checks

- **Status:** Accepted
- **Date:** 2026-10-29
- **Decided by:** Claude

## Context

ADR 0017 and ADR 0018 left open items: the mask chip in the Layers panel is always white, a frame in flight when a layer loses its mask can give a `NoMask` toast, `AdjustmentPanel` does not obey the `all` lock, the dialog values are lost when the user cancels the unsaved-changes question, and the TS limits are copies of Rust constants. The work went in one round: shell and UI in parallel with one shared contract (PRs #84, #85).

## Decisions

### Mask thumbnail (shell, PR #84)

- `LayerInfo.maskVersion: u64 | null` is `Layer::mask_version()` when the layer has a mask, else `null`. The UI caches a thumbnail by it.
- Command `editor_mask_thumbnail(documentId, layerId, maxSize)`, `maxSize` in `1..=256`. The result is binary (little-endian): width `u32`, height `u32`, then one gray byte per pixel, rows top first (0 = hidden, 255 = shown).
- The thumbnail covers the whole canvas. Canvas pixels outside the mask buffer have the `MaskFill` value. The `enabled` flag does not change the values (the chip shows "off" with its own mark).
- Each thumbnail pixel averages at most 4 × 4 mask values over its canvas area. 4096² → 256²: 4.1 ms (release). It runs on the engine thread (`Job::MaskThumbnail`), like `editor_sample_color`.
- New error `EditorEngineError::InvalidThumbnailSize` (`invalidInput`). A layer without a mask gives `NoMask` (`invalidInput`).

### Mask chip (UI, PR #85)

- `EditorTab` keeps the thumbnails per layer with their `maskVersion`. The store fetches only when the version changes and no fetch for that version runs. Late results for an old version or a closed tab are dropped.
- Size: chip size (24 CSS px) × device pixel ratio, at most 256. The chip letterboxes the thumbnail with its aspect.
- An error keeps the plain chip and goes to `console.error` (no toast: the chip is a decoration).

### Stale show-mask errors (UI)

- The frame client ignores an error of a request with a `showMaskLayerId` when a newer request exists, or when the store no longer shows that mask (`showsMask` option). Other errors still give a toast.

### `all` lock in the adjustment panel (UI)

- With `editor.selectedLocks.all`, the sliders, number fields, reset buttons, and curve edits are disabled. The channel buttons stay enabled, because they only change the view.

### TS copies of Rust constants (UI)

- `lib/rustConstants.test.ts` reads the Rust sources and compares them with the TS copies: new-document limits (also `MAX_NAME_LENGTH` vs. `MAX_NAME_CHARS`), levels gamma range, brush size and spacing, feather limit, develop slider ranges (`ranges::*`), `MAX_CURVE_POINTS`, preview long-edge limits. A missing constant fails the test.

### Dialog values on cancel

- No change. Since the editor tabs (ADR 0023), a new document opens in its own tab, so the New document dialog never asks the unsaved-changes question.

## Consequences

- The chip shows painted masks; one small CPU job per mask change.
- A rename of a Rust constant breaks a UI test; update the test parser together with the constant.

## Open

- The develop defaults (`DEFAULTS`, `SHARPEN_DEFAULTS`, `CROP_DEFAULTS`) come from Rust `Default` code and are not checked. The levels `MIN_INPUT_SPAN` has no Rust constant.
- The thumbnail command was tested against the real shell only through the merged gates, not in the real webview.
- Optional levels input bar with draggable handles (not done).
