# 0010 — Export decisions

- **Status:** Accepted
- **Date:** 2026-10-04
- **Decided by:** Claude (implementation details)

## Context

PRs #20 (shell) and #21 (UI) added photo export: JPEG, PNG, and TIFF, size options, quality and bit depth, and the output color space (`FEATURES.md` 5.1, P1). Two agents built the two parts at the same time against one command contract. This record collects the decisions.

## Decisions

### Command contract

- `export_start(request, onEvent: Channel<ExportEvent>) -> jobId` returns at once. A worker thread does the work.
- `export_cancel(jobId)`. An unknown or finished job gives `notFound`. A cancel takes effect between photos.
- Events: `started`, `photoDone` (`index` counts from 0 after duplicate ids are removed), `photoSkipped`, `photoFailed`, `finished`. `finished` is always the last event, also after a cancel or an internal error.
- The shell parses the request JSON itself, so a value of the wrong type also gives `invalidInput`. A bad request sends no events.
- Unknown photo ids, bad stored develop settings, and a panic in a decoder give `photoFailed`. The job continues. Bad stored settings do not fall back to the defaults.

### Shell (`apps/desktop/src-tauri/src/export/`)

- One worker thread, one photo at a time. A 24 MP image in the working format is about 384 MB plus the resize buffer.
- A second job waits in a queue. It sends `started` when it begins. The UI does not start a second job while one runs.
- Steps per photo: decode → develop on the CPU → Lanczos-3 resize → atomic encode (`encode_path`). The CPU path keeps the GPU free for the preview render thread. A 24 MP photo takes about 0.73 s (release build, JPEG q90).
- Outputs above 200 megapixels give `photoFailed`. The request limits could otherwise ask for more than 16 GB.
- If the window closes, the job continues and writes the files. Events that cannot be sent are logged.

### File names

- Name: original file stem (+ optional suffix) + extension of the format.
- Within one job, names are compared without case (the default macOS and Windows file systems do not tell `A.jpg` and `a.jpg` apart). A later photo with the same name gets `-2`, `-3`, and so on, under all conflict policies.
- **Originals are read-only.** An export never writes to the file of any catalog photo. At the start of each job, the shell reads the file names of catalog photos in the destination folder (canonical paths, compared without case) and reserves them. A reserved name moves to the next free `-N` name under all three policies. If the list cannot be read, each photo gets `photoFailed`.
- The output name is chosen before the decode, so a skipped photo costs no decode.

### UI (`ui/src/lib/export/`, `ui/src/components/export/`)

- Defaults: JPEG quality 85 with 4:2:0, sRGB, original size, original name, unique name on conflict, dither on. When the user selects a size mode, the value starts at long edge 2048 px, short edge 1080 px, 8 MP, or 50 %.
- The form is flat, so values stay when the format changes. The last options are kept in `localStorage` (`expose.export.options.v1`); each field is checked, and a bad field gets its default.
- `Mod+Shift+E` and the toolbar button export the selected photos (Library) or the active photo (Develop).
- The dialog stops key events, so app shortcuts do not run behind it.

## Consequences

- Export does not apply crop or sharpening yet, because the develop pipeline has only per-pixel stages. See `TODO.md` → export-followups.
- Export does not copy metadata (EXIF/XMP/IPTC) yet. See encode-followups.
