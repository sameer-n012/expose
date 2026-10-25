//! Tests of the editor output link.

#![cfg(all(feature = "sqlite", feature = "native"))]
// Test helpers may panic on failure, as tests do.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use expose_catalog::{
    AddOutcome, CatalogError, CatalogStore, EditorOutput, FixedClock, LATEST_VERSION, NewPhoto,
    PhotoId, SqliteStore,
};

const NOW: i64 = 1_700_000_000_000;

fn memory_store() -> SqliteStore {
    SqliteStore::open_in_memory_with_clock(Box::new(FixedClock::new(NOW))).unwrap()
}

/// Adds the files to the folder and returns their ids.
fn add(store: &mut SqliteStore, folder: &str, names: &[&str]) -> Vec<PhotoId> {
    let folder = store.add_folder(Path::new(folder)).unwrap();
    let photos: Vec<NewPhoto> = names
        .iter()
        .map(|n| NewPhoto {
            file_name: (*n).to_owned(),
            file_size: 1,
            file_modified_ms: 0,
        })
        .collect();
    store
        .add_photos(folder.id, &photos)
        .unwrap()
        .into_iter()
        .map(|o| match o {
            AddOutcome::Added(id) | AddOutcome::AlreadyKnown(id) => id,
        })
        .collect()
}

fn output(photo: PhotoId, doc: &str, source: Option<PhotoId>) -> EditorOutput {
    EditorOutput {
        photo_id: photo,
        document_path: PathBuf::from(doc),
        source_photo_id: source,
    }
}

#[test]
fn migration_from_version_one_keeps_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    // A database as version 1 made it, with one folder and one photo.
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch(
        r"
        CREATE TABLE folders (
            id INTEGER PRIMARY KEY,
            parent_id INTEGER REFERENCES folders(id) ON DELETE CASCADE,
            path TEXT NOT NULL UNIQUE
        ) STRICT;
        CREATE TABLE photos (
            id INTEGER PRIMARY KEY,
            folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
            file_name TEXT NOT NULL CHECK (length(file_name) > 0),
            file_size INTEGER NOT NULL CHECK (file_size >= 0),
            file_modified_ms INTEGER NOT NULL,
            imported_ms INTEGER NOT NULL,
            capture_time_ms INTEGER,
            width INTEGER CHECK (width IS NULL OR width > 0),
            height INTEGER CHECK (height IS NULL OR height > 0),
            rating INTEGER NOT NULL DEFAULT 0 CHECK (rating BETWEEN 0 AND 5),
            flag TEXT NOT NULL DEFAULT 'none' CHECK (flag IN ('none', 'pick', 'reject')),
            color_label TEXT NOT NULL DEFAULT 'none'
                CHECK (color_label IN ('none', 'red', 'yellow', 'green', 'blue', 'purple')),
            UNIQUE (folder_id, file_name)
        ) STRICT;
        CREATE TABLE develop_settings (
            photo_id INTEGER PRIMARY KEY REFERENCES photos(id) ON DELETE CASCADE,
            schema_version INTEGER NOT NULL CHECK (schema_version >= 0),
            settings_json TEXT NOT NULL CHECK (json_valid(settings_json)),
            updated_ms INTEGER NOT NULL
        ) STRICT;
        INSERT INTO folders (id, path) VALUES (1, '/pics');
        INSERT INTO photos (id, folder_id, file_name, file_size, file_modified_ms, imported_ms)
            VALUES (7, 1, 'a.jpg', 10, 0, 0);
        ",
    )
    .unwrap();
    raw.pragma_update(None, "application_id", expose_catalog::APPLICATION_ID)
        .unwrap();
    raw.pragma_update(None, "user_version", 1).unwrap();
    drop(raw);

    let mut store = SqliteStore::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), LATEST_VERSION);
    let id = store
        .find_photo_by_path(Path::new("/pics/a.jpg"))
        .unwrap()
        .unwrap();
    assert_eq!(id, PhotoId(7));
    assert!(!store.get_photo(id).unwrap().unwrap().is_editor_output);
    store
        .set_editor_output(&output(id, "/pics/a.exdoc", None))
        .unwrap();
    assert!(store.get_photo(id).unwrap().unwrap().is_editor_output);
}

