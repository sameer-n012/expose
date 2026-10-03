# TODO

Work tracker for Expose. `FEATURES.md` is the master feature list. This file tracks the work in progress and partial work.

No `FEATURES.md` item is marked `[x]` yet: the libraries exist, but no feature is available in the UI with tests from end to end.

## In progress

- (none — all agent branches are merged)

## Next (suggested order)

Model hint: **S** = easier task (use the sonnet model), **O** = hard task (use the default model).

1. [ ] **ui-catalog** (S) — `apps/desktop/ui`: folder tree (left panel), Library grid with paging (`listPhotos` with `limit`/`offset`), import button (`pickFolder` + `importFolder`), rating/flag/label shortcuts (0–5, P/X/U, 6–9) on the selection. Typed command wrappers already exist in `src/lib/catalog.ts`. Errors go through `callBackend` toasts.
2. [ ] **capture-time** (S) — at import, call `expose_raw::read_metadata` and store capture time and size with `CatalogStore::set_image_info`. Decide the time zone rule (capture time without offset: store as local wall time; document it). Touches `expose-catalog` (import hook) or `src-tauri` (after import).
3. [ ] **thumbnails** (O) — preview cache: decode (`expose-raw`), develop at small size (`expose-develop::render_preview`), encode JPEG, store in a cache folder keyed by photo ID + settings hash; Tauri command or custom protocol to serve them to the grid. Use an embedded JPEG preview from RAW files when present (fast path).
4. [ ] **frame-transport** (O) — prototype how develop preview frames go from Rust to the UI (custom URI protocol with raw/encoded bytes vs. IPC binary vs. shared surface). Measure frames per second at 2560 px. Record the result as an ADR. Paths: `src-tauri` + `ui`.
5. [ ] **develop-ui** (S after 4) — Develop module panel with sliders bound to `DevelopSettings`, live preview through the frame transport, settings saved to the catalog (`develop_settings` JSON, version from `DEVELOP_SETTINGS_VERSION`).
6. [ ] **export-v1** (O) — `expose-raw` encode JPEG/PNG/TIFF + output color space (core `to_image`) + size options (FEATURES 5.1 P1).
7. [ ] **editor-gpu / editor-ui** — later: GPU compositor that matches the CPU reference, then the editor canvas UI.

### Follow-ups from merged work

- [ ] **core-additions** (S) — move shared code into `expose-core`: single-channel f32 image (editor masks), monotone cubic spline (develop + editor have copies), orientation transform and `Image<f32>` float I/O (raw has local copies), signed `Rect` helpers, public `Chromaticity::to_uv`, smooth tangent in `temperature_tint_to_chromaticity` near 4000 K.
- [ ] **gpu-api** (O) — kernels with different input/output size (GPU downsample), multiple inputs / storage buffers, parameter buffer reuse, double-buffered read-back, `WorkingImage` at the API boundary, render graph.
- [ ] **raw-real-samples** (operator + S) — run `EXPOSE_RAW_SAMPLES=/path cargo test -p expose-raw -- --ignored real_samples --nocapture` on real camera files and fix what fails. Then: forward matrices, BaselineExposure, DNG black level origin check, multi-threaded/tiled demosaic.
- [ ] **develop-tuning** — tune slider strengths and zone centers on real photos; local (edge-aware) tone zones; RAW white balance as camera-space gains.
- [ ] **editor-storage** — sparse tiled layer storage; history memory budget; clipping masks; layer effects.
- [ ] **ui-polish** (S) — save panel layout between sessions; Tab shortcut conflicts with keyboard focus navigation; refresh shortcut tooltips after a remap.
- [ ] **catalog-paths** (S) — case-insensitive path comparison on macOS; Windows paths in tests.
- [ ] **wasm-check** — `expose-core` must compile for `wasm32-unknown-unknown`. The target is not installed (operator). `expose-raw` must build for WASM with `--no-default-features` (no `rawler`).
- [ ] **lgpl-relink** — before the first public release, make the distributed build let users relink with a modified `rawler` (ADR 0004).

### Open operator decisions

- Editor default blend space: linear (current) or perceptual (ADR 0005, "Open").

## Done

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
- The Tauri app has not been started yet (`npm --prefix apps/desktop/ui run tauri dev`). Startup, the folder picker, and the layout in the real webview are not checked.
