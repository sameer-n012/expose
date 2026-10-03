//! Expose desktop application.
//!
//! # Responsibility
//!
//! This crate is the Tauri shell. It opens the main window, loads the UI,
//! and gives the UI access to the feature crates through Tauri commands.
//!
//! # Dependency rules
//!
//! - This is the only crate that may use Tauri.
//! - This crate can depend on all feature crates and on `expose-core`.
//! - Keep pixel processing out of this crate. Put it in the feature crates.

pub mod commands;

/// Starts the application and blocks until the application stops.
///
/// # Errors
///
/// Returns an error if Tauri cannot start the application.
pub fn run() -> tauri::Result<()> {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![commands::app_version])
        .run(tauri::generate_context!())
}
