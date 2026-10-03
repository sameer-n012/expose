//! Tests of the SQLite catalog store and of import.

#![cfg(all(feature = "sqlite", feature = "native"))]
// Test helpers may panic on failure, as tests do.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::time::Instant;

use expose_catalog::{
    AddOutcome, CatalogError, CatalogStore, ColorLabel, FixedClock, Flag, FolderNode, ImageInfo,
    LATEST_VERSION, LabelUpdate, MemoryFileSystem, NewPhoto, PhotoFilter, PhotoId, PhotoQuery,
    PhotoSort, Rating, SortDirection, SortField, SqliteStore, StdFileSystem,
};
use serde_json::json;

const NOW: i64 = 1_700_000_000_000;

fn store() -> SqliteStore {
    SqliteStore::open_in_memory_with_clock(Box::new(FixedClock::new(NOW))).unwrap()
}

fn rating(stars: u8) -> Rating {
    Rating::new(stars).unwrap()
}

fn new_photo(name: &str) -> NewPhoto {
    NewPhoto {
        file_name: name.to_owned(),
        file_size: 100,
        file_modified_ms: 0,
    }
}

/// Returns the paths of the tree in depth-first order, indented by depth.
fn tree_lines(nodes: &[FolderNode]) -> Vec<String> {
    fn walk(nodes: &[FolderNode], depth: usize, out: &mut Vec<String>) {
        for node in nodes {
            out.push(format!(
                "{}{} ({})",
                "  ".repeat(depth),
                node.folder.path.display(),
                node.photo_count
            ));
            walk(&node.children, depth + 1, out);
        }
    }
    let mut out = Vec::new();
    walk(nodes, 0, &mut out);
    out
}

// --- Schema and migrations ---

#[test]
fn fresh_file_is_migrated_with_wal_and_foreign_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), LATEST_VERSION);
    drop(store);

    let raw = rusqlite::Connection::open(&path).unwrap();
    let mode: String = raw
        .pragma_query_value(None, "journal_mode", |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    let app_id: i32 = raw
        .pragma_query_value(None, "application_id", |r| r.get(0))
        .unwrap();
    assert_eq!(app_id, expose_catalog::APPLICATION_ID);
    for table in ["folders", "photos", "develop_settings"] {
        let n: i64 = raw
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "{table}");
    }
}

#[test]
fn foreign_keys_are_enforced() {
    let mut store = store();
    let err = store.add_photos(expose_catalog::FolderId(42), &[new_photo("a.jpg")]);
    assert!(matches!(err, Err(CatalogError::FolderNotFound(_))));
}

#[test]
fn existing_file_reopens_with_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    let photo_id = {
        let mut store = SqliteStore::open(&path).unwrap();
        let folder = store.add_folder(Path::new("/p")).unwrap();
        let outcomes = store.add_photos(folder.id, &[new_photo("a.jpg")]).unwrap();
        let AddOutcome::Added(id) = outcomes[0] else {
            panic!("not added")
        };
        store.set_rating(&[id], rating(3)).unwrap();
        id
    };
    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), LATEST_VERSION);
    assert_eq!(
        store.get_photo(photo_id).unwrap().unwrap().rating,
        rating(3)
    );
}

#[test]
fn newer_schema_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    drop(SqliteStore::open(&path).unwrap());
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.pragma_update(None, "user_version", LATEST_VERSION + 1)
        .unwrap();
    drop(raw);

    match SqliteStore::open(&path) {
        Err(CatalogError::NewerSchema { found, supported }) => {
            assert_eq!(found, LATEST_VERSION + 1);
            assert_eq!(supported, LATEST_VERSION);
        }
        other => panic!("expected NewerSchema, got {other:?}"),
    }
}

#[test]
fn foreign_database_is_rejected_and_not_changed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("other.db");
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch("CREATE TABLE notes (body TEXT)").unwrap();
    drop(raw);

    assert!(matches!(
        SqliteStore::open(&path),
        Err(CatalogError::NotACatalog { .. })
    ));
    let raw = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = raw
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 0);
}

