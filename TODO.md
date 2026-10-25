# TODO

Work tracker for Expose. `FEATURES.md` is the master feature list. This file tracks the work in progress and partial work.

`FEATURES.md` items marked `[x]` (2026-10-04, after the operator check in the real webview): 2.2 exposure, contrast, highlights/shadows/whites/blacks; 2.2 point tone curve; sharpening; crop with presets and overlays; straighten/rotate/flip; histogram with clipping warnings (P2); reset (all settings or one panel); 5.1 the four P1 export items. White balance stays open (presets in the UI and the eyedropper are missing); automatic tone stays open (no UI yet). Loupe view stays open (no 1:1 zoom).

Operator checks in the real webview (2026-10-04/05): loupe view, Grid/Loupe keys, Tone curve panel, Detail panel, histogram, crop tool, Reset all, sharp cropped previews, and crop polish work. 2026-10-10: the Editor first version (open, zoom/pan, layers panel, adjustment layers, undo/redo) works.

`FEATURES.md` editor items marked `[x]` (2026-10-10): 3.2 opacity and fill opacity, blend modes; 3.3 brightness/contrast, hue/saturation. 2026-10-25 (operator check of save): 3.1 open and save native layered document format. Still open in the editor P1 list: new document, rotate view, multiple documents (tabs), add pixel layer, locks UI, layer masks UI, levels and curves editors, selections, brushes.

## In progress

- (none — all agent branches are merged; no worktrees)

## Next (suggested order)

Model hint: **S** = easier task (use the sonnet model), **O** = hard task (use the default model).

