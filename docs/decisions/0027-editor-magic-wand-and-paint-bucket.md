# 0027 — Editor: magic wand and paint bucket

- **Status:** Accepted
- **Date:** 2026-10-29
- **Decided by:** Claude (the operator gave Claude the order of the P1 editor tools, session 12)

## Context

Two P1 items in `FEATURES.md` need a region of similar color around a clicked pixel: "Magic wand (tolerance, contiguous, sample all layers)" (3.4) and "Paint bucket" (3.5). Both tools use one flood-fill core in `expose-editor`. The work goes in two rounds:

1. Crate (`expose-editor`: flood core, `select_region`, `fill_region`) and UI (tools, options, contract types) in parallel.
2. Shell (`EditDto` variants, sample source, error mapping) after the crate PR is merged.

## Decisions

### Flood core (crate, `expose_editor::flood`)

- **Sample.** A `FloodSample` is a canvas-size, premultiplied, linear image. It comes from the composite of all visible layers (`FloodSample::composite`, CPU; or `FloodSample::from_composite_image` with a canvas-size premultiplied image, for example from `GpuCompositor::composite_gpu_image`), or from one pixel layer (`FloodSample::layer`; canvas pixels outside the layer are transparent).
- **Color distance.** Each pixel becomes four values on a 0–255 scale: the sRGB-encoded straight color multiplied by alpha, and alpha (`v = 255 * (enc(r)*a, enc(g)*a, enc(b)*a, a)`). The color is converted from the working space (linear Rec. 2020) to linear sRGB first and clamped to 0–1. A pixel matches if `max_i |v_i - seed_i| <= tolerance`. Thus all transparent pixels are equal, and a semi-transparent pixel is near to transparent.
- **Tolerance.** `0..=MAX_FLOOD_TOLERANCE` (255). Default 32. 0 matches only equal values (after the 0–255 scaling).
- **Contiguous.** With `contiguous`, the region is the 4-connected set of matching pixels that holds the seed (scanline fill, no recursion). Without it, the region is all matching pixels.
- **Anti-alias.** With `anti_alias`, a pixel in the region keeps 1, and a pixel outside the region gets the mean of the binary region in its 3×3 neighborhood (pixels outside the canvas count as 0). Thus only the outer 1 px ring gets partial values. Without it, values are 0 or 1.
- **Result.** `flood_region` returns a canvas-size `MaskImage`. A seed outside the canvas is an error.

### Magic wand (crate + shell + UI)

- `Document::select_region(&MaskImage, SelectionMode)` combines a canvas-size mask with the selection by the same rules as `select_shape` (no feather). One undo step.
- UI tool `magicWand`, key `W`. Options: tolerance (0–255, default 32), contiguous (default on), sample all layers (default off = the selected layer), anti-alias (default on). Modifier keys and the toolbar mode work as for the marquee (Shift add, Alt subtract, Shift+Alt intersect). A click outside the canvas does nothing.
- With "sample all layers" off, the selected layer must be a pixel layer. Otherwise the shell gives `invalidInput` and the UI shows a toast.

### Paint bucket (crate + shell + UI)

- `Document::fill_region(layer, &MaskImage, &FillSettings)` paints `FillSettings.color` (straight linear Rec. 2020) on a pixel layer with coverage `region * selection * opacity` (no selection = 1). It uses the paint rule of the brush (ADR 0022), the brush blend-mode rule (ADR 0024), and the transparency lock. The layer does not grow: only pixels inside the layer bounds change. The undo edit holds the original tiles of the changed area (one undo step). It returns `None` if no value changed (then no history step).
- Errors: pixels lock → `Locked`; not a pixel layer → `NotAPixelLayer`; `Dissolve` → new `EditorError::InvalidFill`; bad values → `OutOfRange`/`NonFinite`.
- UI tool `paintBucket`, key `G` (the gradient tool will later share `G` with a cycle, as `M` and `L`). Options: tolerance, contiguous, sample all layers, anti-alias, opacity (default 100 %), mode (layer blend modes without Dissolve). The color is the foreground color. The bucket paints only on layer pixels; with the mask as the paint target, the UI shows a toast (mask fill is a follow-up).