// --- Folder tree ---

#[test]
fn folder_tree_fills_gaps_and_adopts_descendants() {
    let mut store = store();
    // A deep folder first: it is a root, because the catalog has no ancestor.
    let deep = store.add_folder(Path::new("/x/y/z")).unwrap();
    assert_eq!(deep.parent_id, None);
    assert_eq!(deep.name, "z");

    // Adding an ancestor moves the deep folder under it and fills the gap.
    let x = store.add_folder(Path::new("/x/")).unwrap();
    let y = store.find_folder(Path::new("/x/y")).unwrap().unwrap();
    assert_eq!(y.parent_id, Some(x.id));
    assert_eq!(
        store.get_folder(deep.id).unwrap().unwrap().parent_id,
        Some(y.id)
    );

    // A descendant of a known folder gets all intermediate folders.
    store.add_folder(Path::new("/x/a/b/c")).unwrap();
    store.add_folder(Path::new("/other")).unwrap();

    // Adding a known folder again changes nothing.
    assert_eq!(store.add_folder(Path::new("/x/./y")).unwrap(), y);

    let lines = tree_lines(&store.list_folder_tree().unwrap());
    assert_eq!(
        lines,
        [
            "/other (0)",
            "/x (0)",
            "  /x/a (0)",
            "    /x/a/b (0)",
            "      /x/a/b/c (0)",
            "  /x/y (0)",
            "    /x/y/z (0)",
        ]
    );
    assert_eq!(store.list_folders().unwrap().len(), 7);
}

#[test]
fn adding_a_common_ancestor_joins_two_roots() {
    let mut store = store();
    store.add_folder(Path::new("/r/a/1")).unwrap();
    store.add_folder(Path::new("/r/b/2")).unwrap();
    store.add_folder(Path::new("/r")).unwrap();
    let lines = tree_lines(&store.list_folder_tree().unwrap());
    assert_eq!(
        lines,
        [
            "/r (0)",
            "  /r/a (0)",
            "    /r/a/1 (0)",
            "  /r/b (0)",
            "    /r/b/2 (0)",
        ]
    );
}

#[test]
fn invalid_folder_paths_are_rejected() {
    let mut store = store();
    for bad in ["relative/path", "/a/../b"] {
        assert!(matches!(
            store.add_folder(Path::new(bad)),
            Err(CatalogError::InvalidPath { .. })
        ));
    }
    assert!(store.list_folders().unwrap().is_empty());
}

// --- Import ---

fn sample_fs() -> MemoryFileSystem {
    let mut fs = MemoryFileSystem::new();
    fs.add_file("/pics/a.JPG", 10, 1_000);
    fs.add_file("/pics/b.cr3", 20, 2_000);
    fs.add_file("/pics/notes.txt", 5, 0);
    fs.add_file("/pics/.hidden.jpg", 5, 0);
    fs.add_file("/pics/.cache/c.jpg", 5, 0);
    fs.add_file("/pics/2024/trip/d.nef", 30, 3_000);
    fs.add_dir("/pics/empty");
    fs
}

