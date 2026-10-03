# TODO

Work tracker for Expose. `FEATURES.md` is the master feature list. This file tracks the work in progress and partial work.

## In progress

- [ ] **ui-shell** — `apps/desktop/ui`: main window layout, module switch (Library, Develop, Editor), keyboard shortcut registry, theme tokens.
- [ ] **raw-decode** — `expose-raw`: standard formats (JPEG, PNG, TIFF, WebP) and RAW decode with `rawler` into the working format; EXIF read.
- [ ] **develop-pipeline-v1** — `expose-develop`: exposure, white balance, contrast, highlights/shadows/whites/blacks, tone curve, on CPU (reference) and GPU.
- [ ] **shell-catalog-commands** — `apps/desktop/src-tauri`: Tauri commands for the catalog (open, import folder, folder tree, list photos, ratings/flags/labels).

## Next

- [ ] **frame-transport** — prototype how preview frames go from Rust to the UI, and measure the speed (risk from ADR 0001). Needs ui-shell.
- [ ] **thumbnails** — `expose-catalog`: thumbnail and preview cache. Needs raw-decode.
- [ ] **capture-time** — read capture time and size at import (`set_image_info`). Needs raw-decode.
- [ ] **gpu-core-types** — accept `expose-core::WorkingImage` at the `expose-gpu` API boundary.
- [ ] **wasm-check** — `expose-core` must compile for `wasm32-unknown-unknown`. The target is not installed (operator).
- [ ] **catalog-paths** — case-insensitive path comparison on macOS; Windows paths in tests.
- [ ] **gpu-perf** — parameter buffer pool, double-buffered read-back, render graph.

## Done

- [x] **core-foundation** (PR #2) — color math, image buffers, develop parameter types.
- [x] **gpu-foundation** (PR #1) — wgpu context, float images, compute kernels (24 MP exposure round trip ~103–133 ms on M3 Pro).
- [x] **catalog-foundation** (PR #3) — SQLite store, folder tree, ratings/flags/labels, import in place. ADR 0003 records the decisions.
- [x] Feature list (`FEATURES.md`).
- [x] Technology stack (ADR 0001).
- [x] Working color space and pixel format (ADR 0002).
- [x] Workspace scaffold: Cargo workspace with six crates, Tauri 2 shell, Svelte 5 UI, `scripts/check.sh`.

## Known issues

- An editor tool can put a `rustfmt` for a different CPU earlier in `PATH`. `scripts/check.sh` sets `RUSTFMT` to the toolchain binary. For manual runs, use `RUSTFMT="$(rustup which rustfmt)" cargo fmt --all`.