#[test]
fn set_get_and_replace() {
    let mut store = memory_store();
    let ids = add(&mut store, "/pics", &["a.jpg", "a-edit.tif", "b.jpg"]);
    let (src, tif, other) = (ids[0], ids[1], ids[2]);

    assert_eq!(store.editor_output(tif).unwrap(), None);
    let first = output(tif, "/pics/a-edit.exdoc", Some(src));
    store.set_editor_output(&first).unwrap();
    assert_eq!(store.editor_output(tif).unwrap(), Some(first));

    // A second save replaces the record of the same photo.
    let second = output(tif, "/pics/./a-edit2.exdoc", Some(other));
    store.set_editor_output(&second).unwrap();
    let stored = store.editor_output(tif).unwrap().unwrap();
    assert_eq!(stored.document_path, Path::new("/pics/a-edit2.exdoc"));
    assert_eq!(stored.source_photo_id, Some(other));
    assert_eq!(
        store
            .editor_output_for_document(Path::new("/pics/a-edit.exdoc"))
            .unwrap(),
        None
    );
}

#[test]
fn unknown_photos_are_rejected() {
    let mut store = memory_store();
    let ids = add(&mut store, "/pics", &["a.tif"]);
    assert!(matches!(
        store.set_editor_output(&output(PhotoId(999), "/pics/x.exdoc", None)),
        Err(CatalogError::PhotoNotFound(PhotoId(999)))
    ));
    assert!(matches!(
        store.set_editor_output(&output(ids[0], "/pics/x.exdoc", Some(PhotoId(999)))),
        Err(CatalogError::PhotoNotFound(PhotoId(999)))
    ));
    assert!(matches!(
        store.set_editor_output(&output(ids[0], "relative.exdoc", None)),
        Err(CatalogError::InvalidPath { .. })
    ));
    assert_eq!(store.editor_output(ids[0]).unwrap(), None);
}

#[test]
fn lookups_by_document_and_by_photo_path() {
    let mut store = memory_store();
    let ids = add(&mut store, "/pics/sub", &["a-edit.tif", "plain.jpg"]);
    store
        .set_editor_output(&output(ids[0], "/pics/sub/a-edit.exdoc", None))
        .unwrap();

    let found = store
        .editor_output_for_document(Path::new("/pics/sub//a-edit.exdoc"))
        .unwrap()
        .unwrap();
    assert_eq!(found.photo_id, ids[0]);
    assert_eq!(
        store
            .editor_output_for_document(Path::new("/pics/sub/none.exdoc"))
            .unwrap(),
        None
    );

    assert_eq!(
        store
            .find_photo_by_path(Path::new("/pics/sub/plain.jpg"))
            .unwrap(),
        Some(ids[1])
    );
    assert_eq!(
        store
            .find_photo_by_path(Path::new("/pics/sub/nope.jpg"))
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .find_photo_by_path(Path::new("/other/plain.jpg"))
            .unwrap(),
        None
    );
}

#[test]
fn editor_output_path_checks() {
    let mut store = memory_store();
    let ids = add(&mut store, "/pics", &["a-edit.tif", "plain.jpg"]);
    store
        .set_editor_output(&output(ids[0], "/pics/a-edit.exdoc", None))
        .unwrap();

    assert!(
        store
            .is_editor_output_path(Path::new("/pics/a-edit.tif"))
            .unwrap()
    );
    assert!(
        store
            .is_editor_output_path(Path::new("/pics/a-edit.exdoc"))
            .unwrap()
    );
    assert!(
        !store
            .is_editor_output_path(Path::new("/pics/plain.jpg"))
            .unwrap()
    );
    assert!(
        !store
            .is_editor_output_path(Path::new("/pics/unknown.tif"))
            .unwrap()
    );
}