#[test]
fn import_adds_supported_files_and_skips_hidden_ones() {
    let mut store = store();
    let report = store
        .import_folder(&sample_fs(), Path::new("/pics"), true)
        .unwrap();
    assert_eq!(report.root, PathBuf::from("/pics"));
    assert_eq!(report.added, 3);
    assert_eq!(report.added_ids.len(), 3);
    assert_eq!(report.skipped, 0);
    // notes.txt, .hidden.jpg, .cache
    assert_eq!(report.ignored, 3);
    assert!(report.errors.is_empty());

    // Folders are made only where photos are, plus the root and the gap.
    let lines = tree_lines(&store.list_folder_tree().unwrap());
    assert_eq!(
        lines,
        ["/pics (2)", "  /pics/2024 (0)", "    /pics/2024/trip (1)"]
    );

    let photo = store.get_photo(report.added_ids[0]).unwrap().unwrap();
    assert_eq!(photo.file_name, "a.JPG");
    assert_eq!(photo.file_size, 10);
    assert_eq!(photo.file_modified_ms, 1_000);
    assert_eq!(photo.imported_ms, NOW);
    assert_eq!(photo.capture_time_ms, None);
    assert_eq!(photo.rating, rating(0));
    assert_eq!(photo.flag, Flag::None);
    assert_eq!(photo.color_label, ColorLabel::None);
    assert_eq!(
        store.photo_path(photo.id).unwrap(),
        Some(PathBuf::from("/pics/a.JPG"))
    );
}

#[test]
fn non_recursive_import_reads_only_the_root() {
    let mut store = store();
    let report = store
        .import_folder(&sample_fs(), Path::new("/pics"), false)
        .unwrap();
    assert_eq!(report.added, 2);
    assert_eq!(store.list_folders().unwrap().len(), 1);
}

#[test]
fn import_is_idempotent_and_finds_new_files() {
    let mut fs = sample_fs();
    let mut store = store();
    let first = store.import_folder(&fs, Path::new("/pics"), true).unwrap();
    let second = store.import_folder(&fs, Path::new("/pics"), true).unwrap();
    assert_eq!(second.added, 0);
    assert_eq!(second.skipped, first.added);

    fs.add_file("/pics/2024/trip/e.arw", 1, 1);
    let third = store.import_folder(&fs, Path::new("/pics"), true).unwrap();
    assert_eq!(third.added, 1);
    assert_eq!(third.skipped, 3);

    // Import of a subfolder of known photos also skips them.
    let sub = store
        .import_folder(&fs, Path::new("/pics/2024"), true)
        .unwrap();
    assert_eq!((sub.added, sub.skipped), (0, 2));
    assert_eq!(store.list_folders().unwrap().len(), 3);
}

#[test]
fn bad_files_and_directories_do_not_stop_the_import() {
    let mut fs = sample_fs();
    fs.add_broken_file("/pics/broken.dng", "input/output error");
    fs.add_unreadable_dir("/pics/locked");
    fs.add_file("/pics/z.png", 1, 1);
    let mut store = store();
    let report = store.import_folder(&fs, Path::new("/pics"), true).unwrap();
    assert_eq!(report.added, 4);
    let mut error_paths: Vec<_> = report.errors.iter().map(|e| e.path.clone()).collect();
    error_paths.sort();
    assert_eq!(
        error_paths,
        [
            PathBuf::from("/pics/broken.dng"),
            PathBuf::from("/pics/locked")
        ]
    );
    assert!(
        report.errors[0].message.contains("error") || report.errors[1].message.contains("error")
    );
}

#[test]
fn missing_root_fails_with_no_change() {
    let mut store = store();
    let err = store
        .import_folder(&MemoryFileSystem::new(), Path::new("/nothing"), true)
        .unwrap_err();
    assert!(matches!(err, CatalogError::Fs(_)));
    assert!(store.list_folders().unwrap().is_empty());
}

