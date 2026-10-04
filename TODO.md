# TODO

Work tracker for Expose. `FEATURES.md` is the master feature list. This file tracks the work in progress and partial work.

No `FEATURES.md` item is marked `[x]` yet: the libraries exist, but no feature is available in the UI with tests from end to end.

## In progress

- **export-v1**, split in two agents that share one command contract (`export_start` / `export_cancel`, `ExportEvent` channel): export-backend (`feat/export-backend`, `src-tauri`) and export-ui (`feat/export-ui`, `ui`).

## Next (suggested order)

Model hint: **S** = easier task (use the sonnet model), **O** = hard task (use the default model).

1. [ ] **export-v1** (S/O) — encode API is done (PR #11). Remaining: export dialog in the UI (format, quality, color space, size, destination, file naming), Tauri command that runs decode → develop → resize → `encode_path` on a worker, progress + errors. FEATURES 5.1 P1.
2. [ ] **develop-panels** (S) — Tone curve, Detail (sharpening), and crop/straighten UI for the existing `DevelopSettings` fields; histogram (needs a shell command).
3. [ ] **editor-gpu / editor-ui** — later: GPU compositor that matches the CPU reference, then the editor canvas UI.

### Follow-ups from merged work

- [ ] **develop-ui-followups** (S) — the TS slider ranges in `ui/src/lib/develop/settings.ts` are copies of the Rust constants (add a test or generate them); the shell drops unknown settings fields (ADR 0009).

- [ ] **bench-regression** (O, agent `feat/bench-regression`) — benchmark candidates that run after the pipelined ones are 3–15× slower than on 2026-10-03 (ADR 0008, "Webview measurement after pipelining"). Most likely background load on the machine (operator). Scope: a short leak audit of `FramePresenter`/`PreviewClient`, and benchmark options (`only=`, reverse order) so the operator can re-check on a quiet machine.
- [ ] **preview-followups** — the slider simulation is limited by main-thread time (timer ticks), not the transport; one waiting slot per session in the engine if a prefetch supersedes a visible request; update the target size when the display pixel density changes; move the full-size downsample at session open off the render thread (it stops other sessions for about 0.1 s); check `invoke` binary responses on Linux/Windows (message IPC fallback gives a number array); native GPU surface if the measurement is below 30 fps (P3).
- [ ] **core-followups** (S) — `Rect::translated` is not used yet (editor `add_offset` works on tuples); the Planckian locus also has a piece boundary at 2222 K (y fit) with a small kink.
- [ ] **gpu-api** (O) — kernels with different input/output size (GPU downsample), multiple inputs / storage buffers, parameter buffer reuse, double-buffered read-back, `WorkingImage` at the API boundary, render graph.
- [ ] **raw-real-samples** (operator + S) — run `EXPOSE_RAW_SAMPLES=/path cargo test -p expose-raw -- --ignored real_samples --nocapture` on real camera files and fix what fails. Then: forward matrices, BaselineExposure, DNG black level origin check, multi-threaded/tiled demosaic.
- [ ] **develop-tuning** — tune slider strengths and zone centers on real photos; local (edge-aware) tone zones; RAW white balance as camera-space gains.
- [ ] **editor-storage** — sparse tiled layer storage; history memory budget; clipping masks; layer effects.
- [ ] **ui-polish** (S) — save panel layout between sessions; Tab shortcut conflicts with keyboard focus navigation; refresh shortcut tooltips after a remap.
- [ ] **ui-catalog-followups** (S) — search box is a placeholder; per-request version for optimistic rollback (two quick changes can restore an old value); folder count is direct-only, so it differs from the grid total with "include subfolders".
- [ ] **thumbnail-followups** — RAW with non-default white balance: give the pipeline the as-shot `SourceInfo` (from `DecodeInfo.raw.multipliers`), not `SourceInfo::RENDERED`; apply crop/straighten when a geometry stage exists; reduce memory (scaled JPEG decode, or decode to 8-bit, instead of a full-size f32 image); cache size limit and removal of thumbnails of deleted photos; check the `thumb` scheme, CSP, and 304 handling in a real webview.
- [ ] **encode-followups** — copy EXIF/XMP/IPTC into exported files; gamut mapping instead of clipping; JPEG optimized Huffman tables, progressive mode, gray output; TIFF ICC tag type UNDEFINED instead of BYTE.
- [ ] **catalog-paths** (S) — case-insensitive path comparison on macOS; Windows paths in tests.
- [ ] **wasm-check** — `expose-core` must compile for `wasm32-unknown-unknown`. The target is not installed (operator). `expose-raw` must build for WASM with `--no-default-features` (no `rawler`).
- [ ] **lgpl-relink** — before the first public release, make the distributed build let users relink with a modified `rawler` (ADR 0004).

### Open operator decisions

- Editor default blend space: linear (current) or perceptual (ADR 0005, "Open").
- JPEG encoder: keep the local baseline encoder, or approve `jpeg-encoder` (licence "(MIT OR Apache-2.0) AND IJG") (ADR 0006, "Open").

## Done

- [x] **develop-ui** (PR #19) — commands `get_develop_settings`/`set_develop_settings`; `DevelopStore` (debounced save, flush on photo/module change); Develop view with `PreviewCanvas` and the Basic panel (white balance, exposure, contrast, highlights, shadows, whites, blacks; per-slider reset, Reset all); thumbnails refresh after a save. ADR 0009.

- [x] **preview-pipelining** (PR #18) — `PreviewClient` keeps 2 requests in flight; `FramePresenter` draws with `createImageBitmap` once per display frame and never draws a stale frame; optional target long edge (`longEdge`, 64..=session size, 128 px steps); new pipelined benchmark modes; PNG note fixed. ADR 0008, "Preview pipelining". Webview measurement (2026-10-04): 97.7 fps stream (was 48.3), 232 fps at 1280 px; operator: sliders feel good, thumbnails update after an edit.

- [x] **import-batch-info** (PR #17) — import stores image info with one `set_image_infos` call (one transaction, one lock). Items with a zero width or height are filtered out before the batch and reported as import issues. Unknown ids and a failed batch give one issue per photo. The fake-file import test asserts its metadata errors.

- [x] **frame-transport** (PR #16) — GPU display encode to 8-bit sRGB/Display P3 (`expose_develop::display`), `Rgba8Unorm` in `expose-gpu`, preview engine (one render thread, ≤ 2 sessions, latest wins), commands `preview_open`/`preview_render`/`preview_close` (raw `ipc::Response`), `preview://` scheme, `PreviewClient` + `PreviewCanvas`, benchmark at `?bench=transport`. Rust side about 4.3 ms per 2560 px frame. ADR 0008.

- [x] **core-additions** (PR #15) — `expose_core::math::MonotoneSpline` (shared by develop and editor), `buffer::GrayImage` (editor `MaskImage` wraps it), `buffer::Orientation` (moved from `expose-raw`, re-exported there; `apply` returns `CoreResult`), `Rect::intersection_signed`/`translated`, public `Chromaticity::to_uv`/`from_uv`, Planckian locus blended over 3800–4200 K (continuous tint direction). ADR 0008.

- [x] **catalog-batch-info** (PR #14) — `CatalogStore::set_image_infos` → `BatchInfoOutcome { written, unknown }`; one SQLite transaction; unknown IDs skipped and reported; other errors roll back. 50 000 rows: 53 ms vs. 415 ms. ADR 0008.

- [x] **to-image-speed** (PR #13) — exact encode lookup tables in `to_image` (`buffer/encode_lut.rs`); output identical to the old formula; 24 MP sRGB u8 437 → 66 ms, 24 MP JPEG export ~790 → ~200 ms. ADR 0007.

- [x] **thumbnails** (PR #12) — `expose_raw::embedded_preview{,_min}` (TIFF-type RAW, JPEG EXIF, RAF, CR3; no `rawler`); thumbnail cache in `src-tauri/src/thumbnails.rs` (512 px JPEG, FNV-1a key, LIFO worker pool with de-duplication); `thumb` URI scheme with ETag/304; `thumbnailUrl` in the UI with placeholder fallback. ADR 0007.

- [x] **export-encode** (PR #11) — Lanczos-3 `resize` + `fit_size` in `expose-core`; `encode`/`encode_path` in `expose-raw` (JPEG local baseline encoder, PNG, TIFF) with ICC v4 profiles (sRGB, Display P3, opRGB, Rec. 2020, ROMM RGB); decoder reads BT.2020/ROMM curves and small-TIFF ICC. ADR 0006.

- [x] **ui-catalog** (PR #10) — folder tree, row-virtualized grid with page cache, sort/filter toolbar, import button, selection, rating/flag/label shortcuts with optimistic update. Thumbnails plug in at `ui/src/lib/library/thumbnail.ts`.

- [x] **capture-time** (PR #9) — import reads metadata with no catalog lock and stores capture time and display-oriented size. `captureTimeMs` is the camera wall-clock time encoded as if UTC (offset ignored). Metadata failures go to `ImportReport.errors`.

- [x] **raw-decode** (PR #8) — format detection, metadata/EXIF, standard formats, RAW via `rawler` 0.7.2, demosaic (bilinear, MHC). ADR 0005.
- [x] **editor-document** (PR #7) — layer tree, 27 blend modes, masks, 4 adjustment layers, region compositor, undo history. ADR 0005.
- [x] **develop-pipeline-v1** (PR #6) — white balance, exposure, tone zones, contrast, tone curve, auto tone; CPU reference + fused GPU kernel (24 MP pass ~7 ms on M3 Pro). ADR 0005.
- [x] **shell-catalog-commands** (PR #5) — ten Tauri commands for the catalog; TypeScript wrappers in `ui/src/lib/catalog.ts`.
- [x] **ui-shell** (PR #4) — layout, module switching, shortcut registry, theme tokens, toasts.
- [x] **catalog-foundation** (PR #3) — SQLite store, folder tree, ratings/flags/labels, import in place. ADR 0003.
- [x] **core-foundation** (PR #2) — color math, image buffers, develop parameter types. ADR 0003.
- [x] **gpu-foundation** (PR #1) — wgpu context, float images, compute kernels. ADR 0003.
- [x] Licence: 0BSD. `rawler` (LGPL-2.1) approved (ADR 0004).
- [x] Feature list, stack (ADR 0001), working format (ADR 0002), workspace scaffold, `scripts/check.sh`.

## Known issues

- An editor tool can put a `rustfmt` for a different CPU earlier in `PATH`. `scripts/check.sh` sets `RUSTFMT` to the toolchain binary. For manual runs, use `RUSTFMT="$(rustup which rustfmt)" cargo fmt --all`.
- The Tauri app runs in the real webview on macOS (operator, 2026-10-03): thumbnails show, and the transport benchmark runs. The folder picker and the full layout are not checked in detail yet.
