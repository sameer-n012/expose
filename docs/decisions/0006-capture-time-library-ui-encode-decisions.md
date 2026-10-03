# 0006 — Capture Time, Library UI, and Encode Decisions

- **Status:** Accepted (one item open for the operator, see "Open")
- **Date:** 2026-10-03
- **Decided by:** Claude (implementation details)

## Context

PRs #9–#11 added metadata at import, the library catalog UI, and the encode side of export (resize, encoders, output ICC profiles). This record collects their technical decisions.

## Decisions

### Capture time at import (`apps/desktop/src-tauri`, PR #9)

- `captureTimeMs` stores the camera wall-clock time, encoded as if it were UTC ("floating" local time). The time zone offset of the file is ignored for this field. The UI formats the value with `timeZone: 'UTC'`, so it shows the camera clock.
- Reason: most files have no offset. One rule for all files keeps the sort order stable and the displayed time equal to the camera clock.
- A leap second (second 60) counts as the next minute at second 0. Invalid dates give no capture time.
- Width and height are the display-oriented size (swapped for 90° and 270° orientations). For RAW files, the size is the default crop size from the file header.
- Metadata is read with no catalog lock, on at most 8 worker threads. A failure for one file does not fail the import. It is added to `ImportReport.errors`, and the photo stays in the catalog with no capture time and size.

### Library UI (`apps/desktop/ui`, PR #10)

- The grid is row-virtualized (fixed row height). A sparse page cache loads pages of 200 photos on scroll. Only visible cells render.
- Select-all, range select, and arrow keys act on loaded photos only.
- Rating, flag, and label changes update the UI at once and roll back with a toast on error.
- Library shortcuts use the shortcut registry with scope `library`. They do not act in text inputs.
- Thumbnail URLs come from one function, `ui/src/lib/library/thumbnail.ts`. It returns `null` until the thumbnail task is done.

### Resize (`expose-core`, PR #11)

- Separable Lanczos-3 in linear light on premultiplied alpha. The filter width scales with the downscale ratio. Edge weights are renormalized.
- Alpha is clamped to 0..1. Color overshoot (ringing) is not clamped, because the working format allows values outside 0..1 and the output conversion clips them.
- `fit_size` does not upscale unless the caller allows it.

### Encode (`expose-raw`, PR #11)

- New `EncodeError` type, separate from `DecodeError`.
- Formats: JPEG (baseline, quality 1–100, 4:4:4 / 4:2:2 / 4:2:0), PNG (8/16 bit), TIFF (8/16 bit, none/LZW/deflate).
- JPEG uses a local baseline encoder with the example tables from Annex K of the JPEG standard. Reason: the `image` crate JPEG encoder has no chroma subsampling option, and `jpeg-encoder` has an IJG licence term that is not on the approved list (see "Open").
- TIFF uses the `tiff` crate (MIT) directly, because the `image` crate TIFF encoder cannot compress.
- All outputs embed an ICC v4.3 matrix/TRC profile with parametric curves and a `chad` tag. Profile ID is zero. The profile date is fixed, so output is deterministic. Supported spaces: sRGB, Display P3, opRGB, Rec. 2020, linear Rec. 2020 (16 bit only), ROMM RGB.
- Out-of-gamut colors are clipped per channel. Gamut mapping is later work.
- Optional 8×8 ordered dither for 8-bit output (off by default).
- No metadata (EXIF/XMP) is written yet. Pixels are written in display orientation.
- `encode_path` writes a temporary file in the same folder and then renames it (atomic replace).

## Open (operator decision)

- JPEG encoder: keep the local baseline encoder, or approve `jpeg-encoder` ("(MIT OR Apache-2.0) AND IJG" licence). `jpeg-encoder` adds progressive mode and optimized Huffman tables.

## Consequences

- Thumbnails and export can use `resize` and `encode` now.
- The output color conversion (`to_image`) takes about 580 ms of the 790 ms for a 24 MP JPEG export. It is the next speed target.
- The catalog has no batch write for image information. Large imports write one row at a time.