#[test]
fn import_of_many_files_is_fast() {
    const DIRS: usize = 50;
    const FILES_PER_DIR: usize = 100;
    const BIG_DIR_FILES: usize = 2_500;

    let mut fs = MemoryFileSystem::new();
    for dir in 0..DIRS {
        for f in 0..FILES_PER_DIR {
            fs.add_file(format!("/lib/{dir:02}/day/IMG_{f:04}.CR3"), 25_000_000, 0);
        }
    }
    // One large directory exercises the batch split.
    for f in 0..BIG_DIR_FILES {
        fs.add_file(format!("/lib/big/DSC{f:05}.jpg"), 8_000_000, 0);
    }
    let total = DIRS * FILES_PER_DIR + BIG_DIR_FILES;

    let mut store = store();
    let start = Instant::now();
    let report = store.import_folder(&fs, Path::new("/lib"), true).unwrap();
    let first = start.elapsed();
    assert_eq!(report.added, total);
    assert!(report.errors.is_empty());

    let start = Instant::now();
    let again = store.import_folder(&fs, Path::new("/lib"), true).unwrap();
    let second = start.elapsed();
    assert_eq!(again.skipped, total);

    eprintln!("import of {total} files: {first:?}; re-import: {second:?}");
    // A generous limit, so that slow debug builds on CI also pass.
    assert!(first.as_secs_f64() < 10.0, "import took {first:?}");

    let root = store.find_folder(Path::new("/lib")).unwrap().unwrap();
    let mut query = PhotoQuery::new(root.id);
    query.recursive = true;
    assert_eq!(store.count_photos(&query).unwrap(), total as u64);
}

#[test]
fn real_file_system_import_does_not_change_originals() {
    let dir = tempfile::tempdir().unwrap();
    let photos = dir.path().join("photos");
    std::fs::create_dir_all(photos.join("sub")).unwrap();
    std::fs::write(photos.join("a.jpg"), b"jpeg bytes").unwrap();
    std::fs::write(photos.join("sub").join("b.DNG"), b"raw bytes").unwrap();
    std::fs::write(photos.join("readme.md"), b"text").unwrap();

    let mut store = SqliteStore::open(&dir.path().join("catalog.db")).unwrap();
    let report = store.import_folder(&StdFileSystem, &photos, true).unwrap();
    assert_eq!(report.added, 2);
    assert_eq!(report.ignored, 1);
    let photo = store.get_photo(report.added_ids[0]).unwrap().unwrap();
    assert_eq!(photo.file_size, 10);
    assert!(photo.file_modified_ms > 0);
    assert_eq!(std::fs::read(photos.join("a.jpg")).unwrap(), b"jpeg bytes");
}

// --- Labels ---

/// Makes a folder with photos `a.jpg` .. and returns their ids.
fn folder_with_photos(
    store: &mut SqliteStore,
    names: &[&str],
) -> (expose_catalog::FolderId, Vec<PhotoId>) {
    let folder = store.add_folder(Path::new("/f")).unwrap();
    let photos: Vec<_> = names.iter().map(|n| new_photo(n)).collect();
    let ids = store
        .add_photos(folder.id, &photos)
        .unwrap()
        .into_iter()
        .map(|o| match o {
            AddOutcome::Added(id) | AddOutcome::AlreadyKnown(id) => id,
        })
        .collect();
    (folder.id, ids)
}

#[test]
fn add_photos_reports_known_files() {
    let mut store = store();
    let (folder, ids) = folder_with_photos(&mut store, &["a.jpg", "b.jpg"]);
    let outcomes = store
        .add_photos(folder, &[new_photo("b.jpg"), new_photo("c.jpg")])
        .unwrap();
    assert_eq!(outcomes[0], AddOutcome::AlreadyKnown(ids[1]));
    assert!(matches!(outcomes[1], AddOutcome::Added(_)));
    assert!(matches!(
        store.add_photos(folder, &[new_photo("x/y.jpg")]),
        Err(CatalogError::InvalidPath { .. })
    ));
}

#[test]
fn batch_label_updates_apply_to_all_photos() {
    let mut store = store();
    let (_, ids) = folder_with_photos(&mut store, &["a.jpg", "b.jpg", "c.jpg"]);

    store.set_rating(&ids[..2], rating(5)).unwrap();
    store.set_flag(&ids, Flag::Pick).unwrap();
    store
        .set_color_label(&ids[2..], ColorLabel::Purple)
        .unwrap();
    store
        .update_labels(
            &ids[1..2],
            LabelUpdate {
                flag: Some(Flag::Reject),
                color_label: Some(ColorLabel::Green),
                ..LabelUpdate::default()
            },
        )
        .unwrap();

    let get = |id| store.get_photo(id).unwrap().unwrap();
    let (a, b, c) = (get(ids[0]), get(ids[1]), get(ids[2]));
    assert_eq!(
        (a.rating, a.flag, a.color_label),
        (rating(5), Flag::Pick, ColorLabel::None)
    );
    assert_eq!(
        (b.rating, b.flag, b.color_label),
        (rating(5), Flag::Reject, ColorLabel::Green)
    );
    assert_eq!(
        (c.rating, c.flag, c.color_label),
        (rating(0), Flag::Pick, ColorLabel::Purple)
    );
}

