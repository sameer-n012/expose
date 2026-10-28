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
//! - [`preview`]: the develop preview engine (GPU render thread) and the
//!   binary frame format.
//! - [`preview_protocol`]: the `preview` URI scheme, the alternative frame
//!   transport (used by the transport benchmark).
//! - [`editor`]: the editor engine (open layered documents, history, GPU
//!   render thread of the canvas view) and the transport types.
//! - [`export`]: photo export (the job queue, the worker thread, and the
//!   events).
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
pub mod editor;
pub mod error;
pub mod export;
pub mod preview;
pub mod preview_protocol;
pub mod scan;
pub mod service;
pub mod state;
pub mod thumbnail_protocol;
pub mod thumbnails;

use tauri::Manager;

use crate::editor::EditorEngine;
use crate::export::ExportManager;
use crate::preview::PreviewEngine;
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
            // The engine thread starts now; the GPU context is made at the
            // first preview open. Without the engine, the preview commands
            // return an internal error and the scheme answers 503.
            match PreviewEngine::start() {
                Ok(engine) => {
                    app.manage(engine);
                }
                Err(error) => {
                    tracing::error!(%error, "cannot start the preview render thread");
                }
            }
            // The editor thread starts now; its GPU context is made at the
            // first document open. Without the engine, the editor commands
            // return an internal error.
            match EditorEngine::start() {
                Ok(engine) => {
                    app.manage(engine);
                }
                Err(error) => {
                    tracing::error!(%error, "cannot start the editor render thread");
                }
            }
            // Without the export worker, the export commands return an
            // internal error.
            match ExportManager::start() {
                Ok(manager) => {
                    app.manage(manager);
                }
                Err(error) => {
                    tracing::error!(%error, "cannot start the export worker thread");
                }
            }
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol(
            thumbnail_protocol::SCHEME,
            thumbnail_protocol::handle,
        )
        .register_asynchronous_uri_scheme_protocol(
            preview_protocol::SCHEME,
            preview_protocol::handle,
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
            commands::catalog::get_develop_settings,
            commands::catalog::set_develop_settings,
            commands::dialog::pick_folder,
            commands::editor::editor_open,
            commands::editor::editor_new,
            commands::editor::editor_render,
            commands::editor::editor_edit,
            commands::editor::editor_stroke_begin,
            commands::editor::editor_stroke_points,
            commands::editor::editor_stroke_end,
            commands::editor::editor_stroke_cancel,
            commands::editor::editor_selection_outline,
            commands::editor::editor_undo,
            commands::editor::editor_redo,
            commands::editor::editor_close,
            commands::editor::editor_save,
            commands::export::export_start,
            commands::export::export_cancel,
            commands::preview::preview_open,
            commands::preview::preview_open_test_pattern,
            commands::preview::preview_render,
            commands::preview::preview_close,
            commands::preview::preview_bench_channel,
        ])
        .run(tauri::generate_context!())
}
