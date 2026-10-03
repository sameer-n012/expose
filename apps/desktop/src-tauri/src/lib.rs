//! Expose desktop application.
//!
//! # Responsibility
//!
//! This crate is the Tauri shell. It opens the main window, loads the UI,
//! and gives the UI access to the feature crates through Tauri commands.
//!
//! # Structure
//!
//! - [`state`]: the catalog that opens at startup (managed state).
//! - [`service`]: the logic of the catalog commands, with no Tauri
//!   dependency, so that tests can call it.
//! - [`scan`]: the file system scan before an import.
//! - [`capture`]: capture time and image size, read from file metadata
//!   after an import.
//! - [`thumbnails`]: the thumbnail cache and its worker pool.
//! - [`thumbnail_protocol`]: the `thumb` URI scheme that serves the
//!   thumbnails to the UI.
//! - [`commands`]: the `#[tauri::command]` wrappers.
//! - [`error`]: the error type that the UI receives.
//!
//! # Dependency rules
//!
//! - This is the only crate that may use Tauri.
//! - This crate can depend on all feature crates and on `expose-core`.
//! - Keep pixel processing out of this crate. Put it in the feature crates.

pub mod capture;
pub mod commands;
pub mod error;
pub mod scan;
pub mod service;
pub mod state;
pub mod thumbnail_protocol;
pub mod thumbnails;

use tauri::Manager;

use crate::state::CatalogState;
use crate::thumbnails::{THUMB_DIR_NAME, ThumbnailService, default_workers};

/// Starts the application and blocks until the application stops.
///
/// At startup, the application opens the catalog
/// [`state::CATALOG_FILE_NAME`] in the application data directory. If the
/// catalog cannot open, the application starts all the same, and the UI
/// gets the error from the `catalog_status` command.
///
/// # Errors
///
/// Returns an error if Tauri cannot start the application.
pub fn run() -> tauri::Result<()> {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let state = match app.path().app_data_dir() {
                Ok(dir) => CatalogState::open_in_dir(&dir),
                Err(error) => CatalogState::failed(
                    None,
                    format!("cannot find the application data directory: {error}"),
                ),
            };
            app.manage(state);
            // Without a cache folder the library shows placeholders; the
            // `thumb` scheme then answers 503.
            match app.path().app_cache_dir() {
                Ok(dir) => {
                    match ThumbnailService::new(dir.join(THUMB_DIR_NAME), default_workers()) {
                        Ok(service) => {
                            tracing::info!(dir = %service.dir().display(), "thumbnail cache opened");
                            app.manage(service);
                        }
                        Err(error) => {
                            tracing::error!(%error, "the thumbnail cache is not available");
                        }
                    }
                }
                Err(error) => tracing::error!(
                    %error,
                    "cannot find the application cache directory; no thumbnails"
                ),
            }
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol(
            thumbnail_protocol::SCHEME,
            thumbnail_protocol::handle,
        )
        .invoke_handler(tauri::generate_handler![
            commands::app::app_version,
            commands::catalog::catalog_status,
            commands::catalog::import_folder,
            commands::catalog::folder_tree,
            commands::catalog::list_photos,
            commands::catalog::get_photo,
            commands::catalog::set_rating,
            commands::catalog::set_flag,
            commands::catalog::set_color_label,
            commands::dialog::pick_folder,
        ])
        .run(tauri::generate_context!())
}
