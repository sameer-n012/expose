//! Import of photo files in place (the files are not copied or moved).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::model::{AddOutcome, NewPhoto, PhotoId};
use crate::paths::normalize_folder_path;
use crate::platform::{DirListing, EntryKind, FileSystem, FsError};
use crate::store::CatalogStore;

/// The file extensions that import accepts. The comparison ignores letter
/// case.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    // Standard formats.
    "jpg", "jpeg", "png", "tif", "tiff", "webp", "heic", "heif", // Camera RAW formats.
    "dng", "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "raf", "orf", "rw2", "pef", "srw",
    "3fr", "iiq", "rwl", "x3f",
];

/// The maximum number of files that one `add_photos` call (one
/// transaction) receives.
const BATCH_SIZE: usize = 1000;

/// Returns `true` if the file name has a supported extension.
pub fn is_supported_file(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            SUPPORTED_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(ext))
        })
}

/// A problem with one file or directory during an import. The import
/// continues after it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportIssue {
    /// The file or directory that caused the problem.
    pub path: PathBuf,
    /// A description of the problem.
    pub message: String,
}

impl From<FsError> for ImportIssue {
    fn from(error: FsError) -> Self {
        Self {
            path: error.path,
            message: error.message,
        }
    }
}

/// The result of an import.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    /// The normalized root folder of the import.
    pub root: PathBuf,
    /// The number of photos that were added.
    pub added: usize,
    /// The number of supported files that the catalog already knew.
    pub skipped: usize,
    /// The number of entries that were not imported because they are
    /// hidden, have an unsupported extension, or are not regular files.
    pub ignored: usize,
    /// The ids of the added photos.
    pub added_ids: Vec<PhotoId>,
    /// The files and directories that could not be read.
    pub errors: Vec<ImportIssue>,
}

/// Imports the supported photo files in `root` in place (no copy).
///
/// The function:
/// - Reads the directory `root` and, if `recursive` is `true`, all its
///   subdirectories.
/// - Ignores hidden entries (names that start with `.`) and files with no
///   supported extension (see [`SUPPORTED_EXTENSIONS`]).
/// - Adds `root` as a folder. It adds a subfolder only if it contains at
///   least one new photo.
/// - Skips files that the catalog already knows. Thus a second import of
///   the same folder adds only new files.
/// - Does not read file contents. The capture time stays unknown.
///
/// A file or subdirectory that cannot be read is recorded in
/// [`ImportReport::errors`]. It does not stop the import.
///
/// # Errors
///
/// Returns an error if `root` is not a valid absolute path, if `root`
/// cannot be listed, or if the storage fails. If the storage fails during
/// the import, the batches that were written before the failure stay in
/// the catalog.
pub fn import_folder<S: CatalogStore + ?Sized>(
    store: &mut S,
    fs: &dyn FileSystem,
    root: &Path,
    recursive: bool,
) -> Result<ImportReport> {
    let root = normalize_folder_path(root)?;
    // A root that cannot be listed stops the import before any change.
    let root_listing = fs.read_dir(&root)?;
    store.add_folder(&root)?;

    let mut report = ImportReport {
        root: root.clone(),
        ..ImportReport::default()
    };
    let mut subdirs = Vec::new();
    import_dir(
        store,
        fs,
        &root,
        root_listing,
        recursive,
        &mut subdirs,
        &mut report,
    )?;

    // Depth-first walk. `subdirs` is a stack of directories to read.
    while let Some(dir) = subdirs.pop() {
        match fs.read_dir(&dir) {
            Ok(listing) => {
                import_dir(
                    store,
                    fs,
                    &dir,
                    listing,
                    recursive,
                    &mut subdirs,
                    &mut report,
                )?;
            }
            Err(error) => report.errors.push(error.into()),
        }
    }

    tracing::info!(
        root = %report.root.display(),
        added = report.added,
        skipped = report.skipped,
        ignored = report.ignored,
        errors = report.errors.len(),
        "import finished"
    );
    Ok(report)
}

/// Imports the files of one directory listing. Pushes the subdirectories
/// to `subdirs` if `recursive` is `true`.
fn import_dir<S: CatalogStore + ?Sized>(
    store: &mut S,
    fs: &dyn FileSystem,
    dir: &Path,
    listing: DirListing,
    recursive: bool,
    subdirs: &mut Vec<PathBuf>,
    report: &mut ImportReport,
) -> Result<()> {
    report
        .errors
        .extend(listing.errors.into_iter().map(ImportIssue::from));

    let mut entries = listing.entries;
    // Sort so that photo ids and the walk order are deterministic.
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    // Do not read metadata of files that the catalog already knows.
    let known = match store.find_folder(dir)? {
        Some(folder) => store.known_file_names(folder.id)?,
        None => std::collections::HashSet::new(),
    };

    let mut new_photos = Vec::new();
    for entry in entries {
        if entry.name.starts_with('.') {
            report.ignored += 1;
            continue;
        }
        let path = dir.join(&entry.name);
        match entry.kind {
            EntryKind::Dir if recursive => subdirs.push(path),
            EntryKind::File if is_supported_file(&entry.name) => {
                if known.contains(&entry.name) {
                    report.skipped += 1;
                    continue;
                }
                match fs.metadata(&path) {
                    Ok(meta) => new_photos.push(NewPhoto {
                        file_name: entry.name,
                        file_size: meta.size,
                        file_modified_ms: meta.modified_ms,
                    }),
                    Err(error) => {
                        tracing::warn!(path = %path.display(), %error, "cannot read file metadata");
                        report.errors.push(error.into());
                    }
                }
            }
            // A subdirectory of a non-recursive import is not counted.
            EntryKind::Dir => {}
            EntryKind::File | EntryKind::Other => report.ignored += 1,
        }
    }

    if new_photos.is_empty() {
        return Ok(());
    }
    let folder = store.add_folder(dir)?;
    for batch in new_photos.chunks(BATCH_SIZE) {
        for outcome in store.add_photos(folder.id, batch)? {
            match outcome {
                AddOutcome::Added(id) => {
                    report.added += 1;
                    report.added_ids.push(id);
                }
                AddOutcome::AlreadyKnown(_) => report.skipped += 1,
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_check_ignores_case() {
        for name in ["a.jpg", "B.JPEG", "c.Cr3", "d.3FR", "e.x3f", "f.HEIC"] {
            assert!(is_supported_file(name), "{name}");
        }
        for name in ["a.txt", "noext", "jpg", "a.jpg.xmp", "a."] {
            assert!(!is_supported_file(name), "{name}");
        }
    }
}
