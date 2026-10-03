//! Platform services: file system access and the clock.
//!
//! The catalog does not call native APIs directly. It uses the
//! [`FileSystem`] and [`Clock`] traits. The desktop build uses
//! [`StdFileSystem`] and [`SystemClock`] (feature `native`). A web build
//! can supply its own implementations. Tests use [`MemoryFileSystem`] and
//! [`FixedClock`].
//!
//! The catalog only reads from the file system. It never writes to
//! original photo files.

mod memory;
#[cfg(feature = "native")]
mod native;

use std::path::{Path, PathBuf};

pub use memory::{FixedClock, MemoryFileSystem};
#[cfg(feature = "native")]
pub use native::{StdFileSystem, SystemClock};

/// The kind of a file system error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FsErrorKind {
    /// The path does not exist.
    NotFound,
    /// Access to the path is not permitted.
    PermissionDenied,
    /// The path is not a directory, but a directory is necessary.
    NotADirectory,
    /// The name is not valid UTF-8, or not valid in the catalog.
    InvalidName,
    /// A different error.
    Other,
}

/// A file system error for one path.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{path:?}: {message}")]
pub struct FsError {
    /// The path of the failed operation.
    pub path: PathBuf,
    /// The kind of error.
    pub kind: FsErrorKind,
    /// A description of the error.
    pub message: String,
}

impl FsError {
    /// Makes an error.
    pub fn new(path: impl Into<PathBuf>, kind: FsErrorKind, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            kind,
            message: message.into(),
        }
    }
}

/// The kind of a directory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryKind {
    /// A regular file (or a link to a regular file).
    File,
    /// A directory. Links to directories are [`EntryKind::Other`], so that
    /// a scan cannot loop.
    Dir,
    /// A different kind of entry (link to a directory, device, socket).
    Other,
}

/// One entry of a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// The name of the entry (not the full path).
    pub name: String,
    /// The kind of the entry.
    pub kind: EntryKind,
}

/// The contents of one directory.
///
/// An entry that cannot be read is put in `errors`. It does not cause the
/// full listing to fail.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirListing {
    /// The entries that were read.
    pub entries: Vec<DirEntry>,
    /// The entries that could not be read.
    pub errors: Vec<FsError>,
}

/// The metadata of one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileMetadata {
    /// The file size in bytes.
    pub size: u64,
    /// The modification time (Unix milliseconds).
    pub modified_ms: i64,
}

/// Read-only access to a file system.
pub trait FileSystem {
    /// Lists the entries of the directory at `path`.
    ///
    /// The order of the entries is not specified.
    fn read_dir(&self, path: &Path) -> Result<DirListing, FsError>;

    /// Reads the metadata of the file at `path`.
    fn metadata(&self, path: &Path) -> Result<FileMetadata, FsError>;
}

/// A source of the current time.
pub trait Clock {
    /// Returns the current time (Unix milliseconds).
    fn now_ms(&self) -> i64;
}