1. [x] **editor-save-check** (operator, 2026-10-25: "save seems to work"; Save As not reported separately) — check in the real webview: `Mod+S` writes `<name>-edit.exdoc` + `<name>-edit.tif` next to the original, the TIFF appears in the library with a "Layers" badge, "Edit in Editor" on it reopens the layers, a re-save overwrites and refreshes the thumbnail, and the unsaved-changes question on photo switch and on window close; also Save As… (`Mod+Shift+S`) to another folder (PR #52). After the check: mark 3.1 "Open and save native layered document format" `[x]`.
2. [x] **editor-save-followups** (PR #52) — `add_editor_output` (one transaction), `update_photo_file` on re-save, Save As… to another folder (ADR 0015, "Follow-ups").
3. [ ] **editor-layers-ui** (S) — remaining P1 layer items in the UI: add an empty pixel layer, lock toggles (the model has `set_locks`), new document dialog (`DocumentSpec`), levels and curves editors for adjustment layers (the model and compositor support them). Check whether the `EditDto` contract needs new variants (shell + UI together).
4. [ ] **editor-tools** (O) — split before work: (a) layer masks in the UI (add/delete/enable, show mask); (b) brush engine (`write_pixels`/`write_mask` exist; needs a stroke model, dabs, GPU or CPU path, and history per stroke); (c) selections (rect/ellipse/lasso, selection as a mask). Design draft: ADR 0016 (Proposed) — order masks UI → selections → brushes; points (not pixels) over the IPC; selection outline as polygons; partial uploads from per-layer changed rectangles.
5. [ ] **editor-tabs** — multiple open documents (P1); the engine already holds several documents.

### Follow-ups from editor-gpu (ADR 0012)

- [x] **editor-gpu-followups** (PR #53) — below snapshot in `render_view` (repeated slider step 62 → ~29 ms at 4096², +256 MiB), curve table cache, curve end-point test, doc links fixed (ADR 0014).
- [ ] **editor-gpu-followups-2** — the first step on a new layer is a full composite (70–79 ms); composite and rebuild the pyramid only in the changed area; GPU memory about 2.6 GiB for a 4096² 10-layer document (sparse tiles; `rgba16float` fails the 5e-5 CPU tolerance at 2.6e-4 — operator decision if a looser tolerance is acceptable).

### Follow-ups from merged work

- [ ] **develop-ui-followups** (S) — the TS slider ranges in `ui/src/lib/develop/settings.ts` are copies of the Rust constants (add a test or generate them); the shell drops unknown settings fields (ADR 0009).

- [ ] **geometry-followups** — crops tighter than about 60 % of the long edge hit the 4096 px session base and show slightly enlarged; `PreviewRenderer` still allocates its unused tone-only output image (change in `expose-develop`); `DevelopTargets` intermediates are full `rgba32float` (single-channel formats in `expose-gpu` would save memory); auto straighten (P2).
- [ ] **develop-ui-followups-2** (S) — histogram is computed from the 8-bit display frame (not the working data).
- [ ] **export-followups** — copy metadata (see encode-followups); output sharpening, naming templates, presets (P2); the UI `Mod+Shift+E` does nothing while the dialog is open (by design, the dialog stops key events).
- [ ] **bench-recheck** (operator, optional) — the numbers are from before the geometry/sharpening stages (PR #29) and the 4096 px session source (PR #38); measure again. On a quiet machine: `?bench=transport&only=slider-ipc` and `&only=ipc-rgba8-bitmap` against the 2026-10-03 rows. PR #22 found no leak; the slow rows were most likely background load.
- [ ] **preview-followups** — the slider simulation is limited by main-thread time (timer ticks), not the transport; one waiting slot per session in the engine if a prefetch supersedes a visible request; update the target size when the display pixel density changes; move the full-size downsample at session open off the render thread (it stops other sessions for about 0.1 s); check `invoke` binary responses on Linux/Windows (message IPC fallback gives a number array); native GPU surface if the measurement is below 30 fps (P3).
- [ ] **core-followups** (S) — `Rect::translated` is not used yet (editor `add_offset` works on tuples); the Planckian locus also has a piece boundary at 2222 K (y fit) with a small kink.
- [ ] **gpu-api** (O) — GPU downsample in the preview (multi-input and any-size kernels exist since PR #26), storage buffers, single-channel formats, parameter buffer reuse, double-buffered read-back, `WorkingImage` at the API boundary, render graph.
- [ ] **raw-real-samples** (operator + S) — run `EXPOSE_RAW_SAMPLES=/path cargo test -p expose-raw -- --ignored real_samples --nocapture` on real camera files and fix what fails. Then: forward matrices, BaselineExposure, DNG black level origin check, multi-threaded/tiled demosaic.
- [ ] **develop-tuning** — tune slider strengths and zone centers on real photos; local (edge-aware) tone zones; RAW white balance as camera-space gains.
- [ ] **editor-storage** — sparse tiled layer storage; history memory budget; clipping masks; layer effects.
- [ ] **ui-polish** (S) — save panel layout between sessions; Tab shortcut conflicts with keyboard focus navigation; refresh shortcut tooltips after a remap.
- [ ] **ui-catalog-followups** (S) — search box is a placeholder; per-request version for optimistic rollback (two quick changes can restore an old value); folder count is direct-only, so it differs from the grid total with "include subfolders".
- [ ] **thumbnail-followups** — RAW with non-default white balance: give the pipeline the as-shot `SourceInfo` (from `DecodeInfo.raw.multipliers`), not `SourceInfo::RENDERED`; reduce memory (scaled JPEG decode, or decode to 8-bit, instead of a full-size f32 image); cache size limit and removal of thumbnails of deleted photos; check the `thumb` scheme, CSP, and 304 handling in a real webview.
- [ ] **encode-followups** — copy EXIF/XMP/IPTC into exported files; gamut mapping instead of clipping; JPEG optimized Huffman tables, progressive mode, gray output; TIFF ICC tag type UNDEFINED instead of BYTE.
- [ ] **catalog-paths** (S) — case-insensitive path comparison on macOS; Windows paths in tests.
- [ ] **wasm-check** — `expose-core` must compile for `wasm32-unknown-unknown`. The target is not installed (operator). `expose-raw` must build for WASM with `--no-default-features` (no `rawler`).
- [ ] **lgpl-relink** — before the first public release, make the distributed build let users relink with a modified `rawler` (ADR 0004).

### Open operator decisions

- JPEG encoder: keep the local baseline encoder, or approve `jpeg-encoder` (licence "(MIT OR Apache-2.0) AND IJG") (ADR 0006, "Open").

## Done

- [x] **editor first version** (PRs #43–#46, ADRs 0012–0014; operator check 2026-10-10) — `GpuCompositor` (matches the CPU `composite`); revision + layer content versions, layer texture cache, `render_view` (pan 1–2 ms at 4096²), `History::push_merge`; Editor UI (canvas zoom/pan, layers panel, adjustment layers, undo/redo, `Mod+E`); `EditorEngine` + `editor_open/render/edit/undo/redo/close` (own render thread and `GpuContext`, history 50 steps).
- [x] **editor save** (PRs #47–#50, ADR 0015; operator check 2026-10-25) — catalog `editor_outputs` (schema 2); `.exdoc` format (`expose_editor::file`), `Document::from_layer`/`layer_by_number`; `editor_save` (`.exdoc` + 16-bit Rec. 2020 TIFF next to the original, atomic writes, import + link, overwrite only own outputs), linked photos reopen their `.exdoc`, `Mod+S`, unsaved-changes question, "Layers" grid badge.

- [x] **editor-gpu** (PR "GPU compositor") — `GpuCompositor` in `expose-editor` matches the CPU `composite` for all 27 blend modes in both blend spaces, the 4 adjustment kinds, masks, nested groups, and dissolve (ADR 0012). 4096², 10 layers, perceptual: CPU 15.3 s, GPU 271 ms (warm).

- [x] **develop follow-ups** (PRs #37, #38, #39) — "Reset all" (footer, `Mod+Shift+R`) resets every panel in one edit; `longEdge` = frame long edge, the engine enlarges the develop input by the crop factor (session base up to 4096 px, about 180 MB CPU per session), thumbnails of cropped photos develop on up to 4096 px (format version 3), `preview://` `geometry=0|1`; crop mode zooms out at large angles, crop frame keys (arrows move, Alt+arrows resize), fine grid on keyboard angle changes.
- [x] **crop-tool** (PR #34) — Crop panel and crop mode (`R`; `Enter` keeps, `Esc` cancels; `O` overlays; `X` swaps orientation). Preview with `applyGeometry: false` and one CSS matrix for flip + rotation (tested against the backend mapping); 8 handles, move, aspect presets and lock; `maxInscribedRect`/`rectIsInside` ported to `lib/develop/cropGeometry.ts` with the Rust test cases.- [x] **detail-histogram** (PR #33) — Detail panel (amount, radius, detail, masking); RGB histogram of the displayed frame (UI-side, throttled 100 ms, subsampled, sqrt scale, clip indicators). Develop panel order: Histogram, Crop, Basic, Tone curve, Detail.
- [x] **geometry-integration** (PR #31) — preview, thumbnails, and export run the full develop (crop, straighten, flip, sharpening). `preview_render` optional `applyGeometry` (default true); frame header size = real output size; display encoder rebuilt only on a size change. Thumbnail format version 2. Export develops at source resolution before the resize. ADR 0011.

- [x] **develop-geometry-gpu** (PR #29) — geometry (first) and sharpening (last) in `Pipeline`: `develop_cpu`/`develop_cpu_owned`, `develop_gpu` + `DevelopTargets`, `PreviewRenderer::develop_full`/`set_original_size`, `set_apply_geometry`; 5 new GPU kernels (GPU vs CPU ≤ 2.3e-6 relative). `is_identity` now covers all stages; `tone_is_identity` is the old meaning. ADR 0011.
- [x] **tone-curve-panel** (PR #28) — Tone curve panel in Develop (Master/R/G/B, add/drag/remove points, keyboard, per-channel reset); `lib/develop/curve.ts`. "Reset all" still resets only the Basic values.
- [x] **loupe-view** (PR #27) — the library loupe shows the photo with its saved develop settings (`LoupeStore`, `LoupeImage`): one preview session per loupe photo, late results dropped, session closed on step/leave/module change; thumbnail until the first frame; caption with name and date.
- [x] **develop-geometry / develop-sharpen / gpu-multi-input** (PRs #24, #25, #26) — CPU references for flip/straighten/crop (`Geometry`, `max_inscribed_rect`, `rect_is_inside`) and for luminance sharpening (`Sharpener`, four passes); `GpuContext::run_kernel_multi` (1..=4 inputs, any output size). ADR 0011.

- [x] **library-navigation** (PR #23) — Grid/Loupe toggle in the library toolbar; `L` opens the loupe on the selected photo (`E` is the Editor module); `G` and `Esc` return to the grid; a click on the Library tab in the loupe returns to the grid; ArrowLeft/ArrowRight step in the loupe (no wrap). The selection follows the loupe photo, so rating/flag/label keys act on it.- [x] **bench-regression** (PR #22) — no leak in the preview path; release tests added (bitmaps closed, late responses dropped before parse). Benchmark options `only=`, `reverse=1`, `settle=<ms>`. The 12 ms shell time at `@1280` was one GPU view rebuild in the mean; slider candidates now render a warm-up frame.
- [x] **export-v1** (PRs #20, #21) — `export_start`/`export_cancel` with an event channel; one worker, one photo at a time, CPU develop (about 0.73 s per 24 MP photo); conflict policies; never writes to a catalog original. Export dialog, `Mod+Shift+E`, progress with cancel. ADR 0010.

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
- The Tauri app runs in the real webview on macOS (operator, 2026-10-03 to 2026-10-05): library, loupe, Develop panels, crop tool, and export are checked. The preview frame rate is not measured again since the geometry stages (PR #29); see bench-recheck.
