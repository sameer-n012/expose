# 0015 — Editor: save

- **Status:** Accepted
- **Date:** 2026-10-10
- **Decided by:** Claude (the operator gave the choice to Claude on 2026-10-10)

## Context

The editor (ADR 0013) keeps documents in memory only. Two needs exist: keep the layers for later work, and get the result back into the library as a photo (round-trip). Originals are read-only (ADR 0010).

## Decisions

### Both: a layered file and a flattened photo

"Save" (`Mod+S`) in the editor writes two files next to the original photo:

1. `<stem>-edit.exdoc` — the layered document (native format, below).
2. `<stem>-edit.tif` — the flattened composite, 16-bit TIFF with an ICC profile (the export encoder). The shell imports it into the catalog in the folder of the original. It is a normal photo: the library, develop, and export use it.

If the names exist and are not files from an earlier save of this document, the shell adds a number (`-edit-2`). The catalog records the link (below). "Edit in Editor" on a photo with a link opens the `.exdoc` file, not the photo. A second save of the same document overwrites its two files and refreshes the thumbnail. Develop settings of the TIFF photo apply on top of the flattened pixels, as for any photo.

### Originals stay read-only

The shell overwrites a file only if the catalog records it as an editor output (the `.tif` photo or its `.exdoc`). Any other catalog photo path is refused (ADR 0010 rule).

### Catalog link

A new table records, for an editor output photo: the photo ID of the TIFF, the `.exdoc` path, and the source photo ID (the photo that the document was opened from; may be null if the source is gone).

### Native format `.exdoc`

- The `expose-editor` crate reads and writes it through `std::io::Read`/`Write` (no paths, so a WASM build can supply its own storage).
- Layout: magic and format version, a JSON manifest (document spec, layer tree, layer properties, adjustments, mask metadata, blob table), then binary blobs. Each blob: pixel or mask `f32` little-endian data, byte-shuffled (all byte 0s, then all byte 1s, ...), then deflate (`miniz_oxide`), with a CRC-32. Lossless: a round trip gives the same document.
- A reader rejects an unknown major version, checks all sizes against the editor limits before it allocates, and checks the CRCs. Unknown JSON fields are ignored, so minor versions can add fields.

### Format details (editor-file)

- Header (20 bytes, little-endian): magic `89 'EXDOC' 0D 0A`, major `u16` (1), minor `u16` (0), manifest length `u64` (at most 64 MiB). Byte layout: `crates/expose-editor/src/file/mod.rs`.
- The CRC-32 covers the stored (compressed) bytes. Deflate level 1. The reader rejects a raw length larger than the deflate limit for the compressed length, so a small file cannot force a large allocation.
- Group nesting in a file is limited to 32 levels. The writer refuses deeper documents.
- 6000 × 4000 single layer (release): noise 134.5 MB, write 1.9 s, read 1.3 s; smooth 70.6 MB, write 0.6 s, read 0.6 s.

## Consequences

- The `.tif` duplicates the composite in a second file; this is the cost of a photo that all other modules can use without the editor.
- `.exdoc` files are large (full `f32`), but they are lossless. A later version can add `f16` storage.
- Writing next to the original needs write access to the photo folder. If the folder is read-only, the save fails with a clear error; a "save as" location is later work.
