# 0003 — Foundation Crate Decisions

- **Status:** Accepted
- **Date:** 2026-10-02
- **Decided by:** Claude (implementation details)

## Context

The first versions of `expose-core`, `expose-gpu`, and `expose-catalog` were built in parallel (PRs #1, #2, #3). This record collects the technical decisions from that work, so that later work follows them.

## Decisions

### expose-core

- `Mat3` uses `f64`. Pixel loops and GPU code use `Mat3::to_f32()`.
- `WorkingImage` stores `Vec<[f32; 4]>`. It casts to `&[f32]` and `&[u8]` with no copy.
- I/O buffers are one generic type `Image<T: Sample>` (`u8`, `u16`) with a `ChannelLayout` field.
- Images with a zero width or height are an error.
- Transfer functions are odd functions for negative values, so that scene-referred negative values survive a round trip.
- Rec. 2020 output uses the BT.2020 OETF.
- Output gamut mapping is a clip per channel (P1 only, see ADR 0002).
- `DevelopSettings` uses `#[serde(default)]`. Older files load with defaults. `validate()` clamps finite slider values and rejects NaN, infinite values, bad tone curves, bad crops, and unknown versions.
- Tint scale: `Duv = -tint / 3000`. Positive tint is magenta. This value can change when the white balance stage is tuned.

### expose-gpu

- wgpu 30. No wgpu type is in the public API.
- No optional GPU features. Kernels read with `textureLoad` (float32 textures are not filterable in WebGPU core).
- The CPU-side API is always `f32`. `Rgba16Float` is converted on the CPU with the `half` crate.
- Upload and read-back move strips of at most 64 MiB.
- Kernels are WGSL source strings (`Kernel { name, source }`). Other crates can define kernels without a direct wgpu dependency.
- GPU errors are returned as `GpuError` values through wgpu error scopes. They do not panic.

### expose-catalog

- Times are signed Unix milliseconds (`i64`).
- Flag and color label are TEXT columns with CHECK constraints.
- SQLite `application_id` is set, so that the catalog never migrates a database that belongs to another program.
- All pending migrations run in one `IMMEDIATE` transaction.
- A folder links to its nearest ancestor in the catalog. Ancestors are not created up to the file system root.
- The store is `Send` but not `Sync`. The app shell wraps it in a `Mutex`.
- Develop settings are stored as versioned JSON. The catalog does not depend on the `DevelopSettings` type.

## Consequences

- Open items: WASM build check of `expose-core` (target not installed), capture time from EXIF, case-insensitive path comparison on macOS.
