//! Photo library for Expose.
//!
//! # Responsibility
//!
//! This crate contains the library database (folders, photos, labels, and
//! develop settings) and the import of photo files in place. Later work
//! adds the preview cache and sidecar files.
//!
//! # Structure
//!
//! - [`CatalogStore`]: the storage trait. All catalog operations go through
//!   it.
//! - [`SqliteStore`]: the SQLite implementation (feature `sqlite`, on by
//!   default).
//! - [`FileSystem`] and [`Clock`]: platform services. The native
//!   implementations are [`StdFileSystem`] and [`SystemClock`] (feature
//!   `native`, on by default). [`MemoryFileSystem`] and [`FixedClock`] are
//!   for tests and for hosts with no native file system.
//! - [`import_folder`]: scans a folder and adds its photos.
//!
//! # Dependency rules
//!
//! - This crate can depend on `expose-core` only.
//! - Storage is behind a trait. The desktop build uses SQLite. A web build
//!   can supply a browser storage backend.
//! - The catalog never writes to original photo files.
//!
//! # Example
//!
//! ```
//! # #[cfg(all(feature = "sqlite", feature = "native"))]
//! # fn main() -> Result<(), expose_catalog::CatalogError> {
//! use std::path::Path;
//! use expose_catalog::{CatalogStore, MemoryFileSystem, PhotoQuery, Rating, SqliteStore};
//!
//! let mut fs = MemoryFileSystem::new();
//! fs.add_file("/photos/a.jpg", 1024, 0);
//!
//! let mut store = SqliteStore::open_in_memory()?;
//! let report = store.import_folder(&fs, Path::new("/photos"), true)?;
//! assert_eq!(report.added, 1);
//!
//! store.set_rating(&report.added_ids, Rating::new(4)?)?;
//! let folder = store.find_folder(Path::new("/photos"))?.expect("folder was imported");
//! let photos = store.list_photos(&PhotoQuery::new(folder.id))?;
//! assert_eq!(photos[0].rating.stars(), 4);
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "sqlite", feature = "native")))]
//! # fn main() {}
//! ```

mod error;
mod import;
mod model;
mod paths;
mod platform;
mod query;
#[cfg(feature = "sqlite")]
mod sqlite;
mod store;

pub use error::{CatalogError, Result};
pub use import::{
    ImportIssue, ImportReport, SUPPORTED_EXTENSIONS, import_folder, is_supported_file,
};
pub use model::{
    AddOutcome, BatchInfoOutcome, ColorLabel, DevelopSettings, EditorOutput, Flag, Folder,
    FolderId, FolderNode, ImageInfo, InvalidRating, LabelUpdate, NewEditorOutput, NewPhoto, Photo,
    PhotoId, Rating,
};
pub use paths::{build_folder_tree, folder_name, normalize_folder_path};
pub use platform::{
    Clock, DirEntry, DirListing, EntryKind, FileMetadata, FileSystem, FixedClock, FsError,
    FsErrorKind, MemoryFileSystem,
};
#[cfg(feature = "native")]
pub use platform::{StdFileSystem, SystemClock};
pub use query::{PhotoFilter, PhotoQuery, PhotoSort, SortDirection, SortField};
#[cfg(feature = "sqlite")]
pub use sqlite::{APPLICATION_ID, LATEST_VERSION, SqliteStore};
pub use store::CatalogStore;

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::CRATE_NAME;

    #[test]
    fn crate_name_is_correct() {
        assert_eq!(CRATE_NAME, "expose-catalog");
    }
}
