//! Native dialog commands.

use tauri::{AppHandle, Runtime};
use tauri_plugin_dialog::DialogExt;

use super::run_blocking;
use crate::error::CommandError;

/// Shows the native folder picker and returns the absolute path of the
/// selected folder, or `None` if the user cancels.
///
/// The dialog blocks its thread, so it runs on a blocking thread and not
/// on the main thread.
#[tauri::command]
pub async fn pick_folder<R: Runtime>(app: AppHandle<R>) -> Result<Option<String>, CommandError> {
    run_blocking(move || {
        let Some(selected) = app.dialog().file().blocking_pick_folder() else {
            return Ok(None);
        };
        let path = selected.into_path().map_err(|error| {
            CommandError::invalid_input(format!("the selected item is not a folder path: {error}"))
        })?;
        path.into_os_string()
            .into_string()
            .map(Some)
            .map_err(|raw| {
                CommandError::invalid_input(format!(
                    "the folder path {} is not valid UTF-8",
                    raw.to_string_lossy()
                ))
            })
    })
    .await
}
