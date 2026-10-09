# 0013 — Editor: first version

- **Status:** Accepted
- **Date:** 2026-10-08
- **Decided by:** Claude (the operator gave the choice of scope to Claude on 2026-10-08)

## Context

The editor crate has a layered document model, history, a CPU compositor, and a GPU compositor (ADR 0012). The Editor module of the UI is a placeholder. The first version must make the layer stack usable and set the transport between the shell and the UI. It must not fix decisions that are hard to change later (file format, tools).

## Decisions

### Scope

- **In:** open the selected library photo as a document with one pixel layer (the developed photo at full size); canvas view with zoom and pan; layers panel (tree, visibility, rename, opacity, fill, blend mode, order, delete, duplicate); adjustment layers (all four kinds can be added; brightness/contrast and hue/saturation have a property panel); undo/redo.
- **Out (later tasks):** tools (brush, selection, clone/heal, transforms, text), masks in the UI, levels/curves editors, layer groups in the UI (the tree shows them), saving.
- **Saving:** none in this version. A document lives in memory until it is closed. A native layered file format and the save back to the library are a separate decision.

### Shell

- An editor engine in `apps/desktop/src-tauri` owns the open documents, their `History`, and one `GpuCompositor`. It runs GPU work on its own thread with its own `GpuContext` (like the preview engine). Two GPU contexts cost a little memory; a shared context is a later optimization.
- The document base layer comes from the same full develop as export (CPU develop at full size). Memory: about 16 bytes per pixel on the CPU and the same on the GPU (about 380 MB each for 24 MP).
- Commands: `editor_open`, `editor_render`, `editor_edit`, `editor_undo`, `editor_redo`, `editor_close`. `editor_render` returns the `EXF1` binary frame of the develop preview (straight alpha, display color space), so the UI reuses `parseFrame`.
- `editor_render` takes a view (`originX`, `originY` = canvas coordinate at the top-left output pixel; `scale` = output pixels per canvas pixel; output size). The compositor keeps the full composite on the GPU and composites again only when the document revision changes. Pan and zoom run only the view kernel and the display encode. The read-back is the view size in 8-bit RGBA, never the full image.
- View filter: nearest neighbor at `scale >= 1` (pixels stay sharp), area average below 1.
- Edits are a tagged union (`EditDto`). Each edit is one undo step, except edits with the same `gesture` number in sequence: they merge into one step (one slider drag = one undo step).
- Layer order in the transport is bottom-first, the same as the document model. The UI reverses it for display.

## Consequences

- A slider drag needs only the kernels and a small read-back per frame, because layer textures are cached by content version.
- A document cannot be saved yet. Closing the document or the app loses the edits.
- The editor and the develop preview have separate GPU contexts.
