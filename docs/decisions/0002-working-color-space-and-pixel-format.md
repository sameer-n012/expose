# 0002 — Working Color Space and Pixel Format

- **Status:** Accepted
- **Date:** 2026-10-02
- **Decided by:** Claude (technical detail of ADR 0001)

## Context

The develop pipeline must use a linear, wide-gamut, scene-referred working space (`FEATURES.md` 2.1). The editor composites layers with alpha. All crates must share one pixel representation, so that the RAW decoder, the develop pipeline, the GPU crate, and the editor can pass images without conversion code in each crate.

## Decision

1. **Working color space:** linear light, Rec. 2020 (ITU-R BT.2020) primaries, D65 white point.
2. **Working sample type:** 32-bit float (`f32`). Values above 1.0 and below 0.0 are allowed (scene-referred). Clamp only at output.
3. **Working layout:** interleaved RGBA, 4 × `f32` per pixel, row-major. This layout maps directly to the GPU format `rgba32float` (and `rgba16float` for previews).
4. **Alpha:**
   - Develop pipeline: alpha is always 1.0.
   - Editor layers: **premultiplied** alpha inside the compositor. Convert to straight alpha only at file I/O.
5. **I/O types:** `expose-core` also has 8-bit and 16-bit integer buffers (gray, RGB, RGBA) for decode and encode. Conversion to the working format happens once, at load.
6. **Output color spaces:** sRGB, Display P3, opRGB, ProPhoto RGB, and Rec. 2020. Each has its primaries, white point (Bradford adaptation to D65 where needed), and transfer function in `expose-core`.

## Reasons

- Rec. 2020 primaries contain almost all real surface colors and all output spaces except parts of ProPhoto RGB. Unlike ProPhoto RGB, its primaries are real colors, so saturation and hue math gives fewer artifacts.
- `f32` keeps highlight data above 1.0 for highlight recovery and exposure changes, and is the native float type on all GPUs.
- RGBA (not RGB) because GPUs have no 3-channel float texture format.
- Premultiplied alpha makes compositing and filtering (blur, resample) correct at layer edges.

## Consequences

- Memory: 16 bytes per pixel. A 50-megapixel image uses 800 MB at full resolution. The pipeline must therefore be tiled, and previews must use reduced sizes.
- Every decoder converts to linear Rec. 2020 RGBA `f32`. RAW decode converts camera RGB with the camera matrix to XYZ, then to Rec. 2020.
- Gamut mapping to smaller output spaces is required at export. A simple clip is acceptable for P1; better gamut mapping is a later item.
