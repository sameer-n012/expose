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

- `Document` keeps the selection that the last `deselect` removed. It is session state: it is not in the history and not in the `.exdoc` file.
- `Document::reselect()` sets that selection again (one undo step). With nothing stored, or a stored selection of a different canvas size, it gives `EditorError::NoSelection`. `Document::can_reselect()` lets the UI disable the command.
- UI key: `Mod+Shift+D`.

### Freehand lasso (UI only)

- A new tool `lasso`. The shell already accepts polygons with 3 to 100 000 points, so a freehand path is sent as a polygon.
- The UI adds a point only when it is at least one screen pixel from the last point, and stops adding points at 100 000.
- `L` selects the last lasso kind; `L` again switches between the freehand lasso and the polygon lasso (same rule as `M` for the marquees).

### Brush blend mode (crate)

- `BrushSettings.blend_mode` (default `Normal`) applies to paint strokes only. Erase and mask strokes with another mode are an input error.
- Per pixel: `B = decode(mode.blend(encode(Cb), encode(Cs)))` in the blend space of the document, then `Cs' = (1 − αb)·Cs + αb·B`, then the paint rule of ADR 0022 with `Cs'`. Thus a blend mode has no effect on transparent pixels, as in layer compositing. `Normal` keeps the old result bit for bit.

### Eyedropper (shell and UI)

- The shell samples on the CPU with `composite_straight(document, region)` for a 1×1, 3×3, or 5×5 region (or reads the selected layer), averages, and returns gamma-encoded sRGB. The GPU view cache is not used, so the canvas frame stays cached.
- UI: tool `I`; Alt+click with the brush samples to the foreground color.

### Color picker

- The foreground and background swatches use the native color input of the web view (the system color panel on macOS). A separate color dialog is not made now. The operator check decides if the native panel is sufficient for the P1 item.
