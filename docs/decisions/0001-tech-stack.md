# 0001 — Technology Stack

- **Status:** Accepted
- **Date:** 2026-10-02

## Context

Expose is a desktop photo application with a non-destructive develop module, a photo library, and a layer-based editor (see `FEATURES.md`). Requirements that affect the stack:

- Fast, GPU-accelerated processing of large images at 16 and 32 bits per channel.
- A web version is planned (P3). The image engine must be able to run in a browser.
- macOS is the first platform. Windows and Linux follow later.

## Decision

| Area            | Decision                                    |
| --------------- | ------------------------------------------- |
| Core language   | Rust                                        |
| GPU             | wgpu                                        |
| App shell       | Tauri 2                                     |
| UI              | TypeScript (strict) + Svelte 5, Vite        |
| Platforms       | macOS first, cross-platform-ready code      |
| RAW decode      | `rawler`; LibRaw only as a fallback         |
| Catalog storage | SQLite (`rusqlite`) behind a storage trait  |

## Reasons

- **Rust:** native speed, memory safety, and a first-class WebAssembly target.
- **wgpu:** one GPU API that runs on Metal, Vulkan, DirectX 12, and WebGPU. The same shaders (WGSL) work on desktop and in the browser.
- **Tauri 2:** small native shell; the UI is web technology, so the web version can reuse it.
- **Svelte 5:** low runtime overhead and fine-grained reactivity. This suits panels with many sliders that update at high frequency.
- **rawler:** pure Rust, so it compiles to WASM. LibRaw has wider camera coverage but is C code and LGPL; use it only if coverage gaps are found, and only with operator approval.
- **SQLite:** reliable single-file catalog. A trait boundary lets the web version use a browser storage backend.

## Consequences

- Pixel processing runs in Rust, not in the UI. The UI receives preview frames from the core.
- The frame transport between the Rust core and the UI (shared GPU surface, or encoded frames over IPC) is a performance risk. It must be prototyped early.
- The core must not call native-only APIs directly. Platform services (file access, threads, clock) go through traits, so a WASM build can supply its own implementation.

## Workspace layout

```
crates/
  expose-core/      # pixel buffers, color math, edit parameter types (no I/O, no GPU)
  expose-gpu/       # wgpu context, shader pipelines, tiled render graph
  expose-develop/   # non-destructive develop pipeline (parameters -> render graph)
  expose-raw/       # RAW + standard format decode/encode
  expose-catalog/   # library database, previews cache, sidecar files
  expose-editor/    # layered document model, tools, history
apps/
  desktop/          # Tauri app (src-tauri/ + Svelte UI in ui/)
docs/decisions/     # decision records
```

Rules:

- Dependency direction: `apps` -> feature crates -> `expose-core`. `expose-core` depends on no other workspace crate.
- Only `apps/desktop` may use Tauri. Only `expose-gpu` may use wgpu directly.
