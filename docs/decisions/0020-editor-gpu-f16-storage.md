# 0020 — Editor GPU: rgba16float storage for layer textures

- **Status:** Accepted
- **Date:** 2026-10-27
- **Decided by:** Claude (operator decision on the tolerance, 2026-10-27)

## Context

The GPU compositor (ADRs 0012, 0014) used about 2.3–2.6 GiB of GPU memory for a 4096² document with 10 layers. An earlier trial with `rgba16float` layer textures gave a difference of 2.6e-4 from the CPU reference, which failed the 5e-5 test tolerance. The operator accepted a looser tolerance (2026-10-27). Implemented in PR #65.

## Decisions

| Texture | Format | Reason |
| --- | --- | --- |
| Layer pixels, masks | f16 | Most of the memory. Each value is read once per composite, so errors do not add up. |
| Composite buffers (pool), view composite, below snapshot | f32 | Each step reads and writes them; f16 errors would add up. The view composite and snapshot move from the pool without a copy. |
| View pyramid, mask view image and pyramid | f16 | Display only. |
| Curve tables, view output | f32 | Small; the view output format is part of the API. |

- All shader math stays f32. The CPU converts f32 to f16 at upload (`GpuImage::from_rgba_f32`).
- On the Apple GPU, kernels that store to `rgba16float` round toward zero. Kernels that write f16 (`mip_down.wgsl`, `mask_view.wgsl`) round to nearest with `quantizeToF16` before the store.
- Tests: the CPU-vs-GPU tests use values that are exact in f16 and keep 5e-5 for the composite math. A new test with plain f32 values measures the storage error: separable modes ≤ 8.8e-4 (tolerance 2e-3), hue/saturation/color/luminosity ≤ 3.0e-3 (tolerance 6e-3). Pyramid and mask view tests use `VIEW_TOL` = 5e-4.
- Dev-dependency `half` 2 (MIT OR Apache-2.0), for tests only; it was already in the tree through `expose-gpu`.

## Measurements (4096², 10 layers, release)

| | Before | After |
| --- | --- | --- |
| `cache_memory_bytes` | 2567 MiB | 1699 MiB |
| First `render_view` | 95–111 ms | 72–77 ms |
| First slider step | 72–80 ms | 61–64 ms |
| Next slider steps | 30–33 ms | 26–27 ms |
| Pan | 1.2–1.9 ms | 0.9–1.3 ms |

## Consequences

- The numbers in ADRs 0012 and 0014 for GPU memory are replaced by these.
- Larger errors are possible with strong adjustments (for example levels with a small gamma: up to 3.6e-3) and at near-gray pixels in the color mode. They come from f16 rounding of the input values.

## Open

- The rest of the memory is mostly the f32 pool, the view composite, and the below snapshot (3 × 256 MiB at 4096²). f16 working buffers need explicit rounding in every kernel; operator decision if wanted.
