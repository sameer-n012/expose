//! Native platform services through the standard library.

use std::fs;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    Clock, DirEntry, DirListing, EntryKind, FileMetadata, FileSystem, FsError, FsErrorKind,
};

/// File system access through `std::fs`. All operations are read-only.
#[derive(Debug, Clone, Copy, Default)]
pub struct StdFileSystem;

/// The system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        // An i64 of milliseconds overflows only about 292 million years
        // from 1970, so saturation can occur only with a broken clock.
        system_time_to_unix_ms(SystemTime::now()).unwrap_or(i64::MAX)
    }
}

/// Converts a system time to signed Unix milliseconds.
/// Returns `None` if the value does not fit in an `i64`.
fn system_time_to_unix_ms(time: SystemTime) -> Option<i64> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).ok(),
        Err(before) => i64::try_from(before.duration().as_millis())
            .ok()
            .map(|ms| -ms),
    }
}

fn io_error(path: &Path, error: &io::Error) -> FsError {
    let kind = match error.kind() {
        io::ErrorKind::NotFound => FsErrorKind::NotFound,
        io::ErrorKind::PermissionDenied => FsErrorKind::PermissionDenied,
        io::ErrorKind::NotADirectory => FsErrorKind::NotADirectory,
        _ => FsErrorKind::Other,
    };
    FsError::new(path, kind, error.to_string())
}

impl FileSystem for StdFileSystem {
    fn read_dir(&self, path: &Path) -> Result<DirListing, FsError> {
        let mut listing = DirListing::default();
        for item in fs::read_dir(path).map_err(|e| io_error(path, &e))? {
            let entry = match item {
                Ok(entry) => entry,
                Err(e) => {
                    listing.errors.push(io_error(path, &e));
                    continue;
                }
            };
            let entry_path = entry.path();
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                listing.errors.push(FsError::new(
                    entry_path,
                    FsErrorKind::InvalidName,
                    "the file name is not valid UTF-8",
                ));
                continue;
            };
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(e) => {
                    listing.errors.push(io_error(&entry_path, &e));
                    continue;
                }
            };
            let kind = if file_type.is_dir() {
                EntryKind::Dir
            } else if file_type.is_file() {
                EntryKind::File
            } else if file_type.is_symlink() {
                // Follow links to files. Do not follow links to directories,
                // so that a recursive scan cannot loop.
                match fs::metadata(&entry_path) {
                    Ok(target) if target.is_file() => EntryKind::File,
                    Ok(_) => EntryKind::Other,
                    Err(e) => {
                        listing.errors.push(io_error(&entry_path, &e));
                        continue;
                    }
                }
            } else {
                EntryKind::Other
            };
            listing.entries.push(DirEntry { name, kind });
        }
        Ok(listing)
    }

    fn metadata(&self, path: &Path) -> Result<FileMetadata, FsError> {
        let meta = fs::metadata(path).map_err(|e| io_error(path, &e))?;
        if !meta.is_file() {
            return Err(FsError::new(
                path,
                FsErrorKind::Other,
                "the path is not a regular file",
            ));
        }
        let modified = meta.modified().map_err(|e| io_error(path, &e))?;
        let modified_ms = system_time_to_unix_ms(modified).ok_or_else(|| {
            FsError::new(
                path,
                FsErrorKind::Other,
                "the modification time is out of range",
            )
        })?;
        Ok(FileMetadata {
            size: meta.len(),
            modified_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn converts_times_before_and_after_epoch() {
        let after = UNIX_EPOCH + Duration::from_millis(1_500);
        assert_eq!(system_time_to_unix_ms(after), Some(1_500));
        let before = UNIX_EPOCH - Duration::from_millis(2_000);
        assert_eq!(system_time_to_unix_ms(before), Some(-2_000));
    }

    #[test]
    fn lists_real_directory() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.jpg"), b"12345").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();

        let fs_impl = StdFileSystem;
        let mut listing = fs_impl.read_dir(dir.path()).unwrap();
        listing.entries.sort_by(|a, b| a.name.cmp(&b.name));
        assert!(listing.errors.is_empty());
        assert_eq!(
            listing.entries,
            vec![
                DirEntry {
                    name: "a.jpg".into(),
                    kind: EntryKind::File
                },
                DirEntry {
                    name: "sub".into(),
                    kind: EntryKind::Dir
                },
            ]
        );
        let meta = fs_impl.metadata(&dir.path().join("a.jpg")).unwrap();
        assert_eq!(meta.size, 5);
        assert!(fs_impl.metadata(&dir.path().join("sub")).is_err());
        let missing = fs_impl.read_dir(&dir.path().join("missing")).unwrap_err();
        assert_eq!(missing.kind, FsErrorKind::NotFound);
    }
}
