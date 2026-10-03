# TODO

Work tracker for Expose. `FEATURES.md` is the master feature list. This file tracks the work in progress and partial work.

## In progress

- [ ] **raw-decode** — `expose-raw`: standard formats (JPEG, PNG, TIFF, WebP) and RAW decode with `rawler` into the working format; EXIF read.
- [ ] **develop-pipeline-v1** — `expose-develop`: exposure, white balance, contrast, highlights/shadows/whites/blacks, tone curve, on CPU (reference) and GPU.
- [ ] **editor-document** — `expose-editor`: layer stack, blend modes, masks, adjustment layers, compositor, history.

## Next

- [ ] **frame-transport** — prototype how preview frames go from Rust to the UI, and measure the speed (risk from ADR 0001). Starts after shell-catalog-commands merges (same paths).
- [ ] **ui-catalog** — Library grid and folder tree in the UI, with the catalog commands. Needs shell-catalog-commands.
- [ ] **ui-polish** — save panel layout between sessions; Tab shortcut conflicts with keyboard focus navigation; refresh shortcut tooltips after a remap.
- [ ] **thumbnails** — `expose-catalog`: thumbnail and preview cache. Needs raw-decode.
- [ ] **capture-time** — read capture time and size at import (`set_image_info`). Needs raw-decode.
- [ ] **gpu-core-types** — accept `expose-core::WorkingImage` at the `expose-gpu` API boundary.
- [ ] **wasm-check** — `expose-core` must compile for `wasm32-unknown-unknown`. The target is not installed (operator).
- [ ] **lgpl-relink** — before the first public release, make the distributed build let users relink with a modified `rawler` (ADR 0004).
- [ ] **catalog-paths** — case-insensitive path comparison on macOS; Windows paths in tests.
- [ ] **gpu-perf** — parameter buffer pool, double-buffered read-back, render graph.

## Done

- [x] **shell-catalog-commands** (PR #5) — ten Tauri commands for the catalog; TypeScript types for the UI are in the PR #5 agent report (to add to `backend.ts` in ui-catalog).
- [x] **ui-shell** (PR #4) — layout, module switching, shortcut registry, theme tokens, toasts.
- [x] **core-foundation** (PR #2) — color math, image buffers, develop parameter types.
- [x] **gpu-foundation** (PR #1) — wgpu context, float images, compute kernels (24 MP exposure round trip ~103–133 ms on M3 Pro).
- [x] **catalog-foundation** (PR #3) — SQLite store, folder tree, ratings/flags/labels, import in place. ADR 0003 records the decisions.
- [x] Feature list (`FEATURES.md`).
- [x] Technology stack (ADR 0001).
- [x] Working color space and pixel format (ADR 0002).
- [x] Workspace scaffold: Cargo workspace with six crates, Tauri 2 shell, Svelte 5 UI, `scripts/check.sh`.

## Known issues

- An editor tool can put a `rustfmt` for a different CPU earlier in `PATH`. `scripts/check.sh` sets `RUSTFMT` to the toolchain binary. For manual runs, use `RUSTFMT="$(rustup which rustfmt)" cargo fmt --all`.
