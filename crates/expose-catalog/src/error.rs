//! Error types for the catalog.

use std::path::PathBuf;

use crate::model::{FolderId, InvalidRating, PhotoId};
use crate::platform::FsError;

/// The result type of all catalog operations.
pub type Result<T, E = CatalogError> = std::result::Result<T, E>;

/// An error from a catalog operation.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    /// The database engine returned an error.
    #[cfg(feature = "sqlite")]
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// The catalog file was made by a newer version of Expose.
    /// This version cannot open it safely.
    #[error("catalog schema version {found} is newer than the supported version {supported}")]
    NewerSchema {
        /// The schema version in the catalog file.
        found: u32,
        /// The newest schema version that this code knows.
        supported: u32,
    },

    /// The file is a database, but it is not an Expose catalog.
    #[error("the database is not an Expose catalog (application id {application_id:#x})")]
    NotACatalog {
        /// The application id in the database header.
        application_id: i32,
    },

    /// A path is not valid as a catalog folder path.
    #[error("invalid path {path:?}: {reason}")]
    InvalidPath {
        /// The path that is not valid.
        path: PathBuf,
        /// Why the path is not valid.
        reason: &'static str,
    },

    /// No photo has this id.
    #[error("photo {0} does not exist")]
    PhotoNotFound(PhotoId),

    /// No folder has this id.
    #[error("folder {0} does not exist")]
    FolderNotFound(FolderId),

    /// A rating is out of range.
    #[error(transparent)]
    InvalidRating(#[from] InvalidRating),

    /// A file system operation failed.
    #[error(transparent)]
    Fs(#[from] FsError),

    /// A JSON document could not be encoded or decoded.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// An image dimension is zero. A dimension must be at least 1, or
    /// unknown.
    #[error("image {field} must be at least 1")]
    ZeroDimension {
        /// `"width"` or `"height"`.
        field: &'static str,
    },

    /// The database contains a value that this code does not know.
    /// This can occur if the file was changed by a different program.
    #[error("unexpected stored value {value:?} in {field}")]
    CorruptValue {
        /// The name of the column.
        field: &'static str,
        /// The value, as text.
        value: String,
    },
}
