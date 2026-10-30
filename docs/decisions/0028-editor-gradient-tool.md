# 0028 — Editor: gradient tool

- **Status:** Accepted
- **Date:** 2026-10-30
- **Decided by:** Claude (the operator gave Claude the order of the P1 editor tools, session 12)

## Context

P1 item in `FEATURES.md` 3.5: "Gradient tool (linear, radial, angle, reflected, diamond; gradient editor)". The paint bucket (ADR 0027) already fills a layer with the brush paint rule. The gradient uses the same rule, but the color changes per pixel. The work goes in two rounds, as in ADR 0027:

1. Crate (`expose-editor`: gradient math and `fill_gradient`) and UI (tool, options, gradient editor) in parallel.
2. Shell (`EditDto::Gradient`) after the crate PR is merged.

## Decisions

### Gradient (crate, `expose_editor::gradient`)

- **Stops.** A gradient has `2..=MAX_GRADIENT_STOPS` (32) stops. Each stop has a position `0..=1`, a straight color (linear Rec. 2020, finite), and an opacity `0..=1`. The positions must be in ascending order (equal positions are allowed: a hard edge). Before the first stop and after the last stop, the end stop applies.
- **Color interpolation.** Between two stops, the color is a linear interpolation of the **encoded** colors in the blend space of the document (`BlendSpace::encode` / `decode`; perceptual by default), and the opacity is a linear interpolation. Thus a black-to-white gradient looks even, as in 8-bit tools, and a document in linear blend space interpolates in linear light.
- **Shapes.** With the start point `p0`, the end point `p1`, `d = p1 - p0`, `L = |d|`, and the pixel center `p` (`x + 0.5`, `y + 0.5`), `u = dot(p - p0, d) / L²`, `v = cross(d, p - p0) / L²`:
  - linear: `t = u`
  - radial: `t = |p - p0| / L`
  - angle: `t` = the angle from `d` to `p - p0`, clockwise on screen, divided by 2π, in `[0, 1)`
  - reflected: `t = |u|`
  - diamond: `t = |u| + |v|`

  Then `t` is clamped to `0..=1`, and `reverse` gives `1 - t`. `L < 0.5` px is an error (`InvalidGradient`).
- **Paint.** On a pixel layer, the coverage is `selection * opacity * stop opacity(t)`, and the color is the gradient color at `t`. The paint rule, the blend mode, and the transparency lock are those of the brush and the bucket (`Mode::paint`, ADR 0022, ADR 0024). The layer does not grow.
- **Mask.** On a layer mask, each stop gives a mask value (the caller computes it; the shell uses the luminance rule of the brush mask target). The value is interpolated linearly; the coverage is `selection * opacity * stop opacity(t)`; `m = lerp(orig_m, value, coverage)`. The blend mode must be Normal.
- **API.** `Document::fill_gradient(layer, &GradientSpec, target) -> EditorResult<Option<(Edit, Rect)>>`, one undo edit of the changed tiles, `None` if nothing changed (as `fill_region`).
- **Errors.** Bad stops, a too-short line, `Dissolve`, or a non-Normal mode on a mask → new `EditorError::InvalidGradient(&'static str)`; locks, layer kind, and missing mask as for strokes.
- **No dither** in P1. Layers are f32 on the CPU and f16 on the GPU, so banding is small. Dither is a follow-up if the check finds banding in 8-bit exports.

### UI

- Tool `gradient`. `G` selects the last tool of the group; `G` again switches between the paint bucket and the gradient (the same rule as `M` and `L`).
- Drag from the start point to the end point. A line with end handles shows during the drag. Shift snaps the angle to 45° steps. Escape cancels. On release, the UI sends one `gradient` edit. A drag shorter than the click distance sends nothing.
- Options: shape (five buttons), the gradient (a preview swatch that opens the gradient editor, and a preset list), opacity, mode (layer modes without Dissolve; disabled for the mask target), reverse.
- Presets: foreground to background, foreground to transparent, black to white. The foreground/background presets follow the current colors.
- Gradient editor (dialog): a bar with the stops below it; click on the bar adds a stop (with the interpolated color); drag moves a stop; drag off the bar or Delete removes a stop (at least 2 stay); the selected stop has a color input, an opacity input (0–100 %), and a position input (0–100 %). The edited gradient stays in the session (one custom gradient, shared by all tabs).
- Paint target: pixels or the mask, as for the brush. A layer that is not a pixel layer with the pixels target: toast, nothing sent.

### Shell contract

`EditDto` gets one variant (JSON `type` `gradient`):

```ts
| { type: 'gradient'; layerId: number; target: 'pixels' | 'mask';
    start: [number, number]; end: [number, number];
    shape: 'linear' | 'radial' | 'angle' | 'reflected' | 'diamond';
    stops: { position: number; color: [number, number, number]; opacity: number }[];
    opacity: number; blendMode: BlendModeId; reverse: boolean }
```

- `start`, `end` are canvas coordinates (not rounded). `color` is gamma-encoded sRGB 0–1. For the mask target, the stop mask value is the Rec. 709 luminance of the encoded color (`0.2126 r + 0.7152 g + 0.0722 b`, the rule of `luminance` in `lib/editor/brush.ts`).
- History name: "Gradient". A gradient that changes nothing adds no history step.

### Implementation notes (round 1)

- Crate (PR #95): a 4097-entry table over `t` (nearest entry; `t` error ≤ 1/8192) holds the color and coverage, so a pixel needs no encode. Equal stop colors skip the encode and decode, so a flat gradient is exact. At equal positions the later stop applies. The paint bucket and the gradient share one fill core (`fill_buffer`, `fill_pixels_with`, `fill_mask_with` in `flood.rs`); `fill_region` results did not change. Release timings at 4096² (loaded machine): linear/radial/diamond 150–200 ms, angle ~245 ms, any shape with a non-Normal blend mode ~1.1 s (`PaintBlend::source` calls the transfer function six times per pixel; the bucket has the same cost).
- UI (PR #96): `G` keeps the action id `paintBucketTool` and now cycles bucket ↔ gradient (`cycleFill`). The options swatch and the dialog interpolate in gamma sRGB (the same as the perceptual blend space; a document in linear blend space shows a slightly different preview). Pointer cancel, a tool change, or a document change also cancel the drag.

### Implementation notes (round 2)

- Shell (PR #97): `EditDto::Gradient` with `GradientTargetId`, `GradientShapeId`, `GradientStopDto`; `apply_gradient` in `session.rs`; new `EditorEngineError::InvalidGradientInput` (`invalidInput`) for a stop color outside 0–1. The flood sample cache needs no explicit drop: it is keyed by the revision, and a gradient that changes a value gives a new revision.

## Reasons

- The brush paint rule in one place (`Mode::paint`) keeps the bucket, the brush, and the gradient the same on the same layer.
- Interpolation in the document blend space agrees with how the document blends layers.
- Shape math on pixel centers is deterministic and does not depend on the view.

## Consequences

- A new `EditorError::InvalidGradient` variant: the shell match in `src-tauri/src/editor/mod.rs` needs an arm (the orchestrator adds it in the crate branch).
- `G` becomes a group key (bucket, gradient).

## Open

- Dither.
- Gradient presets saved between sessions.
