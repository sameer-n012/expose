//! A file system scan that is done before the catalog is locked.
//!
//! [`expose_catalog::import_folder`] reads the file system and writes to
//! the store in turns. If the import runs on the real file system, it
//! holds the catalog lock during all the slow file system reads, and all
//! other catalog commands must wait.
//!
//! To prevent this, [`ScannedFileSystem::scan`] first reads all the
//! directory listings and file metadata that the import needs, with no
//! lock. Then the import runs on the scan result, which is in memory, and
//! holds the lock only for the database work.
//!
//! The scan follows the same rules as the import: it ignores hidden
//! entries (names that start with `.`), it reads subdirectories only for a
//! recursive import, and it reads metadata only for supported files.
//! Unlike the import, it also reads the metadata of files that the
//! catalog already knows, because it does not have the lock to ask.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use expose_catalog::{
    DirListing, EntryKind, FileMetadata, FileSystem, FsError, FsErrorKind, is_supported_file,
    normalize_folder_path,
};

/// The result of a scan. It replays the scanned results through the
/// [`FileSystem`] trait.
#[derive(Debug, Clone, Default)]
pub struct ScannedFileSystem {
    /// The result of `read_dir` for each scanned directory.
    dirs: HashMap<PathBuf, Result<DirListing, FsError>>,
    /// The result of `metadata` for each scanned file.
    files: HashMap<PathBuf, Result<FileMetadata, FsError>>,
}

impl ScannedFileSystem {
    /// Scans `root` (and its subdirectories if `recursive` is `true`)
    /// on `fs`.
    ///
    /// Errors from `fs` are kept in the result, and the import reports
    /// them as it does for the real file system.
    ///
    /// # Errors
    ///
    /// Returns an error if `root` is not a valid absolute path (see
    /// [`normalize_folder_path`]).
    pub fn scan(fs: &dyn FileSystem, root: &Path, recursive: bool) -> expose_catalog::Result<Self> {
        let root = normalize_folder_path(root)?;
        let mut scanned = Self::default();
        let mut pending = vec![root];
        while let Some(dir) = pending.pop() {
            let listing = fs.read_dir(&dir);
            if let Ok(listing) = &listing {
                for entry in &listing.entries {
                    if entry.name.starts_with('.') {
                        continue;
                    }
                    let path = dir.join(&entry.name);
                    match entry.kind {
                        EntryKind::Dir if recursive => pending.push(path),
                        EntryKind::File if is_supported_file(&entry.name) => {
                            let metadata = fs.metadata(&path);
                            scanned.files.insert(path, metadata);
                        }
                        EntryKind::Dir | EntryKind::File | EntryKind::Other => {}
                    }
                }
            }
            scanned.dirs.insert(dir, listing);
        }
        Ok(scanned)
    }
}

/// The error for a path that the scan did not read. This occurs only if
/// the import rules and the scan rules are different (a bug).
fn not_scanned(path: &Path) -> FsError {
    FsError::new(
        path,
        FsErrorKind::Other,
        "the path was not part of the scan before the import",
    )
}

impl FileSystem for ScannedFileSystem {
    fn read_dir(&self, path: &Path) -> Result<DirListing, FsError> {
        self.dirs
            .get(path)
            .cloned()
            .unwrap_or_else(|| Err(not_scanned(path)))
    }

    fn metadata(&self, path: &Path) -> Result<FileMetadata, FsError> {
        self.files
            .get(path)
            .cloned()
            .unwrap_or_else(|| Err(not_scanned(path)))
    }
}

#[cfg(test)]
mod tests {
    use expose_catalog::{CatalogStore, MemoryFileSystem, SqliteStore};

    use super::*;

    fn sample_fs() -> MemoryFileSystem {
        let mut fs = MemoryFileSystem::new();
        fs.add_file("/p/a.jpg", 10, 1);
        fs.add_file("/p/B.CR3", 20, 2);
        fs.add_file("/p/notes.txt", 1, 1);
        fs.add_file("/p/.hidden.jpg", 1, 1);
        fs.add_file("/p/.cache/c.jpg", 1, 1);
        fs.add_file("/p/sub/d.nef", 30, 3);
        fs.add_broken_file("/p/sub/bad.arw", "read failed");
        fs.add_unreadable_dir("/p/locked");
        fs
    }

    /// The import on the scan result gives the same report and the same
    /// catalog as the import on the original file system.
    #[test]
    fn import_on_scan_matches_direct_import() {
        let fs = sample_fs();
        for recursive in [false, true] {
            let mut direct = SqliteStore::open_in_memory().unwrap();
            let expected = direct
                .import_folder(&fs, Path::new("/p/"), recursive)
                .unwrap();

            let scanned = ScannedFileSystem::scan(&fs, Path::new("/p/"), recursive).unwrap();
            let mut store = SqliteStore::open_in_memory().unwrap();
            let report = store
                .import_folder(&scanned, Path::new("/p/"), recursive)
                .unwrap();

            assert_eq!(report, expected, "recursive = {recursive}");
            assert_eq!(
                store.list_folder_tree().unwrap(),
                direct.list_folder_tree().unwrap()
            );
        }
    }

    #[test]
    fn second_import_on_scan_skips_known_files() {
        let fs = sample_fs();
        let mut store = SqliteStore::open_in_memory().unwrap();
        let scanned = ScannedFileSystem::scan(&fs, Path::new("/p"), true).unwrap();
        let first = store
            .import_folder(&scanned, Path::new("/p"), true)
            .unwrap();
        let second = store
            .import_folder(&scanned, Path::new("/p"), true)
            .unwrap();
        assert_eq!(first.added, 3);
        assert_eq!(second.added, 0);
        assert_eq!(second.skipped, 3);
    }

    #[test]
    fn keeps_root_error_and_rejects_relative_root() {
        let fs = MemoryFileSystem::new();
        let scanned = ScannedFileSystem::scan(&fs, Path::new("/missing"), true).unwrap();
        let error = scanned.read_dir(Path::new("/missing")).unwrap_err();
        assert_eq!(error.kind, FsErrorKind::NotFound);

        assert!(ScannedFileSystem::scan(&fs, Path::new("rel/path"), true).is_err());
    }

    #[test]
    fn unscanned_paths_are_errors() {
        let scanned = ScannedFileSystem::default();
        assert!(scanned.read_dir(Path::new("/x")).is_err());
        assert!(scanned.metadata(Path::new("/x/a.jpg")).is_err());
    }
}
