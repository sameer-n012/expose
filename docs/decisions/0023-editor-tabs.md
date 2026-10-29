# 0023 — Editor: multiple open documents (tabs)

- **Status:** Accepted
- **Date:** 2026-10-28
- **Decided by:** Claude

## Context

`FEATURES.md` 3.1 has the P1 item "Multiple open documents (tabs)". The shell engine already keeps several documents (one `EditSession` per `DocumentId`, one shared `GpuCompositor`). The UI store (`EditorStore`) keeps one document: `openPhoto` and `newDocument` close the old document after the unsaved-changes question.

## Decisions

### Shell

- `DocumentInfo` gets `name: string`: the title of the document for the tab. For a photo: the file stem of the photo. For a new document: the name of the new-document spec. After a save: the file stem of the saved layered file. If none of these exists: "Untitled". The value comes from `EditSession::name` (it uses `DocumentOrigin.name` and `origin.saved`). The rest of the shell does not change (PR #76).

### UI store

- The store splits into two classes:
  - `EditorTab` (one per open document): `photoId`, `info`, `status`, `loadError`, `selectedId`, mask target, paint mask target, `expanded`, `view`, `fit`, `pending`, `saving`, and the edit queue (chain, live edits, gesture, generation, running save).
  - `EditorStore` (shared): the tab list `tabs` and `activeTab`, the viewport, the tool, the brush and eraser settings, the colors, the feather settings, and the dialogs.
- The store keeps its getters and methods (`info`, `view`, `edit`, `undo`, `save`, ...). They work on the active tab, so the panels do not change. An edit, a save, or an answer that arrives for a tab that is not active goes to that tab, not to the active tab.
- `openPhoto(photoId)` and `newDocument(spec)` open a new tab and activate it. They do not ask about unsaved changes. If a tab already shows the photo (`photoId` or `info.savedPhotoId` is the same), `openPhoto` and `editInEditor` activate that tab.
- `closeTab(tab)` asks the unsaved-changes question for a dirty tab, then closes the document in the shell. After a close, the tab to the right becomes active (else the tab to the left).
- Window close: the guard is installed while any tab is dirty. On a close request, each dirty tab becomes active in turn and asks. Cancel stops the close.
- Automatic open of the focused library photo (`openTarget`): only when the Editor module is active, no tab is open, the New document dialog is closed, and the user did not close the last tab. Closing the last tab stops the automatic open until the user leaves the Editor module and comes back.
- A tab switch cancels an active stroke and ends the gesture. The selection outline and the frame client follow the `documentId` of the active tab. A tab that has `fit` refits to the current viewport when it becomes active.

### UI view

- A tab strip above the canvas: one tab per document with the name, a dirty marker, and a close button. Click activates, middle-click closes. A tab that loads shows "Loading…"; a tab that failed to open shows the error in the canvas area and can be closed.
- The right panels (`LayersPanel`, `AdjustmentPanel`) re-mount on a tab switch (`{#key}` on the tab id), so open menus, rename fields, and drag values of one document do not show for another.
- New tabs go at the end of the strip. A tab that failed to open does not match its photo: opening the photo again makes a new tab (a retry).
- Gesture numbers are counted per tab. The unsaved-changes question names the tab.
- Shortcuts (scope `editor`): `Mod+W` closes the active tab; `Ctrl+Tab` and `Ctrl+Shift+Tab` activate the next and the previous tab.

## Consequences

- The compositor cache is keyed by layer ID, and the layer IDs of two documents can be the same. A tab switch can upload the layers of the new tab again (the version stamps keep the result correct). Follow-up: key the cache by document and layer.
- Each open document holds its layers in memory. There is no limit on the number of tabs.

## Open

- On macOS, the default application menu can take `Cmd+W` ("Close Window") before the web view sees it. The operator check confirms the key. If the menu takes it, replace the menu item in the shell.
