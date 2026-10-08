# 0012 — GPU compositor

- **Status:** Accepted
- **Date:** 2026-10-08
- **Decided by:** Claude (implementation details; planned work in the TODO list)

## Context

`expose-editor` had only the CPU reference compositor (`composite.rs`). The default blend space is perceptual (ADR 0005), and the CPU fast path is for linear Normal/Dissolve only. A 4096 x 4096 document with 10 layers takes about 15 s on the CPU. The editor canvas needs a fast compositor.

## Decisions

- **Place:** `expose-editor/src/gpu.rs` + `src/shaders/*.wgsl`. `expose-editor` now depends on `expose-gpu` and uses only its API (no wgpu). API: `GpuCompositor::new`, `composite_gpu` (premultiplied `rgba32float` `GpuImage` of the region size), `composite_gpu_image` (read-back to `WorkingImage`). The region check is the same as for `composite`.
- **Errors:** `EditorError::Gpu(SharedGpuError)`. `SharedGpuError` wraps `Arc<GpuError>`, so `EditorError` stays `Clone`.
- **Kernels:** four kernels: blend layer (also for isolated groups), adjust, pass-through mix, clear. One dispatch per layer. The sRGB encode/decode of the perceptual space runs per pixel in each kernel; there are no extra passes.
- **Buffers:** a pool of region-size `rgba32float` buffers. The steps alternate between two buffers, because a kernel output cannot also be an input. Each group level adds one or two buffers.
- **Masks:** `rgba32float` textures with four mask values per texel (same memory as a one-channel `f32` mask). Only the part inside the region is uploaded.
- **Curves:** a table of 4097 samples (`x = i / 4096`) for the color curve and the master curve, with linear interpolation in the shader. The other adjustments use the exact formulas.
- **Uploads:** each layer is uploaded once per call, only the part inside the region. There is no layer-texture cache yet, because layers have no content version. The kernels take the canvas position and size of each texture as parameters, so a cache keyed by `LayerId` + version can replace the upload step later.
- **Tolerance against the CPU reference:** `|gpu - cpu| <= tol * max(1, |cpu|)`, `tol = 5e-5` (largest measured error about 1.3e-5); divide mode `5e-4`. Modes with a step in the formula (color burn, color dodge, vivid light, hard mix, darker color, lighter color) can land on the other side of the step; the test accepts at most 1 sample in 1000 outside the tolerance for these modes.

## Consequences

- Benchmark (release, 4096 x 4096, perceptual, 10 layers): CPU 15.3 s; GPU warm 271 ms, of which about 245 ms is the upload and about 27 ms the kernels; first call 1.36 s (shader compile). A layer-texture cache would bring the composite to about 27 ms.
- Full `rgba32float` read-back is slow (about 1.1 s for 268 MB). The editor display must read back a small `rgba16float` or `rgba8unorm` frame, not the full working image.
- A layer-texture cache needs a per-layer content version (generation counter) in the document model.
- A curve end point that is not on a table sample can give an error up to about `slope / 16384`. The tests do not cover this case.
