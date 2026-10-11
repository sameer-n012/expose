// Test code: a failed setup is a test failure.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use expose_catalog::SqliteStore;
use expose_editor::{BlendMode, composite};

use super::*;
use crate::editor::session::build_document;

/// A temporary folder with one catalog photo `a.jpg` (capture time 1234).
struct Fixture {
    dir: tempfile::TempDir,
    state: CatalogState,
    source: PhotoId,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.jpg"), b"original").unwrap();
        let mut store = SqliteStore::open_in_memory().unwrap();
        let folder = store.add_folder(dir.path()).unwrap();
        let outcomes = store
            .add_photos(
                folder.id,
                &[NewPhoto {
                    file_name: "a.jpg".into(),
                    file_size: 8,
                    file_modified_ms: 1,
                }],
            )
            .unwrap();
        let AddOutcome::Added(source) = outcomes[0] else {
            panic!("the source photo was not added");
        };
        store
            .set_image_info(
                source,
                ImageInfo {
                    capture_time_ms: Some(1234),
                    width: Some(4),
                    height: Some(3),
                },
            )
            .unwrap();
        Self {
            dir,
            state: CatalogState::from_store(None, store),
            source,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn origin(&self) -> DocumentOrigin {
        DocumentOrigin {
            source_photo_id: Some(self.source),
            saved: None,
        }
    }

    fn photo_count(&self) -> usize {
        let store = self.state.lock().unwrap();
        let folder = store.find_folder(self.dir.path()).unwrap().unwrap();
        store.known_file_names(folder.id).unwrap().len()
    }

    /// The names of hidden (temporary) files in the folder.
    fn temp_files(&self) -> Vec<String> {
        fs::read_dir(self.dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with('.'))
            .collect()
    }
}

fn document(color: [f32; 4]) -> Document {
    build_document(WorkingImage::new_filled(4, 3, color).unwrap()).unwrap()
}

fn snapshot(document: Document, origin: DocumentOrigin) -> SaveSnapshot {
    let composite = composite(&document, document.bounds()).unwrap();
    let revision = document.revision();
    SaveSnapshot {
        document,
        composite,
        revision,
        origin,
    }
}

#[test]
fn first_save_creates_both_files_the_photo_and_the_link() {
    let fx = Fixture::new();
    let doc = document([0.5, 0.25, 0.125, 1.0]);
    let outcome = save_snapshot(&fx.state, snapshot(doc.clone(), fx.origin())).unwrap();
    assert!(outcome.created);
    assert_eq!(outcome.files.document_path, fx.path("a-edit.exdoc"));
    assert_eq!(outcome.files.image_path, fx.path("a-edit.tif"));
    assert!(outcome.files.document_path.is_file());
    assert!(outcome.files.image_path.is_file());
    assert!(fx.temp_files().is_empty(), "{:?}", fx.temp_files());
    assert_eq!(fs::read(fx.path("a.jpg")).unwrap(), b"original");

    let store = fx.state.lock().unwrap();
    let photo = store.get_photo(outcome.files.photo_id).unwrap().unwrap();
    assert!(photo.is_editor_output);
    assert_eq!(photo.file_name, "a-edit.tif");
    assert_eq!((photo.width, photo.height), (Some(4), Some(3)));
    assert_eq!(photo.capture_time_ms, Some(1234));
    let link = store
        .editor_output(outcome.files.photo_id)
        .unwrap()
        .unwrap();
    assert_eq!(link.document_path, fx.path("a-edit.exdoc"));
    assert_eq!(link.source_photo_id, Some(fx.source));
    drop(store);

    // The layered file reads back as the same document.
    assert_eq!(
        read_linked_document(&outcome.files.document_path).unwrap(),
        doc
    );
    // The TIFF decodes to the same colors (Rec. 2020 profile, 16 bits).
    let decoded = expose_raw::decode_path(&outcome.files.image_path).unwrap();
    assert_eq!((decoded.image.width(), decoded.image.height()), (4, 3));
    let pixel = decoded.image.pixels()[5];
    for (got, want) in pixel.iter().zip([0.5, 0.25, 0.125, 1.0]) {
        assert!((got - want).abs() < 2e-3, "{pixel:?}");
    }
}

#[test]
fn transparent_composite_keeps_alpha_and_opaque_has_none() {
    let fx = Fixture::new();
    let doc = document([0.4, 0.4, 0.4, 0.5]);
    let outcome = save_snapshot(&fx.state, snapshot(doc, fx.origin())).unwrap();
    let decoded = expose_raw::decode_path(&outcome.files.image_path).unwrap();
    let pixel = decoded.image.pixels()[0];
    assert!((pixel[3] - 0.5).abs() < 1e-3, "{pixel:?}");
    // Straight alpha in the file: the color is not darkened.
    assert!((pixel[0] - 0.4).abs() < 2e-3, "{pixel:?}");
}

#[test]
fn second_save_replaces_the_same_files_and_adds_no_photo() {
    let fx = Fixture::new();
    let first = save_snapshot(&fx.state, snapshot(document([0.2; 4]), fx.origin())).unwrap();
    let count = fx.photo_count();
    let mut doc = document([0.8, 0.1, 0.1, 1.0]);
    let id = doc.layers()[0].id();
    doc.set_blend_mode(id, BlendMode::Screen).unwrap();
    let origin = DocumentOrigin {
        source_photo_id: Some(fx.source),
        saved: Some(first.files.clone()),
    };
    let second = save_snapshot(&fx.state, snapshot(doc.clone(), origin)).unwrap();
    assert!(!second.created);
    assert_eq!(second.files, first.files);
    assert_eq!(fx.photo_count(), count);
    assert!(fx.temp_files().is_empty());
    assert_eq!(
        read_linked_document(&second.files.document_path).unwrap(),
        doc
    );
    let decoded = expose_raw::decode_path(&second.files.image_path).unwrap();
    assert!(decoded.image.pixels()[0][0] > 0.7);
}

#[test]
fn names_skip_files_on_disk_and_in_the_catalog() {
    let fx = Fixture::new();
    // `-edit`: the image name is on the disk.
    fs::write(fx.path("a-edit.tif"), b"other").unwrap();
    // `-edit-2`: the document name is in the catalog (not on the disk).
    {
        let mut store = fx.state.lock().unwrap();
        let folder = store.find_folder(fx.dir.path()).unwrap().unwrap();
        store
            .add_photos(
                folder.id,
                &[NewPhoto {
                    file_name: "a-edit-2.exdoc".into(),
                    file_size: 1,
                    file_modified_ms: 1,
                }],
            )
            .unwrap();
    }
    let targets = plan_targets(&*fx.state.lock().unwrap(), &fx.origin()).unwrap();
    assert_eq!(targets.document_path, fx.path("a-edit-3.exdoc"));
    assert_eq!(targets.image_path, fx.path("a-edit-3.tif"));
    assert_eq!(targets.photo_id, None);
    // The file of the other application stays.
    let outcome = save_snapshot(&fx.state, snapshot(document([0.3; 4]), fx.origin())).unwrap();
    assert_eq!(outcome.files.image_path, fx.path("a-edit-3.tif"));
    assert_eq!(fs::read(fx.path("a-edit.tif")).unwrap(), b"other");
}

#[test]
fn save_refuses_to_overwrite_files_that_expose_did_not_create() {
    let fx = Fixture::new();
    // A link that points at the original photo (a damaged or forged record).
    let forged = DocumentOrigin {
        source_photo_id: Some(fx.source),
        saved: Some(SavedFiles {
            photo_id: fx.source,
            document_path: fx.path("a-edit.exdoc"),
            image_path: fx.path("a.jpg"),
        }),
    };
    let error = save_snapshot(&fx.state, snapshot(document([0.3; 4]), forged)).unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidInput);
    assert!(
        error.message.contains("did not create"),
        "{}",
        error.message
    );
    assert_eq!(fs::read(fx.path("a.jpg")).unwrap(), b"original");
    assert!(!fx.path("a-edit.exdoc").exists());

