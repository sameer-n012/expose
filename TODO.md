# TODO

Work tracker for Expose. `FEATURES.md` is the master feature list. This file tracks the work in progress and partial work.

## In progress

- [ ] **core-foundation** — `expose-core`: pixel buffers, color spaces and conversions, develop parameter types (ADR 0002).
- [ ] **gpu-foundation** — `expose-gpu`: headless wgpu context, compute pipeline helper, texture upload/download, first WGSL kernel with tests.
- [ ] **catalog-foundation** — `expose-catalog`: SQLite schema and migrations, storage trait, folders, photos, ratings, flags, color labels, import in place (folder scan).
- [ ] **ui-shell** — `apps/desktop/ui`: main window layout, module switch (Library, Develop, Editor), keyboard shortcut registry, theme tokens.

## Next (after the in-progress items are merged)

- [ ] **raw-decode** — `expose-raw`: standard formats (JPEG, PNG, TIFF, WebP) and RAW decode with `rawler` into the working format; EXIF read. Needs core-foundation.
- [ ] **frame-transport** — prototype how preview frames go from Rust to the UI, and measure the speed (risk from ADR 0001). Needs gpu-foundation and ui-shell.
- [ ] **develop-pipeline-v1** — `expose-develop`: exposure, white balance, contrast, highlights/shadows/whites/blacks, tone curve, on CPU (reference) and GPU. Needs core-foundation and gpu-foundation.
- [ ] **thumbnails** — `expose-catalog`: thumbnail and preview cache. Needs raw-decode.

## Done

- [x] Feature list (`FEATURES.md`).
- [x] Technology stack (ADR 0001).
- [x] Working color space and pixel format (ADR 0002).
- [x] Workspace scaffold: Cargo workspace with six crates, Tauri 2 shell, Svelte 5 UI, `scripts/check.sh`.

## Known issues

- An editor tool can put a `rustfmt` for a different CPU earlier in `PATH`. `scripts/check.sh` sets `RUSTFMT` to the toolchain binary. For manual runs, use `RUSTFMT="$(rustup which rustfmt)" cargo fmt --all`.
