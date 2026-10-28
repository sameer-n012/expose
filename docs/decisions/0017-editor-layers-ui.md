# 0017 — Editor: new layer, locks, new document, levels and curves editors

- **Status:** Accepted
- **Date:** 2026-10-27
- **Decided by:** Claude

## Context

The editor model (`expose-editor`) supports empty pixel layers, layer locks, new documents (`DocumentSpec`), and levels and curves adjustments. The UI and the shell contract did not have these items (task editor-layers-ui in `TODO.md`). Three agents did the work at the same time with one shared contract (PRs #57, #58, #59).

## Decisions

### Contract additions (`editor/dto.rs` and `ui/src/lib/editor/api.ts`)

- `LayerInfo.locks: LocksDto` (`transparency`, `pixels`, `position`, `all`). These are the layer's own locks, not the effective locks. The UI computes the effective locks (union along the ancestor groups) in `effectiveLocks` (`lib/editor/layers.ts`).
- `DocumentInfo.sourcePhotoId: number | null` — the library photo that the document came from.
- Edit `addPixelLayer { aboveLayerId }`: a transparent layer of canvas size at offset (0, 0). The placement rule is the same as for `addAdjustment`. The name is `Layer N`, where N is 1 + the largest N of the layers named `Layer <integer>` in the whole tree.
- Edit `setLocks { layerId, locks }`. It is always permitted (also with the `all` lock), and it is an undo step.
- Command `editor_new(spec, displaySpace)` with `NewDocumentSpec { name, width, height, resolutionPpi, bitDepth (8 | 16 | 32), background ("white" | "black" | "transparent") }`. The shell checks the name (1–120 characters, no `/`, `\`, `:`, or control characters, because it becomes a file stem). `Document::new` checks the size and the resolution. Errors are `invalidInput`. The blend space is perceptual (ADR 0005). A new document is not dirty.

### Save of a new document

`DocumentOrigin.name` keeps the name of a new document. A new document has no folder, so a plain save is `invalidInput` in the shell. The UI sends it to Save As (folder dialog) when `savedPhotoId` and `sourcePhotoId` are both `null`. Save As uses the name as the stem: `<name>-edit.exdoc` and `<name>-edit.tif` (ADR 0015 name rules). The `.exdoc` file already stores the resolution and the bit depth.

### UI

- Layers panel: "New layer" (`Mod+Shift+N`), four lock toggles for the selected layer, and a lock mark on rows with own locks. The `all` lock (own or from a group) disables opacity, fill opacity, and blend mode. The lock toggles stay enabled.
- New document dialog (`Mod+N`, global scope; also in the editor empty state): name `Untitled-N`, presets (1920×1080, 3840×2160, A4 at 300 ppi, 2048²), validation in `lib/editor/newDocument.ts`. The limits are copies of the Rust constants (`MAX_DIMENSION`, `MAX_PIXELS`).
- Levels editor: channel select, input black / gamma / input white, output black / white as sliders (values shown as 0–255). The UI clamps the values to the rules of `LevelsChannel::validate`.
- Curves editor: reuses `lib/develop/curve.ts`. The Rust `MonotoneCubic` uses the same spline and the same limits (2–32 points, x strictly increasing) as the develop tone curve.
- A slider or point drag is one undo step (gesture key). Add, delete, and reset are separate steps.

## Reasons

- Own locks in the transport keep the toggles exact; the effective locks are a pure function of the tree, so the UI can compute them.
- Routing the first save of a new document to Save As keeps one save path in the shell and needs no default folder setting.
- Reuse of the develop curve code keeps one spline implementation in the UI.

## Consequences

- A new pixel layer allocates a full canvas buffer (256 MiB at 4096² in f32). Sparse layer storage is a later task (editor-storage).
- The TS limits for new documents and levels are copies of Rust constants (as for the develop sliders, develop-ui-followups).

## Open

- `AdjustmentPanel` does not yet disable its controls for the `all` lock (the store has `selectedLocks`).
- While the new document dialog shows in the editor module, the auto-open of the focused library photo can still open that photo; the create step then replaces it.
- If the user cancels the unsaved-changes question, the dialog values are lost.
