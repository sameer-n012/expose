//! The error type that all commands return to the UI.
//!
//! The UI receives the error as JSON: `{ "kind": "notFound", "message": "..." }`.

use expose_catalog::{CatalogError, FsErrorKind};
use serde::Serialize;

/// The category of a command error. The UI uses it to select a response
/// (for example, a "not found" message or a retry button).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    /// The photo, folder, or path does not exist.
    NotFound,
    /// An argument from the UI is not valid (for example, a relative path
    /// or a rating of 6).
    InvalidInput,
    /// The catalog database failed, or it contains data that is not valid.
    Database,
    /// A file system operation failed.
    Io,
    /// The catalog did not open at startup. Use `catalog_status` to get the
    /// cause.
    Unavailable,
    /// An internal error (for example, a worker thread stopped). This is
    /// a bug.
    Internal,
}

/// An error that a command returns to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct CommandError {
    /// The category of the error.
    pub kind: ErrorKind,
    /// A description of the error for the user or for the log.
    pub message: String,
}

impl CommandError {
    /// Makes an error.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// Makes an [`ErrorKind::InvalidInput`] error.
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidInput, message)
    }

    /// Makes an [`ErrorKind::Internal`] error.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, message)
    }
}

impl From<CatalogError> for CommandError {
    fn from(error: CatalogError) -> Self {
        let kind = match &error {
            CatalogError::PhotoNotFound(_) | CatalogError::FolderNotFound(_) => ErrorKind::NotFound,
            CatalogError::InvalidPath { .. }
            | CatalogError::InvalidRating(_)
            | CatalogError::ZeroDimension { .. } => ErrorKind::InvalidInput,
            CatalogError::Fs(fs_error) => match fs_error.kind {
                FsErrorKind::NotFound => ErrorKind::NotFound,
                FsErrorKind::NotADirectory | FsErrorKind::InvalidName => ErrorKind::InvalidInput,
                FsErrorKind::PermissionDenied | FsErrorKind::Other => ErrorKind::Io,
            },
            CatalogError::Database(_)
            | CatalogError::NewerSchema { .. }
            | CatalogError::NotACatalog { .. }
            | CatalogError::CorruptValue { .. }
            | CatalogError::Json(_) => ErrorKind::Database,
        };
        Self::new(kind, error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use expose_catalog::{FsError, InvalidRating, PhotoId};

    use super::*;

    #[test]
    fn serializes_as_kind_and_message() {
        let error = CommandError::new(ErrorKind::NotFound, "photo 3 does not exist");
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "kind": "notFound", "message": "photo 3 does not exist" })
        );
        let kinds = [
            (ErrorKind::InvalidInput, "invalidInput"),
            (ErrorKind::Database, "database"),
            (ErrorKind::Io, "io"),
            (ErrorKind::Unavailable, "unavailable"),
            (ErrorKind::Internal, "internal"),
        ];
        for (kind, text) in kinds {
            assert_eq!(serde_json::to_value(kind).unwrap(), text);
        }
    }

    #[test]
    fn maps_catalog_errors_to_kinds() {
        let fs = |kind| CatalogError::Fs(FsError::new("/x", kind, "failed"));
        let cases = [
            (CatalogError::PhotoNotFound(PhotoId(1)), ErrorKind::NotFound),
            (
                CatalogError::InvalidPath {
                    path: PathBuf::from("x"),
                    reason: "the path is not absolute",
                },
                ErrorKind::InvalidInput,
            ),
            (
                CatalogError::InvalidRating(InvalidRating(9)),
                ErrorKind::InvalidInput,
            ),
            (fs(FsErrorKind::NotFound), ErrorKind::NotFound),
            (fs(FsErrorKind::NotADirectory), ErrorKind::InvalidInput),
            (fs(FsErrorKind::PermissionDenied), ErrorKind::Io),
            (fs(FsErrorKind::Other), ErrorKind::Io),
            (
                CatalogError::NewerSchema {
                    found: 9,
                    supported: 1,
                },
                ErrorKind::Database,
            ),
            (
                CatalogError::CorruptValue {
                    field: "flag",
                    value: "maybe".into(),
                },
                ErrorKind::Database,
            ),
        ];
        for (error, kind) in cases {
            let text = error.to_string();
            let mapped = CommandError::from(error);
            assert_eq!(mapped.kind, kind, "{text}");
            assert_eq!(mapped.message, text);
        }
    }
}
