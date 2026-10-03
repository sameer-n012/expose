//! Path rules for catalog folders, and the folder tree builder.
//!
//! The catalog stores each folder as an absolute, normalized UTF-8 path.
//! The functions here do not access the file system. They do not resolve
//! symbolic links and do not change letter case.

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;
use std::path::{Component, Path, PathBuf};

use crate::error::{CatalogError, Result};
use crate::model::{Folder, FolderId, FolderNode};

/// Normalizes a folder path for storage in the catalog.
///
/// The result has no `.` components, no repeated separators, and no
/// trailing separator.
///
/// # Errors
///
/// Returns [`CatalogError::InvalidPath`] if the path is not absolute, has a
/// `..` component, or is not valid UTF-8.
pub fn normalize_folder_path(path: &Path) -> Result<PathBuf> {
    let invalid = |reason| CatalogError::InvalidPath {
        path: path.to_path_buf(),
        reason,
    };
    if !path.is_absolute() {
        return Err(invalid("the path is not absolute"));
    }
    if path.to_str().is_none() {
        return Err(invalid("the path is not valid UTF-8"));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => return Err(invalid("the path contains '..'")),
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}

/// Returns the path as text. The path must come from
/// [`normalize_folder_path`] or from the catalog, which are UTF-8.
// Only storage backends use this helper.
#[cfg_attr(not(feature = "sqlite"), allow(dead_code))]
pub(crate) fn path_text(path: &Path) -> Result<&str> {
    path.to_str().ok_or_else(|| CatalogError::InvalidPath {
        path: path.to_path_buf(),
        reason: "the path is not valid UTF-8",
    })
}

/// Returns the display name of a folder: its last component, or the full
/// path for a file system root.
pub fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Returns `true` if `name` is a valid file name for a photo: not empty,
/// not `.` or `..`, and with no path separator.
// Only storage backends use this helper.
#[cfg_attr(not(feature = "sqlite"), allow(dead_code))]
pub(crate) fn is_valid_file_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\'])
}

/// Builds the folder tree from a flat list of folders and their direct
/// photo counts.
///
/// A folder whose parent is not in the list becomes a root. Roots and
/// children are sorted by name (case-insensitive), then by path.
pub fn build_folder_tree<H: BuildHasher>(
    folders: Vec<Folder>,
    counts: &HashMap<FolderId, u64, H>,
) -> Vec<FolderNode> {
    let known: HashSet<FolderId> = folders.iter().map(|f| f.id).collect();
    let mut children: HashMap<Option<FolderId>, Vec<Folder>> = HashMap::new();
    for folder in folders {
        let parent = folder.parent_id.filter(|p| known.contains(p));
        children.entry(parent).or_default().push(folder);
    }
    build_level(None, &mut children, counts)
}

fn build_level<H: BuildHasher>(
    parent: Option<FolderId>,
    children: &mut HashMap<Option<FolderId>, Vec<Folder>>,
    counts: &HashMap<FolderId, u64, H>,
) -> Vec<FolderNode> {
    let mut level = children.remove(&parent).unwrap_or_default();
    level.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    level
        .into_iter()
        .map(|folder| {
            let id = folder.id;
            FolderNode {
                photo_count: counts.get(&id).copied().unwrap_or(0),
                children: build_level(Some(id), children, counts),
                folder,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_paths() {
        assert_eq!(
            normalize_folder_path(Path::new("/a//b/./c/")).unwrap(),
            PathBuf::from("/a/b/c")
        );
        assert_eq!(
            normalize_folder_path(Path::new("/")).unwrap(),
            PathBuf::from("/")
        );
    }

    #[test]
    fn rejects_relative_and_parent_paths() {
        assert!(matches!(
            normalize_folder_path(Path::new("a/b")),
            Err(CatalogError::InvalidPath { .. })
        ));
        assert!(matches!(
            normalize_folder_path(Path::new("/a/../b")),
            Err(CatalogError::InvalidPath { .. })
        ));
    }

    #[test]
    fn validates_file_names() {
        assert!(is_valid_file_name("IMG_0001.CR3"));
        for bad in ["", ".", "..", "a/b", "a\\b"] {
            assert!(!is_valid_file_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn folder_names() {
        assert_eq!(folder_name(Path::new("/a/b")), "b");
        assert_eq!(folder_name(Path::new("/")), "/");
    }

    #[test]
    fn builds_sorted_tree_with_orphans_as_roots() {
        let folder = |id, parent: Option<i64>, path: &str| Folder {
            id: FolderId(id),
            parent_id: parent.map(FolderId),
            path: PathBuf::from(path),
            name: folder_name(Path::new(path)),
        };
        let folders = vec![
            folder(1, None, "/p"),
            folder(2, Some(1), "/p/b"),
            folder(3, Some(1), "/p/A"),
            folder(4, Some(99), "/q"),
        ];
        let counts = HashMap::from([(FolderId(2), 7)]);
        let tree = build_folder_tree(folders, &counts);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].folder.path, PathBuf::from("/p"));
        assert_eq!(tree[1].folder.path, PathBuf::from("/q"));
        let names: Vec<_> = tree[0]
            .children
            .iter()
            .map(|n| n.folder.name.as_str())
            .collect();
        assert_eq!(names, ["A", "b"]);
        assert_eq!(tree[0].children[1].photo_count, 7);
        assert_eq!(tree[0].photo_count, 0);
    }
}
