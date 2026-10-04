//! Tauri commands that the UI can call with `invoke`.
//!
//! Commands are in these modules and not at the crate root. A `pub`
//! command at the crate root makes the `#[tauri::command]` macro define its
//! helper macros two times, and the crate does not compile.
//!
//! The commands are thin wrappers. The logic is in [`crate::service`],
//! which tests can call with no Tauri runtime.
//!
//! Tauri changes argument names to camelCase for the UI. For example,
//! `import_folder(path, recursive)` is
//! `invoke("import_folder", { path, recursive })`.

pub mod app;
pub mod catalog;
pub mod dialog;
pub mod export;
pub mod preview;

use tauri::{AppHandle, Manager, Runtime};

use crate::error::CommandError;
use crate::state::CatalogState;

/// Runs `task` with the catalog state on a blocking thread.
///
/// Catalog work locks a mutex and does disk I/O. It must not run on the
/// main thread (the UI freezes) or on an async worker thread (the worker
/// cannot do other work while it waits for the lock).
async fn with_catalog<R, T, F>(app: AppHandle<R>, task: F) -> Result<T, CommandError>
where
    R: Runtime,
    T: Send + 'static,
    F: FnOnce(&CatalogState) -> Result<T, CommandError> + Send + 'static,
{
    run_blocking(move || {
        // `run` manages the state in `setup`, before the UI can call a
        // command. Use `try_state` and not `state`, so that a bug gives an
        // error and not a panic.
        let state = app
            .try_state::<CatalogState>()
            .ok_or_else(|| CommandError::internal("the catalog state is not registered"))?;
        task(state.inner())
    })
    .await
}

/// Runs `task` on a blocking thread and returns its result.
async fn run_blocking<T, F>(task: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, CommandError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| {
            tracing::error!(%error, "a blocking command task failed");
            CommandError::internal(format!("the command task failed: {error}"))
        })?
}
