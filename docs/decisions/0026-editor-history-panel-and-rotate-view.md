# 0026 — Editor: history panel and rotate view

- **Status:** Accepted
- **Date:** 2026-10-29
- **Decided by:** Claude (the operator left the order of the P1 editor tools to Claude)

## Context

Two P1 items in `FEATURES.md` are open: "Undo/redo with history panel" (undo and redo exist, the panel does not) and "Zoom, pan, rotate view; fit and 100% view" (rotation is missing). Both build on code that exists: `expose_editor::History` already has named steps, `cursor()`, and `jump_to` (ADR 0005), and the editor view math is in `ui/src/lib/editor/view.ts`.

The work goes in one round with three parallel agents: shell history, UI history panel, UI rotate view.

## Decisions

### History (shell)

- `DocumentInfo.history: { steps: string[]; cursor: number }`. `steps` are the step names, oldest first. `cursor` is the number of applied steps (`0..=steps.length`). The list is at most `HISTORY_STEPS` (50) names, so it goes with each info.
- Command `editor_history_jump(documentId, step) -> DocumentInfo`. It ends an active stroke first (as undo does) and calls `History::jump_to`. Errors: `notFound` (document), `invalidInput` (`step` > number of steps).
- The dirty state and the GPU change log follow the same rules as undo and redo.

### History panel (UI)

- A "History" panel in the right column below the Adjustment panel. The first row is the start state (cursor 0, label "Start"); row `i` (1-based) is step `steps[i - 1]`. The current row is marked; rows after the cursor are dim (they can be redone).
- A click on a row jumps to that state. The jump goes through the same queue as undo and redo, so it runs after the queued edits.
- When the history drops old steps (more than 50), "Start" is the oldest kept state, not the opened file.

### Rotate view (UI only)

- `View` gets `rotation` (degrees, −180..180, 0 by default). It is view state per tab, not a document edit (no undo step, not saved).
- The canvas frame, the document box, and the canvas overlays sit in one container with a CSS rotation about the viewport center. The existing math (`canvasToScreen`, `screenToCanvas`, `frameBox`) stays in the unrotated space. Pointer input first rotates back about the viewport center.
- The frame request covers the bounding box of the rotated viewport (up to about 1.41 × the viewport side at 45°). The shell does not change.
- Tool "Rotate view" (`R` in the editor scope): a drag rotates about the viewport center, Shift snaps to 15°. Escape while dragging cancels. A rotation readout with a reset button shows while the rotation is not 0. Fit (`Mod+0`) and 100 % (`Mod+1`) keep the rotation.

## Reasons

- The history data already exists in the crate; sending it with the info keeps the panel in step with every edit with no extra command.
- A CSS rotation keeps the GPU compositor, the frame transport, and all canvas-space overlay code unchanged; the cost is a larger frame at odd angles.

## Consequences

- At 45°, frames have up to 2 × the pixels of the viewport.
- The frame shows rotated with the browser's image filtering (no own resampling).

## Open

- Thumbnails per history step and snapshots are not planned (P2 "snapshots" in `FEATURES.md`).
