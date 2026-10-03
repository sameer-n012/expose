//! In-memory platform services for tests and for hosts with no native
//! file system.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};

use super::{
    Clock, DirEntry, DirListing, EntryKind, FileMetadata, FileSystem, FsError, FsErrorKind,
};

#[derive(Debug, Clone)]
enum Node {
    Dir,
    File(FileMetadata),
    /// A file whose metadata cannot be read. The string is the error text.
    Broken(String),
}

/// An in-memory file system.
///
/// Use it to test file scans with no disk access. Paths must be absolute.
/// Parent directories are made automatically.
#[derive(Debug, Clone, Default)]
pub struct MemoryFileSystem {
    /// Directory path -> (entry name -> node).
    dirs: HashMap<PathBuf, BTreeMap<String, Node>>,
    /// Directories that fail when they are listed.
    unreadable: HashSet<PathBuf>,
}

impl MemoryFileSystem {
    /// Makes an empty file system.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a directory and all its missing parents.
    pub fn add_dir(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        if self.dirs.contains_key(path) {
            return;
        }
        self.dirs.insert(path.to_path_buf(), BTreeMap::new());
        self.link_to_parent(path, Node::Dir);
    }

    /// Adds a file and all its missing parent directories.
    pub fn add_file(&mut self, path: impl AsRef<Path>, size: u64, modified_ms: i64) {
        self.link_to_parent(
            path.as_ref(),
            Node::File(FileMetadata { size, modified_ms }),
        );
    }

    /// Adds a file whose metadata cannot be read. A call to
    /// [`FileSystem::metadata`] for it returns an error with `message`.
    pub fn add_broken_file(&mut self, path: impl AsRef<Path>, message: impl Into<String>) {
        self.link_to_parent(path.as_ref(), Node::Broken(message.into()));
    }

    /// Adds a directory that cannot be listed.
    pub fn add_unreadable_dir(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        self.add_dir(path);
        self.unreadable.insert(path.to_path_buf());
    }

    /// Puts `node` into the listing of the parent of `path`.
    /// A path with no parent or no UTF-8 name is a root: it is not linked.
    fn link_to_parent(&mut self, path: &Path, node: Node) {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str()))
        else {
            return;
        };
        self.add_dir(parent);
        if let Some(entries) = self.dirs.get_mut(parent) {
            entries.insert(name.to_owned(), node);
        }
    }

    fn node(&self, path: &Path) -> Option<&Node> {
        let parent = path.parent()?;
        let name = path.file_name()?.to_str()?;
        self.dirs.get(parent)?.get(name)
    }
}

impl FileSystem for MemoryFileSystem {
    fn read_dir(&self, path: &Path) -> Result<DirListing, FsError> {
        if self.unreadable.contains(path) {
            return Err(FsError::new(
                path,
                FsErrorKind::PermissionDenied,
                "the directory cannot be read",
            ));
        }
        let Some(entries) = self.dirs.get(path) else {
            let kind = if self.node(path).is_some() {
                FsErrorKind::NotADirectory
            } else {
                FsErrorKind::NotFound
            };
            return Err(FsError::new(path, kind, "the directory does not exist"));
        };
        let entries = entries
            .iter()
            .map(|(name, node)| DirEntry {
                name: name.clone(),
                kind: match node {
                    Node::Dir => EntryKind::Dir,
                    Node::File(_) | Node::Broken(_) => EntryKind::File,
                },
            })
            .collect();
        Ok(DirListing {
            entries,
            errors: Vec::new(),
        })
    }

    fn metadata(&self, path: &Path) -> Result<FileMetadata, FsError> {
        match self.node(path) {
            Some(Node::File(meta)) => Ok(*meta),
            Some(Node::Broken(message)) => Err(FsError::new(path, FsErrorKind::Other, message)),
            Some(Node::Dir) => Err(FsError::new(
                path,
                FsErrorKind::Other,
                "the path is not a regular file",
            )),
            None => Err(FsError::new(
                path,
                FsErrorKind::NotFound,
                "the file does not exist",
            )),
        }
    }
}

/// A clock that returns a set time. Use it to make tests deterministic.
#[derive(Debug, Default)]
pub struct FixedClock(AtomicI64);

impl FixedClock {
    /// Makes a clock that returns `now_ms`.
    pub fn new(now_ms: i64) -> Self {
        Self(AtomicI64::new(now_ms))
    }

    /// Changes the time that the clock returns.
    pub fn set(&self, now_ms: i64) {
        self.0.store(now_ms, Ordering::Relaxed);
    }
}

impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makes_parents_and_lists_entries() {
        let mut fs = MemoryFileSystem::new();
        fs.add_file("/a/b/c.jpg", 10, 1);
        fs.add_broken_file("/a/bad.nef", "read failed");

        let root = fs.read_dir(Path::new("/")).unwrap();
        assert_eq!(root.entries.len(), 1);
        assert_eq!(root.entries[0].kind, EntryKind::Dir);

        let a = fs.read_dir(Path::new("/a")).unwrap();
        let names: Vec<_> = a.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["b", "bad.nef"]);

        assert_eq!(fs.metadata(Path::new("/a/b/c.jpg")).unwrap().size, 10);
        assert!(fs.metadata(Path::new("/a/bad.nef")).is_err());
        assert_eq!(
            fs.read_dir(Path::new("/a/b/c.jpg")).unwrap_err().kind,
            FsErrorKind::NotADirectory
        );
        assert_eq!(
            fs.read_dir(Path::new("/x")).unwrap_err().kind,
            FsErrorKind::NotFound
        );
    }

    #[test]
    fn fixed_clock_can_change() {
        let clock = FixedClock::new(5);
        assert_eq!(clock.now_ms(), 5);
        clock.set(9);
        assert_eq!(clock.now_ms(), 9);
    }
}
