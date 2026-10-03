//! The catalog that the application opens at startup.
//!
//! The application keeps one [`CatalogState`] as Tauri managed state. If
//! the catalog cannot open, the state keeps the error. The application
//! continues to run, and each catalog command returns an
//! [`ErrorKind::Unavailable`] error. The UI calls `catalog_status` to show
//! the cause.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use expose_catalog::SqliteStore;

use crate::error::{CommandError, ErrorKind};

/// The file name of the catalog database in the application data
/// directory.
pub const CATALOG_FILE_NAME: &str = "catalog.expose-catalog";

/// The catalog of the application, or the reason why it did not open.
///
/// The store is a concrete [`SqliteStore`] and not a `dyn CatalogStore`:
/// the status command needs [`SqliteStore::schema_version`], and the
/// desktop shell always uses SQLite. A web build has its own shell.
///
/// The store is `Send` but not `Sync`, so a [`Mutex`] guards it.
#[derive(Debug)]
pub struct CatalogState {
    /// The catalog file, or `None` if the application data directory is
    /// not known.
    path: Option<PathBuf>,
    /// The open store, or the error text from startup.
    store: Result<Mutex<SqliteStore>, String>,
}

impl CatalogState {
    /// Opens (or makes) the catalog file [`CATALOG_FILE_NAME`] in
    /// `data_dir`. Makes `data_dir` if it does not exist.
    ///
    /// This function does not fail. If the catalog cannot open, it logs
    /// the error and returns a state that keeps the error.
    pub fn open_in_dir(data_dir: &Path) -> Self {
        let path = data_dir.join(CATALOG_FILE_NAME);
        if let Err(error) = std::fs::create_dir_all(data_dir) {
            let message = format!(
                "cannot make the data directory {}: {error}",
                data_dir.display()
            );
            return Self::failed(Some(path), message);
        }
        match SqliteStore::open(&path) {
            Ok(store) => {
                tracing::info!(path = %path.display(), "catalog opened");
                Self::from_store(Some(path), store)
            }
            Err(error) => {
                let message = format!("cannot open the catalog {}: {error}", path.display());
                Self::failed(Some(path), message)
            }
        }
    }

    /// Makes a state with an open store.
    pub fn from_store(path: Option<PathBuf>, store: SqliteStore) -> Self {
        Self {
            path,
            store: Ok(Mutex::new(store)),
        }
    }

    /// Makes a state for a catalog that did not open. Logs the error.
    pub fn failed(path: Option<PathBuf>, message: impl Into<String>) -> Self {
        let message = message.into();
        tracing::error!(error = %message, "the catalog is not available");
        Self {
            path,
            store: Err(message),
        }
    }

    /// Returns the catalog file path, or `None` if it is not known.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Returns the startup error, or `None` if the catalog is open.
    pub fn startup_error(&self) -> Option<&str> {
        self.store.as_ref().err().map(String::as_str)
    }

    /// Returns an [`ErrorKind::Unavailable`] error if the catalog did not
    /// open.
    pub fn ensure_available(&self) -> Result<(), CommandError> {
        self.mutex().map(|_| ())
    }

    /// Locks the store.
    ///
    /// Returns [`ErrorKind::Unavailable`] if the catalog did not open, and
    /// [`ErrorKind::Internal`] if an earlier holder of the lock panicked.
    /// A panic can leave the store in an unknown state, so the lock is not
    /// recovered.
    pub fn lock(&self) -> Result<MutexGuard<'_, SqliteStore>, CommandError> {
        self.mutex()?.lock().map_err(|_| {
            CommandError::internal("the catalog lock is poisoned because an operation panicked")
        })
    }

    fn mutex(&self) -> Result<&Mutex<SqliteStore>, CommandError> {
        self.store.as_ref().map_err(|message| {
            CommandError::new(
                ErrorKind::Unavailable,
                format!("the catalog is not available: {message}"),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_and_reopens_catalog_in_new_dir() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");

        let state = CatalogState::open_in_dir(&data_dir);
        assert_eq!(state.startup_error(), None);
        assert_eq!(
            state.path(),
            Some(data_dir.join(CATALOG_FILE_NAME).as_path())
        );
        assert!(state.lock().is_ok());
        drop(state);

        let reopened = CatalogState::open_in_dir(&data_dir);
        assert_eq!(reopened.startup_error(), None);
    }

    #[test]
    fn keeps_startup_error_when_dir_cannot_be_made() {
        let dir = tempfile::tempdir().unwrap();
        // A file is in the place of the data directory.
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, b"x").unwrap();

        let state = CatalogState::open_in_dir(&blocker.join("data"));
        let message = state.startup_error().unwrap();
        assert!(
            message.contains("cannot make the data directory"),
            "{message}"
        );
        assert_eq!(state.lock().unwrap_err().kind, ErrorKind::Unavailable);
        assert_eq!(
            state.ensure_available().unwrap_err().kind,
            ErrorKind::Unavailable
        );
    }

    #[test]
    fn keeps_startup_error_when_file_is_not_a_catalog() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(CATALOG_FILE_NAME), b"not a database").unwrap();

        let state = CatalogState::open_in_dir(dir.path());
        let message = state.startup_error().unwrap();
        assert!(message.contains("cannot open the catalog"), "{message}");
    }
}
