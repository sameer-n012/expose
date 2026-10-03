# 0008 — Frame Transport, Shared Core Code, and Batch Catalog Writes

- **Status:** Accepted (the webview measurement is still open, see "Open")
- **Date:** 2026-10-03
- **Decided by:** Claude (implementation details)

## Context

PRs #14–#16 added batch image info writes to the catalog, moved shared code into `expose-core`, and added the transport for develop preview frames from Rust to the UI. This record collects their technical decisions.

## Decisions

### Frame transport (`expose-gpu`, `expose-develop`, `apps/desktop`, PR #16)

Target: a 2560 px (long edge) preview that updates while a slider moves, at 30 fps or more.

Measured on an M3 Pro (release build, 2560×1707):

| Stage | Median |
|---|---|
| GPU develop | 2.10 ms |
| GPU display encode (8-bit sRGB) | 0.95 ms |
| RGBA8 read-back (17.5 MB) | 2.23 ms |
| Engine loop, 60 frames | about 4.3 ms per frame (233 fps) |
| RGBA f32 read-back (70 MB), for comparison | 8.58 ms |
| CPU `to_image` to 8-bit sRGB | 15.41 ms |
| JPEG q85 encode | 35.61 ms |
| PNG 8-bit encode | 114.93 ms |

Candidates (webview cost is an estimate until the operator runs the benchmark):

| Candidate | Rust side | Result |
|---|---|---|
| Tauri command, raw `ipc::Response` → `putImageData` | 4.3 ms | **Default.** Expected about 45–65 fps. |
| Custom URI scheme, raw RGBA8 → `fetch` → `putImageData` | 4.3 ms | Same path inside the webview. Kept as a switch. |
| Tauri `Channel` | 4.3 ms | Rejected: more cost per frame and no backpressure. |
| Encoded frames (JPEG or PNG) through a URI scheme | 60–140 ms | Rejected: the CPU encode costs 10–30 times more than the raw transfer, and JPEG is lossy. |
| Native GPU surface below a transparent webview | about 3 ms, no read-back | Not built. Feasible, but needs code for each platform and a surface API in `expose-gpu`. P3, or earlier if the real measurement is below 30 fps. |

- The display encode runs on the GPU: a kernel converts linear Rec. 2020 to 8-bit sRGB or Display P3 into an `Rgba8Unorm` texture. The kernel writes `code / 255`, so the stored codes are exact. The output is within 1 code of `WorkingImage::to_image`. Reason: a read-back of f32 data is 4 times larger, and the CPU conversion is slower than the full GPU frame.
- The 8-bit image is a texture, not a storage buffer, so that `run_kernel` stays the same.
- Frame format: a 32-byte header ("EXF1", width, height, sequence number, queue time, render time) and then the RGBA8 pixels. The header is in the body, not in HTTP headers, so the same body works for both transports.
- Engine: one render thread. It makes the GPU context at the first open. It keeps at most 2 sessions (about 160 MB of GPU memory each) and drops the least recently used. Reason: a limit on GPU memory, with space to prefetch the next photo.
- Latest wins: if a newer request replaces a waiting request, the waiting request fails with the error kind `superseded`. The UI client keeps one request in flight and sends only the newest waiting settings. It ignores `superseded` and frames of a closed session.
- Switch the transport with `localStorage.setItem('expose.previewTransport', 'scheme')` or `DEFAULT_TRANSPORT` in `ui/src/lib/develop/transport.ts`.
- The benchmark is a separate UI chunk. It loads only with `?bench=transport`.
- The CSP allows `preview:` and `http://preview.localhost` in `connect-src` and `img-src`.
- New dependencies of `expose-desktop`: `pollster` 0.4 (MIT OR Apache-2.0), `tokio` 1 with the `sync` feature only (MIT; Tauri already uses it).

### Shared code in `expose-core` (PR #15)

- `expose_core::math::MonotoneSpline` (f64) is the one monotone cubic spline. Develop re-exports it as `MonotoneCurve`. The editor `MonotoneCubic` is a thin wrapper that keeps its f32 API, checks, and output clamp. Reason: the develop version had the higher precision.
- `expose_core::buffer::GrayImage` is the general single-channel f32 image. The editor `MaskImage` wraps it, so the editor size limits and error types do not change.
- `Orientation` and its pixel transform moved to `expose_core::buffer`. `expose_raw::Orientation` is a re-export. `Orientation::apply` returns `CoreResult`.
- `Rect::intersection_signed` and `Rect::translated` are the signed helpers.
- `Chromaticity::to_uv` and `from_uv` are public.
- The Planckian locus fit has two pieces that meet at 4000 K with a kink. The two pieces now blend with a smoothstep weight over 3800–4200 K. Reason: one change fixes `temperature_tint_to_chromaticity`, its inverse, and the locus frame, because all of them use `planckian_chromaticity`. A test checks that the tint direction changes by less than 0.5° over 3990–4010 K. Without the blend, the test sees a jump of 1.85°.
- The float `Image<f32>` I/O move was not done: `expose-raw` had no copy, and `Sample` is sealed and needs `Eq`.

### Batch image info writes (`expose-catalog`, PR #14)

- `CatalogStore::set_image_infos(&mut self, items: &[(PhotoId, ImageInfo)]) -> Result<BatchInfoOutcome>`. `BatchInfoOutcome { written, unknown }`.
- `SqliteStore` writes all rows in one transaction with one prepared statement. The default trait implementation loops over `set_image_info` and is not atomic.
- An unknown photo ID is skipped and reported, not an error. Reason: import can race with a delete. Other errors (for example a zero dimension) roll back the full batch. Empty input is a no-op. For a duplicate ID, the last item wins.
- 50 000 rows (release, file database): 53 ms, against 415 ms for one call for each row.

## Consequences

- The develop module can show a 2560 px preview at a high frame rate, if the webview transfer is as fast as estimated.
- Each frame copies 17.5 MB into the web process. Rendering at the canvas display size would make frames smaller.
- Opening a session downsamples the full image on the render thread. This stops frames of other sessions for about 0.1 s.
- On Linux and Windows, if the webview uses message IPC instead of the custom protocol, `invoke` returns a number array, not an `ArrayBuffer`. This is not tested.
- The import in `src-tauri` does not use `set_image_infos` yet (TODO: import-batch-info).

## Open

- The operator runs the webview benchmark: `npm --prefix apps/desktop/ui run tauri dev`, then in the developer console `location.search = '?bench=transport'` (optional `&frames=120&long=2560`). If the measured rate is below 30 fps, change the default transport or move the native surface earlier.