    // A file on the disk at the document path of a real link, but the link
    // has a different photo: refused.
    let first = save_snapshot(&fx.state, snapshot(document([0.2; 4]), fx.origin())).unwrap();
    let other = DocumentOrigin {
        source_photo_id: Some(fx.source),
        saved: Some(SavedFiles {
            photo_id: fx.source,
            ..first.files
        }),
    };
    let error = plan_targets(&*fx.state.lock().unwrap(), &other).unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidInput);

    // A file on the disk that the catalog does not know.
    fs::write(fx.path("loose.tif"), b"loose").unwrap();
    let error = check_target(&*fx.state.lock().unwrap(), &fx.path("loose.tif"), None).unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidInput);
}

#[test]
fn missing_or_damaged_layered_file_is_an_error() {
    let fx = Fixture::new();
    let error = read_linked_document(&fx.path("gone.exdoc")).unwrap_err();
    assert_eq!(error.kind, ErrorKind::NotFound);
    assert!(error.message.contains("missing"), "{}", error.message);
    fs::write(fx.path("bad.exdoc"), b"not a document").unwrap();
    let error = read_linked_document(&fx.path("bad.exdoc")).unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidInput);
    assert!(
        error.message.contains("cannot be read"),
        "{}",
        error.message
    );
}

#[test]
fn atomic_write_replaces_or_leaves_no_trace() {
    let fx = Fixture::new();
    let target = fx.path("out.bin");
    write_atomic(&target, |out| {
        out.write_all(b"one")
            .map_err(|e| io_error("cannot write", &target, &e))
    })
    .unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"one");
    // A failed write keeps the old file and removes the temporary file.
    let error = write_atomic(&target, |out| {
        out.write_all(b"partial").unwrap();
        Err(CommandError::internal("stop"))
    })
    .unwrap_err();
    assert_eq!(error.message, "stop");
    assert_eq!(fs::read(&target).unwrap(), b"one");
    assert!(fx.temp_files().is_empty(), "{:?}", fx.temp_files());
    // A new file that fails does not exist after the call.
    let fresh = fx.path("fresh.bin");
    write_atomic(&fresh, |_| Err(CommandError::internal("stop"))).unwrap_err();
    assert!(!fresh.exists());
    assert!(fx.temp_files().is_empty());
    // A missing folder is `notFound`.
    let error = write_atomic(&fx.path("no/such.bin"), |_| Ok(())).unwrap_err();
    assert_eq!(error.kind, ErrorKind::NotFound);
}

#[test]
fn document_with_no_source_cannot_make_a_first_save() {
    let fx = Fixture::new();
    let error = plan_targets(&*fx.state.lock().unwrap(), &DocumentOrigin::default()).unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidInput);
}
