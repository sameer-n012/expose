# 0009 — Develop module decisions

- **Status:** Accepted
- **Date:** 2026-10-04
- **Decided by:** Claude (implementation details)

## Context

PR #19 added the first usable Develop module: the Basic panel, the live preview, and develop settings saved in the catalog. This record collects its technical decisions.

## Decisions

### Settings commands (`apps/desktop/src-tauri`)

- `get_develop_settings(photoId)` returns the stored settings after parse and `DevelopSettings::validate()`. A photo with no stored settings gives `DevelopSettings::default()`.
- A stored document that does not parse or validate gives `Internal` with a message. The command does not replace it with the defaults, because a silent fallback hides data loss.
- `set_develop_settings(photoId, settings)` parses and validates the input, and stores it with `DEVELOP_SETTINGS_VERSION`. Bad input gives `InvalidInput`. An unknown photo gives `NotFound`. It returns the stored (clamped) settings.
- The shell stores the re-serialized `DevelopSettings`. Unknown fields are dropped. The UI keeps all fields that it receives and changes only the edited values.

### Develop store (`ui/src/lib/stores/develop.svelte.ts`)

- The active photo is the focused photo of the library, else the first selected photo. The store has an active photo only while the Develop module shows.
- Each edit goes to the preview client at once. The catalog save is debounced (300 ms). The store flushes the save when the photo changes, when the module changes, and on unmount. Save errors show as toasts.
- When the preview cannot open, the sliders still work and a toast shows the reason.
- `AppShell` creates the store.

### Basic panel

- White balance: "As shot" or "Custom". Temperature and tint are disabled in "As shot". A temperature or tint edit sets the mode to "Custom". A preset mode shows as a disabled "Preset" option.
- Double-click on a slider resets it. "Reset all" resets only the Basic panel values (white balance to "As shot", tone to 0). It does not change the tone curve, crop, or sharpening. **Update (PR #37, 2026-10-05):** "Reset all" moved to a footer at the end of the panel column (`Mod+Shift+R`) and resets all sections (white balance, tone, tone curve, sharpening, crop) in one edit. Each panel has its own "Reset".
- The slider ranges (`RANGES`) and identity values (`DEFAULTS`) in `ui/src/lib/develop/settings.ts` are copies of the Rust constants.

### Thumbnail refresh

- The `thumb` scheme sends `ETag` and `Cache-Control: no-cache`, but the library grid stays mounted while it is hidden, so an `<img>` does not reload by itself.
- After a successful save, the UI increments a revision for that photo (`thumbnailRevisions.svelte.ts`) and adds `e=<revision>` to the thumbnail URL. The server ignores this parameter. After an app restart, URLs have no revision, and the ETag check gives the current image.

## Consequences

- The TS ranges can drift from the Rust ranges. A later task can generate them from Rust, or add a test that compares them.
- The module is not checked in the real webview yet.
