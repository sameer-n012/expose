# 0024 — Editor: reselect, freehand lasso, brush blend mode, eyedropper

- **Status:** Accepted
- **Date:** 2026-10-28
- **Decided by:** Claude

## Context

After the selections UI (ADR 0021) and the brushes (ADR 0022), four parts of P1 items in `FEATURES.md` 3.4 and 3.5 are missing: the freehand lasso, reselect, the brush blend mode, and the eyedropper. The work goes in two rounds:

1. Crate (`expose-editor`: reselect, brush blend mode) and UI (freehand lasso) in parallel.
2. Shell (commands and DTO fields) and UI (reselect, blend mode, eyedropper) in parallel, with one shared contract.

## Decisions

### Reselect (crate)

- `Document` keeps the selection that the last `deselect` removed (compact form). It is session state: it is not in the history, not in the `.exdoc` file, and document equality ignores it. Only `deselect` changes it; undo and redo of a deselect do not (PR #80).
- `Document::reselect()` sets that selection again (one undo step). With nothing stored, or a stored selection of a different canvas size, it gives `EditorError::NoSelection`. `Document::can_reselect()` lets the UI disable the command.
- UI key: `Mod+Shift+D`.

### Freehand lasso (UI only)

- A new tool `lasso`. The shell already accepts polygons with 3 to 100 000 points, so a freehand path is sent as a polygon.
- The UI adds a point only when it is at least one screen pixel from the last point, and stops adding points at 100 000.
- `L` selects the last lasso kind; `L` again switches between the freehand lasso and the polygon lasso (same rule as `M` for the marquees).
- A drag shorter than the click distance is a click (deselect in replace mode, as the marquee). A path with fewer than 3 points sends nothing (PR #79).

### Brush blend mode (crate)

- `BrushSettings.blend_mode` (default `Normal`) applies to paint strokes only. Erase and mask strokes with another mode, and `Dissolve` for any stroke, give the new error `EditorError::InvalidBrush` (shell: `invalidInput`). Dissolve is refused because the coverage rule of a stroke cannot give its per-pixel alpha pattern. The stroke reads the blend space of the document once, at `Stroke::begin` (PR #80).
- Per pixel: `B = decode(mode.blend(encode(Cb), encode(Cs)))` in the blend space of the document, then `Cs' = (1 − αb)·Cs + αb·B`, then the paint rule of ADR 0022 with `Cs'`. Thus a blend mode has no effect on transparent pixels, as in layer compositing. `Normal` keeps the old result bit for bit.

### Eyedropper (shell and UI)

- The shell samples on the CPU with `composite_straight(document, region)` for a 1×1, 3×3, or 5×5 region (or reads the selected layer), averages, and returns gamma-encoded sRGB. The GPU view cache is not used, so the canvas frame stays cached.
- Command: `editor_sample_color(documentId, x, y, size, source, layerId)` with `source` = `composite` or `layer` (a pixel layer; pixels outside the layer count as transparent). The mean is alpha-weighted. Result: `{ color: "#rrggbb" | null, alpha }`; `color` is null when the mean alpha is 0. Bad input gives `EditorEngineError::InvalidSample` (`invalidInput`). The sample does not commit an active stroke (PR #81).
- UI: tool `I` with sample size and source options; click sets the foreground color, Alt+click the background color. Alt+pointer-down with the brush samples all layers to the foreground color and starts no stroke (PR #82).
- UI reselect: `Mod+Shift+D` and a toolbar entry, disabled when `canReselect` is false. The brush Mode select lists the layer blend modes without Dissolve; the eraser and the mask target always send Normal.

### Color picker

- The foreground and background swatches use the native color input of the web view (the system color panel on macOS). A separate color dialog is not made now. The operator check decides if the native panel is sufficient for the P1 item.
