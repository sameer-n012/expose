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

### Save details (editor-save)

- TIFF: 16-bit, deflate, Rec. 2020 primaries with the BT.2020 curve and its ICC profile (it holds the full working gamut, and color-managed apps read it). The TIFF has an alpha channel (straight) only if a composite pixel is not opaque.
- The flattened image is the GPU composite (`composite_gpu_image`) on the editor render thread. The save clones the document (about 16 B/px for a short time) so that the `.exdoc` write runs on a blocking thread and frames continue.
- Each file is written atomically (temporary file in the same folder, fsync, rename). If the first save fails, both new files are removed.
- The image info of the new photo comes from known values (composite size, capture time of the source photo), so the photo sorts next to its source.
- Dirty: the revision differs from the revision at the last open or save. An undo back to the saved state is still dirty.
- Window close with unsaved changes: a close guard asks Save / Don't save / Cancel. It needs the `core:window:allow-destroy` permission.
- A linked photo with a missing or damaged `.exdoc` gives an error. The editor does not open the flattened TIFF in its place.

### Follow-ups (editor-save-followups, 2026-10-25)

- A first save records the catalog rows in one transaction: `CatalogStore::add_editor_output(&NewEditorOutput)` (find or add the folder, add the photo, image info, link). A duplicate image path gives `InvalidPath`; no rows stay after an error. A re-save calls `update_photo_file` (file size, file time, image info) in one transaction.
- **Save As** (`Mod+Shift+S`, Layers panel button): `editor_save(documentId, folder)`. The folder must be an existing absolute directory. Save As is a first save in that folder: new `<stem>-edit[-N]` names, a new photo and a new link; `source_photo_id` stays. The old files stay. Later plain saves write the new files. The stem comes from the source photo, or from the saved image stem if no source is known.
- A plain save that fails with an `io` error tells the user to try Save As.

## Consequences

- The `.tif` duplicates the composite in a second file; this is the cost of a photo that all other modules can use without the editor.
- `.exdoc` files are large (full `f32`), but they are lossless. A later version can add `f16` storage.
- Writing next to the original needs write access to the photo folder. If the folder is read-only, the save fails with a clear error, and the user can use Save As.
