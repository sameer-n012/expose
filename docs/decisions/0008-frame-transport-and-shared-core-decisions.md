# 0008 — Frame Transport, Shared Core Code, and Batch Catalog Writes

- **Status:** Accepted (webview measurement done on 2026-10-03, see "Webview measurement")
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

## Webview measurement

The operator ran the benchmark on 2026-10-03 (macOS webview, `tauri dev`, so the Rust side was a debug build). 60 frames at 2560×1707 (17.48 MB per frame), 6000×4000 synthetic source. Times are milliseconds per frame.

| Candidate | fps | transfer | draw | shell |
|---|---|---|---|---|
| ipc-rgba8-put (default) | 46.3 | 19.62 | 2.00 | 6.66 |
| ipc-rgba8-bitmap | 49.1 | 18.18 | 2.18 | 7.33 |
| scheme-rgba8-put | 49.8 | 18.18 | 1.87 | 6.28 |
| scheme-rgba8-bitmap | 49.8 | 17.97 | 2.13 | 7.42 |
| channel-rgba8-put | 97.2 | 8.02 (gap between frames) | 2.27 | 8.30 |
| scheme-jpeg-bitmap | 0.4 | 2779 | 12.72 | 2778 |
| scheme-png-bitmap | 0.2 | 4850 | 66.55 | 4847 |
| scheme-jpeg-img | 0.4 | 2792 | 3.50 | — |
| slider-ipc (request every 4 ms, 2 s) | 33.8 | 18.22 | 1.90 | 6.69 |
| slider-scheme | 33.8 | 17.42 | 1.97 | 6.72 |

Findings:

- The raw request/response transports reach 46–50 fps. The 30 fps target is met. `ipc` and `scheme` are equal within noise. `put` and `bitmap` are equal within noise.
- The transfer of 17.5 MB into the web process costs about 11–13 ms after the shell work. It is the largest cost.
- The `Channel` reaches 97 fps because it is a pipeline: the shell renders frame N+1 while the webview receives frame N. Request/response waits for each frame before it sends the next request. The estimate before the measurement (about 2 fps for the channel) was wrong.
- The encoded frames are very slow here because `tauri dev` builds the Rust code without optimization (CPU encode 2.8–4.8 s, against 36–115 ms in release). They are slower than raw frames in release too, so the decision does not change. The raw frame paths are GPU work and change little between debug and release.
- Slider simulation: 33.8 fps, lower than the sequential 46–50 fps. The 4 ms timer fired only 73 times in 2 s, so the main thread was busy for most of each frame (response handling, `putImageData`). Then the client often had no waiting request when a frame finished. Slider input events can be delayed by the same cause.

Decision: keep the raw `ipc::Response` transport as the default for now (it meets the target). The next step is to overlap render and transfer (pipelining) and to send fewer bytes, see `TODO.md` → preview-followups.

## Preview pipelining (PR #18)

Context: request/response waits for each frame before it sends the next request. The `Channel` push is faster because render and transfer overlap.

Decisions:

- **Two requests in flight, not channel push with acknowledge.** `PreviewClient` keeps up to 2 render requests open (`DEFAULT_MAX_IN_FLIGHT = 2`). The engine already has one running frame and one waiting slot (latest wins), so it gives the backpressure. A channel design needs a fetch and an acknowledge command per frame. Two in flight needs one call, and it works with the `ipc` and `scheme` transports.
- **Slot release at frame arrival.** A request slot becomes free when the frame body arrives, not after the draw.
- **No stale draw.** The client drops a frame whose sequence number is not above the last delivered one. `FramePresenter` (`ui/src/lib/develop/presenter.ts`) accepts only higher sequence numbers and draws at most once per display frame (`requestAnimationFrame`). It closes bitmaps that it does not draw. A rejected request does not use a sequence number.
- **Off-main-thread conversion.** The default draw mode is `bitmap` (`createImageBitmap`). The presenter uses `putImageData` when `createImageBitmap` does not exist.
- **Target size.** `preview_render` takes an optional `long_edge` (scheme: `long=<n>`). The shell rejects 0 and values above 8192, and clamps others to 64..=session size (`effective_long_edge`). The client computes the size from the canvas box and the device pixel ratio (`size.ts`), rounds it up to a multiple of 128, and caps it at the session size. When the size changes, the engine frees the old GPU source view before it makes the new one.
- **Canvas CSS.** The canvas fills its box with `object-fit: contain`, so a smaller frame scales up.

Consequences and open items:

- The engine has one waiting slot for all sessions. A request of one session can supersede a waiting request of another session (for example a prefetch). A slot per session is possible if this becomes a problem.
- A move to a display with a different pixel density does not change the target size until the box resizes.
- The webview measurement of the new benchmark modes (`pipelined-*`, `slider-ipc-pipelined`, `@half`) is not done yet. If `bitmap` does not draw faster than `put`, change `DEFAULT_PRESENT_MODE` to `put`.