#[test]
fn delete_cascades_and_clears_source() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    let (src, tif) = {
        let mut store = SqliteStore::open(&path).unwrap();
        let ids = add(&mut store, "/pics", &["a.jpg", "a-edit.tif"]);
        store
            .set_editor_output(&output(ids[1], "/pics/a-edit.exdoc", Some(ids[0])))
            .unwrap();
        (ids[0], ids[1])
    };

    // The store has no delete method: use SQL with foreign keys on.
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.pragma_update(None, "foreign_keys", true).unwrap();
    raw.execute("DELETE FROM photos WHERE id = ?1", [src.0])
        .unwrap();
    drop(raw);

    let store = SqliteStore::open(&path).unwrap();
    let kept = store.editor_output(tif).unwrap().unwrap();
    assert_eq!(kept.source_photo_id, None);

    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.pragma_update(None, "foreign_keys", true).unwrap();
    raw.execute("DELETE FROM photos WHERE id = ?1", [tif.0])
        .unwrap();
    drop(raw);
    drop(store);

    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(store.editor_output(tif).unwrap(), None);
    assert_eq!(
        store
            .editor_output_for_document(Path::new("/pics/a-edit.exdoc"))
            .unwrap(),
        None
    );
}

fn new_output(image: &str, doc: &str, source: Option<PhotoId>) -> expose_catalog::NewEditorOutput {
    expose_catalog::NewEditorOutput {
        image_path: PathBuf::from(image),
        document_path: PathBuf::from(doc),
        file_size: 77,
        file_modified_ms: 5,
        info: expose_catalog::ImageInfo {
            capture_time_ms: Some(9),
            width: Some(4),
            height: Some(3),
        },
        source_photo_id: source,
    }
}

#[test]
fn add_editor_output_records_everything() {
    let mut store = memory_store();
    let src = add(&mut store, "/pics", &["a.jpg"])[0];
    let id = store
        .add_editor_output(&new_output(
            "/new/a-edit.tif",
            "/new/a-edit.exdoc",
            Some(src),
        ))
        .unwrap();
    let photo = store.get_photo(id).unwrap().unwrap();
    assert_eq!(photo.file_name, "a-edit.tif");
    assert_eq!((photo.file_size, photo.file_modified_ms), (77, 5));
    assert_eq!((photo.width, photo.height), (Some(4), Some(3)));
    assert!(photo.is_editor_output);
    let link = store.editor_output(id).unwrap().unwrap();
    assert_eq!(link.source_photo_id, Some(src));
    assert!(store.find_folder(Path::new("/new")).unwrap().is_some());
}

#[test]
fn add_editor_output_rolls_back_on_error() {
    let mut store = memory_store();
    let ids = add(&mut store, "/pics", &["a.tif", "b.tif"]);
    store
        .set_editor_output(&output(ids[0], "/pics/a.exdoc", None))
        .unwrap();
    let folders = store.list_folders().unwrap().len();

    // Same image path as a known photo.
    assert!(
        store
            .add_editor_output(&new_output("/pics/b.tif", "/pics/b.exdoc", None))
            .is_err()
    );
    // The document path of a different photo: fails at the last insert.
    assert!(
        store
            .add_editor_output(&new_output("/other/c.tif", "/pics/a.exdoc", None))
            .is_err()
    );
    // A source photo that does not exist.
    assert!(matches!(
        store.add_editor_output(&new_output(
            "/other/c.tif",
            "/other/c.exdoc",
            Some(PhotoId(999))
        )),
        Err(CatalogError::PhotoNotFound(_))
    ));
    assert_eq!(store.list_folders().unwrap().len(), folders);
    assert_eq!(
        store.find_photo_by_path(Path::new("/other/c.tif")).unwrap(),
        None
    );
}

#[test]
fn update_photo_file_changes_facts_and_info() {
    let mut store = memory_store();
    let id = add(&mut store, "/pics", &["a.tif"])[0];
    let info = expose_catalog::ImageInfo {
        capture_time_ms: None,
        width: Some(10),
        height: Some(20),
    };
    store.update_photo_file(id, 123, 456, info).unwrap();
    let photo = store.get_photo(id).unwrap().unwrap();
    assert_eq!((photo.file_size, photo.file_modified_ms), (123, 456));
    assert_eq!((photo.width, photo.height), (Some(10), Some(20)));
    assert!(matches!(
        store.update_photo_file(PhotoId(999), 1, 1, info),
        Err(CatalogError::PhotoNotFound(_))
    ));
    let zero = expose_catalog::ImageInfo {
        width: Some(0),
        ..info
    };
    assert!(store.update_photo_file(id, 1, 1, zero).is_err());
    assert_eq!(store.get_photo(id).unwrap().unwrap().file_size, 123);
}
