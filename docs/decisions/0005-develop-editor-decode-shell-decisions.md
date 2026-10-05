# 0005 — Develop, Editor, Decode, and Shell Decisions

- **Status:** Accepted (the open item was decided on 2026-10-04, see "Blend space default")
- **Date:** 2026-10-03
- **Decided by:** Claude (implementation details)

## Context

PRs #5–#8 added the catalog commands in the desktop shell, the develop pipeline v1, the editor document model, and image decoding. This record collects their technical decisions.

## Decisions

### Develop pipeline (`expose-develop`, PR #6)

- Fixed stage order: white balance → exposure → highlights/shadows/whites/blacks → contrast → tone curve. Output conversion is not in this crate.
- White balance and exposure fuse into one 3×3 matrix (Bradford adaptation from the scene white to D65).
- Tone zones and contrast are monotonic functions of `e = log2(Y / 0.18)`. They fuse into one 2048-entry LUT. The same ratio multiplies R, G, and B, so hue does not change. Values above 1.0 are kept (highlight recovery).
- The tone curve is a monotone cubic (Fritsch–Carlson), applied to sRGB-encoded values, in a 1024-entry LUT.
- The CPU path is the single-threaded reference (deterministic, WASM-safe). The GPU path is one fused kernel with the same LUTs. Agreement: 9e-7 relative.
- The preview downsample is on the CPU for now (`run_kernel` needs equal input and output sizes).

### Editor (`expose-editor`, PR #7)

- Layer tree: nested lists with stable `LayerId`s. Index 0 is the bottom.
- Each operation returns its inverse edit. History stores the inverse edits. Pixel edits store only the changed rectangle.
- Locks of a group apply to its children. A write that changes alpha on a transparency-locked layer is an error.
- Groups are pass-through by default.
- Blend space is a per-document setting: `Perceptual` (sRGB encoding around the blend; default since 2026-10-04) or `Linear`.
- Modes that are defined only for 0..1 clamp their inputs and then add back the backdrop's part outside 0..1.
- Adjustment layers always work on sRGB-encoded values.

### Decode (`expose-raw`, PR #8)

- Format detection uses magic bytes, not file extensions.
- `rawler` 0.7.2 is the RAW backend (ADR 0004). Its panics are caught and returned as errors. A web build must turn off the `rawler` feature (`rawler` uses memory-mapped files and threads).
- Size limits (default 200 megapixels) are checked before pixel memory is allocated.
- Camera color uses the DNG method with color matrices only. The as-shot white is adapted to D65 with Bradford. Values are not clipped.
- ICC profiles are identified by colorant comparison, not by a full color management system. An unknown color space is flagged as "assumed", never trusted silently.
- Capture time keeps the original string. A time zone offset is set only when the file records one.
- HEIF and AVIF are not supported yet.

### Desktop shell (`apps/desktop/src-tauri`, PR #5)

- The catalog file is `<app_data_dir>/catalog.expose-catalog`.
- State is a concrete `Mutex<SqliteStore>`. A startup failure is kept in the state and reported by `catalog_status`. The app does not panic.
- All commands run on blocking threads. Import scans the file system before it takes the catalog lock.
- Errors go to the UI as `{ kind, message }`.

## Blend space default (operator decision, 2026-10-04)

- **Decided: perceptual.** New documents blend in the perceptual space (`BlendSpace::Perceptual`, `DocumentSpec::new`). Opacity fades and the "light"/"dodge" modes then look as most users expect from other editors.
- Linear stays available per document (physically correct mixing of light).
- Consequence: the compositor fast path for normal/dissolve modes applies only to linear documents. The future GPU compositor must make perceptual blending fast, because it is the common case.

## Consequences

- Proposed changes to `expose-core` (single-channel f32 image, shared monotone cubic, orientation transform, float I/O image, signed rect helpers) and to `expose-gpu` (kernels with different input/output sizes, multiple inputs, buffer reuse) are tracked in `TODO.md`.