#[test]
fn batch_update_with_unknown_photo_changes_nothing() {
    let mut store = store();
    let (_, ids) = folder_with_photos(&mut store, &["a.jpg", "b.jpg"]);
    let err = store
        .set_rating(&[ids[0], PhotoId(9_999), ids[1]], rating(4))
        .unwrap_err();
    assert!(matches!(err, CatalogError::PhotoNotFound(PhotoId(9_999))));
    for id in ids {
        assert_eq!(store.get_photo(id).unwrap().unwrap().rating, rating(0));
    }
}

#[test]
fn ratings_out_of_range_are_rejected() {
    assert!(matches!(
        Rating::new(6),
        Err(expose_catalog::InvalidRating(6))
    ));
    let update: Result<LabelUpdate, _> = serde_json::from_str(r#"{"rating": 9}"#);
    assert!(update.is_err());
}

// --- Sort and filter ---

#[test]
// One-letter names match the file names, which makes the expected orders
// easy to read.
#[allow(clippy::many_single_char_names)]
fn sort_and_filter_photos() {
    use SortDirection::{Ascending as Asc, Descending as Desc};

    let mut store = store();
    let (folder, ids) = folder_with_photos(&mut store, &["b.jpg", "A.jpg", "c.jpg", "d.jpg"]);
    let (b, a, c, d) = (ids[0], ids[1], ids[2], ids[3]);
    // Capture times: c < a < b; d has none.
    for (id, t) in [(c, 10), (a, 20), (b, 30)] {
        store
            .set_image_info(
                id,
                ImageInfo {
                    capture_time_ms: Some(t),
                    width: Some(6000),
                    height: Some(4000),
                },
            )
            .unwrap();
    }
    store.set_rating(&[a], rating(3)).unwrap();
    store.set_rating(&[b, d], rating(5)).unwrap();
    store.set_flag(&[a, d], Flag::Pick).unwrap();
    store.set_color_label(&[d], ColorLabel::Red).unwrap();

    let list = |field, direction, filter| {
        let mut q = PhotoQuery::new(folder);
        q.sort = PhotoSort { field, direction };
        q.filter = filter;
        store
            .list_photos(&q)
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect::<Vec<_>>()
    };
    let none = PhotoFilter::default();

    assert_eq!(list(SortField::CaptureTime, Asc, none), [c, a, b, d]);
    assert_eq!(list(SortField::CaptureTime, Desc, none), [b, a, c, d]);
    assert_eq!(list(SortField::FileName, Asc, none), [a, b, c, d]);
    assert_eq!(list(SortField::FileName, Desc, none), [d, c, b, a]);
    // Equal ratings are ordered by file name.
    assert_eq!(list(SortField::Rating, Desc, none), [b, d, a, c]);
    assert_eq!(list(SortField::Rating, Asc, none), [c, a, b, d]);
    // All have the same import time, so the file name decides.
    assert_eq!(list(SortField::ImportTime, Asc, none), [a, b, c, d]);

    let min3 = PhotoFilter {
        min_rating: Some(rating(3)),
        ..none
    };
    assert_eq!(list(SortField::FileName, Asc, min3), [a, b, d]);
    let picks = PhotoFilter {
        flag: Some(Flag::Pick),
        ..none
    };
    assert_eq!(list(SortField::FileName, Asc, picks), [a, d]);
    let red_picks = PhotoFilter {
        color_label: Some(ColorLabel::Red),
        ..picks
    };
    assert_eq!(list(SortField::FileName, Asc, red_picks), [d]);

    let photo = store.get_photo(a).unwrap().unwrap();
    assert_eq!((photo.width, photo.height), (Some(6000), Some(4000)));
    assert!(matches!(
        store.set_image_info(
            a,
            ImageInfo {
                width: Some(0),
                ..ImageInfo::default()
            }
        ),
        Err(CatalogError::ZeroDimension { field: "width" })
    ));
}

#[test]
fn recursive_listing_limit_and_count() {
    let mut store = store();
    let mut fs = MemoryFileSystem::new();
    for path in [
        "/r/1.jpg",
        "/r/2.jpg",
        "/r/s/3.jpg",
        "/r/s/t/4.jpg",
        "/other/5.jpg",
    ] {
        fs.add_file(path, 1, 0);
    }
    store.import_folder(&fs, Path::new("/r"), true).unwrap();
    store.import_folder(&fs, Path::new("/other"), true).unwrap();
    let root = store.find_folder(Path::new("/r")).unwrap().unwrap();

    let mut q = PhotoQuery::new(root.id);
    q.sort.field = SortField::FileName;
    assert_eq!(store.count_photos(&q).unwrap(), 2);
    q.recursive = true;
    assert_eq!(store.count_photos(&q).unwrap(), 4);
    q.limit = Some(2);
    q.offset = 1;
    let names: Vec<_> = store
        .list_photos(&q)
        .unwrap()
        .into_iter()
        .map(|p| p.file_name)
        .collect();
    assert_eq!(names, ["2.jpg", "3.jpg"]);
    // The count ignores limit and offset.
    assert_eq!(store.count_photos(&q).unwrap(), 4);

    assert!(matches!(
        store.list_photos(&PhotoQuery::new(expose_catalog::FolderId(999))),
        Err(CatalogError::FolderNotFound(_))
    ));
}

// --- Develop settings ---

#[test]
fn develop_settings_round_trip() {
    let mut store = store();
    let (_, ids) = folder_with_photos(&mut store, &["a.jpg"]);
    let id = ids[0];
    assert_eq!(store.get_develop_settings(id).unwrap(), None);

    let first = json!({"exposure": 0.5, "curve": [[0, 0], [255, 255]]});
    let stored = store.set_develop_settings(id, 1, &first).unwrap();
    assert_eq!(stored.updated_ms, NOW);
    assert_eq!(store.get_develop_settings(id).unwrap(), Some(stored));

    let second = json!({"exposure": -1.25, "name": "ünïcode"});
    store.set_develop_settings(id, 2, &second).unwrap();
    let loaded = store.get_develop_settings(id).unwrap().unwrap();
    assert_eq!(loaded.schema_version, 2);
    assert_eq!(loaded.settings, second);

    assert!(matches!(
        store.set_develop_settings(PhotoId(77), 1, &first),
        Err(CatalogError::PhotoNotFound(_))
    ));
    assert!(matches!(
        store.get_develop_settings(PhotoId(77)),
        Err(CatalogError::PhotoNotFound(_))
    ));
}

#[test]
fn photo_serializes_to_camel_case_json() {
    let mut store = store();
    let (_, ids) = folder_with_photos(&mut store, &["a.jpg"]);
    let photo = store.get_photo(ids[0]).unwrap().unwrap();
    let value = serde_json::to_value(&photo).unwrap();
    assert_eq!(value["fileName"], "a.jpg");
    assert_eq!(value["rating"], 0);
    assert_eq!(value["flag"], "none");
    assert_eq!(value["colorLabel"], "none");
    assert_eq!(value["captureTimeMs"], serde_json::Value::Null);
    let back: expose_catalog::Photo = serde_json::from_value(value).unwrap();
    assert_eq!(back, photo);
}
