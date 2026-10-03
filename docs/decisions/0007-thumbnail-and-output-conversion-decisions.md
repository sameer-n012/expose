# 0007 — Thumbnail and Output Conversion Decisions

- **Status:** Accepted
- **Date:** 2026-10-03
- **Decided by:** Claude (implementation details)

## Context

PR #12 added the thumbnail cache for the library grid. PR #13 made the output conversion (`WorkingImage::to_image`) faster. This record collects their technical decisions.

## Decisions

### Embedded previews (`expose-raw`, PR #12)

- `embedded_preview` and `embedded_preview_min` use a local parser. They do not use `rawler`. Reason: `rawler` returns decoded pixels, not the JPEG bytes, and its CR3 code can panic. The local parser also works in the build without the LGPL feature.
- Supported sources: TIFF-type RAW files (including ORF and RW2), the EXIF thumbnail of JPEG files, the RAF header, and CR3.
- The parser accepts only baseline, extended, or progressive JPEG data (SOF0/1/2). Lossless JPEG sensor data is not a preview.
- `embedded_preview_min` selects the smallest preview with a long edge of at least the requested size. If there is no such preview, it selects the largest one. Reason: a smaller preview decodes faster.
- Truncated or bad input gives an error or `None`. It never panics.

### Thumbnail cache (`apps/desktop/src-tauri`, PR #12)

- Size and format: long edge 512 px, JPEG quality 85, sRGB.
- Location: `<app cache dir>/thumbnails/<id % 256 in hex>/<id>-<key>.jpg`. When a new file for a photo is written, the files with an older key for the same photo are removed.
- Cache key: FNV-1a 64-bit hash of the stored develop settings (JSON and schema version), the file size and modified time read from the disk (nanoseconds), and `THUMB_FORMAT_VERSION`. Reason: the hash is stable across Rust versions and platforms. The disk values detect a replaced file.
- Fast path: the embedded preview is used only if the develop settings are the defaults and the preview long edge is at least 512 px. The preview is decoded without orientation. Then the orientation of the file is applied, so that the image does not rotate two times.
- Full path: decode, area downsample to 1024 px, CPU develop pipeline, Lanczos-3 resize, JPEG encode. Reason: the CPU pipeline at 1024 px is fast, and it needs no GPU context.
- Worker pool: half the CPU cores (at least 1), on separate threads. The queue is last-in, first-out, so that the photos that the user scrolled to most recently are made first. Requests for the same key share one job. A panic in a job becomes an error response.
- Stored settings that do not parse: a warning is logged and the defaults are used.
- No invalidate command: the cache key changes when the settings change.

### Serving (`apps/desktop/src-tauri`, PR #12)

- An asynchronous custom URI scheme `thumb` serves the thumbnails. URL form: `thumb://localhost/<id>?v=<fileModifiedMs>` on macOS and Linux, `http://thumb.localhost/<id>?v=...` on Windows and Android.
- Responses: 200 with `ETag` (the cache key) and `Cache-Control: no-cache`; 304 if the ETag matches; 400 for a bad ID; 404 for an unknown photo or a missing file; 503 if the catalog or cache is not available; 500 with a short text body for other errors (also logged).
- Reason for ETag with `no-cache`: the URL carries only the file modified time. The webview checks again at each load, so a settings change shows without a new URL.
- The CSP `img-src` allows `thumb:` and `http://thumb.localhost`.
- The grid cell keeps the URL that failed (`failedUrl`) and shows the placeholder for it. Reason: grid cells are reused, so a hidden `<img>` would stay hidden for the next photo.

### Output conversion speed (`expose-core`, PR #13)

- The transfer function encode in `to_image` uses exact code thresholds. For each output code, the table holds the first linear input that gives this code with the exact function (inverse curve, then binary search).
- u8: a table indexed by the float bit pattern (256 buckets for each power of two). Each bucket holds at most one code change, so one compare gives the exact code. The code uses this form only when it is proven exact for the curve.
- u16: an interpolated table gives an estimate biased down by 0.25 code (measured error at most 0.0122 code). One compare with the threshold corrects it.
- Linear transfer functions skip the tables.
- The tables are built at each call (about 0.05 ms for u8, 2–3 ms for u16). There is no global state or cache. Reason: simple, deterministic, and WASM-safe.
- Determinism: the lookup uses only integer operations, compares, and separate multiply and add (no fused multiply-add). The tables use the platform `powf`, as the old code did.
- Clipping sends NaN, negative values, and -0.0 to +0.0 (same output codes as before; the table index needs a positive sign bit).
- Result: exact match with the old formula on this machine (exhaustive test over all `f32` values in 0..=1, all six curves, u8 and u16). 24 MP sRGB u8 conversion: 437 ms → 66 ms. 24 MP JPEG export: about 790 ms → about 200 ms.

## Consequences

- The library grid shows real thumbnails. RAW files with default settings use the camera preview, so the first view is fast.
- For RAW files with a custom white balance, thumbnails are not correct yet: the pipeline gets `SourceInfo::RENDERED`, but the decoded image already has the as-shot white balance. See `TODO.md` → thumbnail-followups.
- The full path makes a full-size f32 image (about 384 MB for 24 MP). This limits how many workers can run at the same time on small machines.
- The cache has no size limit yet.
- If another platform has a `powf` that is not monotonic, one output code can differ by 1 very near a code boundary. The tests accept at most 1 code and 0.1 % of samples.
- The exhaustive conversion test and the 24 MP timing test are `#[ignore]`. Run them by hand after changes to transfer functions.