### Module keys

- In the Editor, `G` is the paint bucket, `E` the eraser, and `D` the default colors, so the global module keys `G`, `D`, `E` do not work there. New global keys that work in all modules: `Mod+Alt+1` = Library grid, `Mod+Alt+2` = Develop, `Mod+Alt+3` = Editor. `G`, `D`, `E` stay as they are outside the Editor.

### Shell contract

`EditDto` gets two variants (JSON `type` values `magicWand` and `fill`):

```ts
| { type: 'magicWand'; x: number; y: number; mode: SelectionModeId; tolerance: number;
    contiguous: boolean; antiAlias: boolean; sampleAllLayers: boolean; layerId: number | null }
| { type: 'fill'; layerId: number; x: number; y: number; color: [number, number, number];
    opacity: number; blendMode: BlendModeId; tolerance: number; contiguous: boolean;
    antiAlias: boolean; sampleAllLayers: boolean }
```

- `x`, `y` are canvas coordinates; the shell takes `floor`. A point outside the canvas, `layerId: null` with `sampleAllLayers: false`, or a layer that is not a pixel layer gives `invalidInput`.
- `color` is gamma-encoded sRGB 0–1 (as for strokes).
- History names: "Magic Wand", "Paint Bucket".
- The shell caches the composite sample per document and revision, so that a series of wand clicks (for example Shift+click to add) composites only once.

### Implementation notes (round 1)

- Crate (PR #91): keys are rounded to `u8` (exact compare; the tolerance is applied as `floor(tolerance)`); the sRGB encode uses a 65536-entry table. A size mismatch gives `InvalidRect`. `FloodSample::layer` uses the raw layer pixels (no mask, opacity, or blend mode). The brush and the fill share `Mode::paint`, so the paint math is in one place. Undo tiles are `STROKE_TILE_SIZE` tiles of the changed area. Release timings at 4096²: layer sample 75–145 ms (key conversion, about 6 ns per pixel), contiguous flood + anti-alias 3 ms, global flood 26 ms, large fill 40–50 ms.
- UI (PR #92): tool settings live in `EditorStore` (shared by all tabs). The wand takes its mode from the modifier keys only. The bucket refuses (toast) a mask paint target, a layer that is not a pixel layer, or no selected layer.

### Implementation notes (round 2)

- Shell (PR #93): `EditSession` keeps one `FloodSample`, keyed by the source (composite or layer number) and `Document::revision()`. A selection change keeps the revision, so a series of wand clicks builds the sample once; a fill or undo drops it. New `EditorEngineError::InvalidFlood` (`invalidInput`) for a bad point, a missing `layerId`, or a bad color. A fill that changes nothing adds no history step.
- Timing (release, 4096², 5 pixel layers): the first "sample all layers" click takes 1.37 s (CPU composite); a click on the cached sample takes 132 ms. A GPU composite readback (`GpuCompositor::composite_gpu_image` + `FloodSample::from_composite_image`) needs the edit path on the engine thread with the GPU; it is a follow-up.

## Reasons

- One core for both tools keeps the color rule the same: a region that the wand selects is the region that the bucket fills.
- Premultiplied encoded values make the distance well defined for transparent pixels, without a special case.
- The 0–255 tolerance scale is the scale that users know from 8-bit tools, and it is independent of the bit depth of the document.
- The anti-alias ring only goes outward, so a contiguous fill with anti-alias covers the full matching region and softens only its edge.

## Consequences

- A new `EditorError::InvalidFill` variant: the exhaustive match in `src-tauri/src/editor/mod.rs` needs an arm (round 2).
- The composite sample is a full-canvas image (256 MiB as f32 at 4096²). The shell keeps at most one per document.

## Open

- Mask fill with the bucket (paint target = mask).
- Wand sample size (point vs. 3×3 / 5×5 mean) — P1 uses one pixel.
