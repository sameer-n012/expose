//! The logic of the catalog commands, with no Tauri dependency.
//!
//! Each function takes the [`CatalogState`] and plain arguments, so that
//! tests can call it with no Tauri runtime. The functions in
//! [`crate::commands`] are thin wrappers around these functions.
//!
//! The functions block: they lock the catalog and do database work. Call
//! them on a blocking thread, not on the main thread.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use expose_catalog::{
    CatalogStore, ColorLabel, FileSystem, Flag, FolderNode, ImageInfo, ImportIssue, ImportReport,
    Photo, PhotoId, PhotoQuery, Rating,
};
use expose_core::develop::{DEVELOP_SETTINGS_VERSION, DevelopSettings};
use serde::Serialize;

use crate::capture::{self, MetadataJob, MetadataOutcome};
use crate::error::{CommandError, ErrorKind};
use crate::scan::ScannedFileSystem;
use crate::state::CatalogState;

/// The status of the catalog, as returned by `catalog_status`.
///
/// JSON: `{ "state": "ready", "path", "schemaVersion", "photoCount" }` or
/// `{ "state": "unavailable", "path", "error" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CatalogStatus {
    /// The catalog is open.
    Ready {
        /// The catalog file path.
        path: Option<String>,
        /// The schema version of the catalog file.
        schema_version: u32,
        /// The number of photos in the catalog.
        photo_count: u64,
    },
    /// The catalog did not open at startup.
    Unavailable {
        /// The catalog file path, or `None` if it is not known.
        path: Option<String>,
        /// The startup error.
        error: String,
    },
}

/// One page of a photo list, as returned by `list_photos`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoPage {
    /// The photos of the page (with the limit and offset of the query).
    pub photos: Vec<Photo>,
    /// The number of photos that match the query, with no limit or offset.
    pub total: u64,
}

/// Returns the status of the catalog.
///
/// # Errors
///
/// Returns an error if the catalog is open but cannot be read.
pub fn catalog_status(state: &CatalogState) -> Result<CatalogStatus, CommandError> {
    let path = state.path().map(|p| p.to_string_lossy().into_owned());
    if let Some(error) = state.startup_error() {
        return Ok(CatalogStatus::Unavailable {
            path,
            error: error.to_owned(),
        });
    }
    let store = state.lock()?;
    let schema_version = store.schema_version()?;
    // Each folder node counts only its direct photos, so the sum over the
    // full tree is the number of photos in the catalog.
    let photo_count = sum_photo_counts(&store.list_folder_tree()?);
    Ok(CatalogStatus::Ready {
        path,
        schema_version,
        photo_count,
    })
}

fn sum_photo_counts(nodes: &[FolderNode]) -> u64 {
    nodes
        .iter()
        .map(|node| node.photo_count + sum_photo_counts(&node.children))
        .sum()
}

/// Imports the supported photo files in the folder `path` in place.
///
/// The function scans `fs` first, with no lock (see [`crate::scan`]).
/// Then it locks the catalog only for the database work.
///
/// After the import, the function reads the metadata of each added file
/// with no lock (see [`crate::capture`]), and then locks the catalog again
/// to store the capture time and the image size of each photo.
///
/// A file whose metadata cannot be read or stored does not fail the
/// import. The photo stays in the catalog with no capture time and size,
/// and the problem is added to [`ImportReport::errors`]. A file with no
/// EXIF data is not a problem: it gets no capture time, but it keeps its
/// size if the file header gives it.
///
/// # Errors
///
/// Returns [`ErrorKind::InvalidInput`] if `path` is not an absolute path,
/// [`ErrorKind::NotFound`] if the folder does not exist, and other kinds
/// for catalog errors.
pub fn import_folder(
    state: &CatalogState,
    fs: &dyn FileSystem,
    path: &str,
    recursive: bool,
) -> Result<ImportReport, CommandError> {
    let root = Path::new(path);
    if !root.is_absolute() {
        return Err(CommandError::invalid_input(format!(
            "the import path {path:?} is not absolute"
        )));
    }
    // Do not scan the file system if the import cannot be written.
    state.ensure_available()?;
    let scanned = ScannedFileSystem::scan(fs, root, recursive)?;
    let mut report;
    let mut jobs = Vec::new();
    {
        let mut store = state.lock()?;
        report = store.import_folder(&scanned, root, recursive)?;
        for &id in &report.added_ids {
            match store.photo_path(id) {
                Ok(Some(path)) => jobs.push(MetadataJob { id, path }),
                Ok(None) => report.errors.push(ImportIssue {
                    path: root.to_path_buf(),
                    message: format!("photo {id} has no file path"),
                }),
                Err(error) => report.errors.push(ImportIssue {
                    path: root.to_path_buf(),
                    message: format!("cannot get the path of photo {id}: {error}"),
                }),
            }
        }
    } // The lock ends here: file reads must not block the catalog.

    let outcomes = capture::read_image_infos(&jobs)?;
    if outcomes.is_empty() {
        return Ok(report);
    }
    let plan = plan_info_batch(outcomes);
    report.errors.extend(plan.issues);
    if plan.batch.is_empty() {
        return Ok(report);
    }
    // One batch write, one transaction, one lock.
    let result = state.lock()?.set_image_infos(&plan.batch);
    match result {
        Ok(outcome) => {
            for id in outcome.unknown {
                report.errors.push(ImportIssue {
                    path: plan
                        .paths
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(|| root.to_path_buf()),
                    message: format!(
                        "photo {id} is not in the catalog: image information not stored"
                    ),
                });
            }
        }
        // The batch writes nothing on error. Report each photo, so the
        // user can see which files have no information.
        Err(error) => {
            for (id, _) in &plan.batch {
                report.errors.push(ImportIssue {
                    path: plan
                        .paths
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| root.to_path_buf()),
                    message: format!("cannot store the image information: {error}"),
                });
            }
        }
    }
    Ok(report)
}

/// The batch write of image information, and the problems found while
/// the batch was built.
struct InfoBatchPlan {
    /// The items to write.
    batch: Vec<(PhotoId, ImageInfo)>,
    /// The file path of each photo in `batch`.
    paths: HashMap<PhotoId, PathBuf>,
    /// One issue for each outcome that cannot be written.
    issues: Vec<ImportIssue>,
}

/// Builds the batch for `set_image_infos` from the metadata outcomes.
///
/// An outcome with a read error, or with a width or height of 0, becomes an
/// issue and is not in the batch. The store rejects the whole batch for a
/// zero size, so the filter stops one bad file from dropping the others.
fn plan_info_batch(outcomes: Vec<MetadataOutcome>) -> InfoBatchPlan {
    let mut plan = InfoBatchPlan {
        batch: Vec::new(),
        paths: HashMap::new(),
        issues: Vec::new(),
    };
    for outcome in outcomes {
        let message = match outcome.info {
            Err(message) => message,
            Ok(info) => {
                let zero = if info.width == Some(0) {
                    Some("width")
                } else if info.height == Some(0) {
                    Some("height")
                } else {
                    None
                };
                let Some(field) = zero else {
                    plan.paths.insert(outcome.id, outcome.path);
                    plan.batch.push((outcome.id, info));
                    continue;
                };
                format!("image {field} must be at least 1")
            }
        };
        plan.issues.push(ImportIssue {
            path: outcome.path,
            message,
        });
    }
    plan
}

/// Returns the folder tree, with the direct photo count of each folder.
///
/// # Errors
///
/// Returns an error if the catalog is not available or cannot be read.
pub fn folder_tree(state: &CatalogState) -> Result<Vec<FolderNode>, CommandError> {
    Ok(state.lock()?.list_folder_tree()?)
}

/// Returns one page of the photos that match `query`, and the total
/// number of matches.
///
/// # Errors
///
/// Returns [`ErrorKind::NotFound`] if the folder of the query does not
/// exist, and other kinds for catalog errors.
pub fn list_photos(state: &CatalogState, query: &PhotoQuery) -> Result<PhotoPage, CommandError> {
    let store = state.lock()?;
    // The store does not report a missing folder; it returns an empty
    // list. An explicit check lets the UI tell the difference.
    if store.get_folder(query.folder_id)?.is_none() {
        return Err(CommandError::new(
            ErrorKind::NotFound,
            format!("folder {} does not exist", query.folder_id),
        ));
    }
    let photos = store.list_photos(query)?;
    let total = store.count_photos(query)?;
    Ok(PhotoPage { photos, total })
}

/// Returns the photo with this id.
///
/// # Errors
///
/// Returns [`ErrorKind::NotFound`] if the photo does not exist.
pub fn get_photo(state: &CatalogState, id: PhotoId) -> Result<Photo, CommandError> {
    state
        .lock()?
        .get_photo(id)?
        .ok_or_else(|| CommandError::new(ErrorKind::NotFound, format!("photo {id} does not exist")))
}

/// Sets the star rating (0 to 5) of all photos in `ids`.
///
/// # Errors
///
/// Returns [`ErrorKind::InvalidInput`] if `rating` is more than 5, and
/// [`ErrorKind::NotFound`] if a photo does not exist. If an error occurs,
/// no photo changes.
pub fn set_rating(state: &CatalogState, ids: &[PhotoId], rating: u8) -> Result<(), CommandError> {
    let rating = Rating::new(rating).map_err(|e| CommandError::invalid_input(e.to_string()))?;
    Ok(state.lock()?.set_rating(ids, rating)?)
}

/// Sets the flag (`"none"`, `"pick"`, or `"reject"`) of all photos in
/// `ids`.
///
/// # Errors
///
/// Returns [`ErrorKind::InvalidInput`] if `flag` is not a known value, and
/// [`ErrorKind::NotFound`] if a photo does not exist. If an error occurs,
/// no photo changes.
pub fn set_flag(state: &CatalogState, ids: &[PhotoId], flag: &str) -> Result<(), CommandError> {
    let flag = Flag::from_str_opt(flag).ok_or_else(|| {
        CommandError::invalid_input(format!(
            "unknown flag {flag:?}; use one of: {}",
            join_names(Flag::ALL.map(Flag::as_str))
        ))
    })?;
    Ok(state.lock()?.set_flag(ids, flag)?)
}

/// Sets the color label (`"none"`, `"red"`, `"yellow"`, `"green"`,
/// `"blue"`, or `"purple"`) of all photos in `ids`.
///
/// # Errors
///
/// Returns [`ErrorKind::InvalidInput`] if `label` is not a known value,
/// and [`ErrorKind::NotFound`] if a photo does not exist. If an error
/// occurs, no photo changes.
pub fn set_color_label(
    state: &CatalogState,
    ids: &[PhotoId],
    label: &str,
) -> Result<(), CommandError> {
    let label = ColorLabel::from_str_opt(label).ok_or_else(|| {
        CommandError::invalid_input(format!(
            "unknown color label {label:?}; use one of: {}",
            join_names(ColorLabel::ALL.map(ColorLabel::as_str))
        ))
    })?;
    Ok(state.lock()?.set_color_label(ids, label)?)
}

/// Returns the develop settings of a photo as a JSON value.
///
/// A photo with no stored settings gives the default settings. Stored
/// settings are parsed and validated (finite out-of-range values are
/// clamped).
///
/// # Errors
///
/// Returns [`ErrorKind::NotFound`] if the photo does not exist. Returns
/// [`ErrorKind::Internal`] if the stored document does not parse or does
/// not validate. The error is not hidden: the UI shows the message.
pub fn get_develop_settings(
    state: &CatalogState,
    id: PhotoId,
) -> Result<serde_json::Value, CommandError> {
    let store = state.lock()?;
    if store.get_photo(id)?.is_none() {
        return Err(CommandError::new(
            ErrorKind::NotFound,
            format!("photo {id} does not exist"),
        ));
    }
    let settings = match store.get_develop_settings(id)? {
        None => DevelopSettings::default(),
        Some(stored) => {
            let mut settings: DevelopSettings = serde_json::from_value(stored.settings)
                .map_err(|e| stored_settings_error(id, &e.to_string()))?;
            settings
                .validate()
                .map_err(|e| stored_settings_error(id, &e.to_string()))?;
            settings
        }
    };
    settings_to_json(&settings)
}

/// Validates and stores the develop settings of a photo. Returns the
/// stored settings (after clamping).
///
/// # Errors
///
/// Returns [`ErrorKind::InvalidInput`] if `settings` does not parse or
/// does not validate, and [`ErrorKind::NotFound`] if the photo does not
/// exist. If an error occurs, the stored settings do not change.
pub fn set_develop_settings(
    state: &CatalogState,
    id: PhotoId,
    settings: serde_json::Value,
) -> Result<serde_json::Value, CommandError> {
    let mut settings: DevelopSettings = serde_json::from_value(settings).map_err(|e| {
        CommandError::invalid_input(format!("the develop settings are not valid: {e}"))
    })?;
    settings.validate().map_err(|e| {
        CommandError::invalid_input(format!("the develop settings are not valid: {e}"))
    })?;
    let value = settings_to_json(&settings)?;
    state
        .lock()?
        .set_develop_settings(id, DEVELOP_SETTINGS_VERSION, &value)?;
    Ok(value)
}

fn settings_to_json(settings: &DevelopSettings) -> Result<serde_json::Value, CommandError> {
    serde_json::to_value(settings)
        .map_err(|e| CommandError::internal(format!("cannot encode the develop settings: {e}")))
}

fn stored_settings_error(id: PhotoId, detail: &str) -> CommandError {
    CommandError::internal(format!(
        "the stored develop settings of photo {id} are not valid: {detail}"
    ))
}

fn join_names<const N: usize>(names: [&str; N]) -> String {
    names.join(", ")
}

#[cfg(test)]
mod tests {
    use expose_catalog::{FolderId, MemoryFileSystem, SqliteStore};

    use super::*;

    fn sample_fs() -> MemoryFileSystem {
        let mut fs = MemoryFileSystem::new();
        fs.add_file("/photos/a.jpg", 10, 1);
        fs.add_file("/photos/b.jpg", 20, 2);
        fs.add_file("/photos/trip/c.nef", 30, 3);
        fs.add_file("/photos/readme.txt", 1, 1);
        fs
    }

    fn open_state() -> CatalogState {
        let store = SqliteStore::open_in_memory().unwrap();
        CatalogState::from_store(None, store)
    }

    /// Returns a state with the sample photos imported, and the ids of the
    /// imported photos.
    fn imported_state() -> (CatalogState, Vec<PhotoId>) {
        let state = open_state();
        let report = import_folder(&state, &sample_fs(), "/photos", true).unwrap();
        (state, report.added_ids)
    }

    fn unavailable_state() -> CatalogState {
        CatalogState::failed(None, "disk full")
    }

    fn root_folder_id(state: &CatalogState) -> FolderId {
        folder_tree(state).unwrap()[0].folder.id
    }

    #[test]
    fn status_reports_ready_catalog() {
        let (state, _) = imported_state();
        let status = catalog_status(&state).unwrap();
        let CatalogStatus::Ready {
            path,
            schema_version,
            photo_count,
        } = status
        else {
            panic!("catalog must be ready: {status:?}");
        };
        assert_eq!(path, None);
        assert_eq!(schema_version, expose_catalog::LATEST_VERSION);
        assert_eq!(photo_count, 3);
    }

    #[test]
    fn status_reports_startup_error() {
        let state = CatalogState::failed(Some("/data/catalog".into()), "disk full");
        let status = catalog_status(&state).unwrap();
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            serde_json::json!({
                "state": "unavailable",
                "path": "/data/catalog",
                "error": "disk full",
            })
        );
    }

    #[test]
    fn status_json_uses_camel_case() {
        let status = CatalogStatus::Ready {
            path: Some("/c".into()),
            schema_version: 1,
            photo_count: 2,
        };
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            serde_json::json!({
                "state": "ready",
                "path": "/c",
                "schemaVersion": 1,
                "photoCount": 2,
            })
        );
    }

    #[test]
    fn import_adds_photos_and_skips_known_files() {
        let state = open_state();
        let fs = sample_fs();
        let first = import_folder(&state, &fs, "/photos", true).unwrap();
        assert_eq!(first.added, 3);
        assert_eq!(first.ignored, 1);
        let second = import_folder(&state, &fs, "/photos", true).unwrap();
        assert_eq!(second.added, 0);
        assert_eq!(second.skipped, 3);
    }

    /// A JPEG with EXIF: capture time 2023-07-14 21:05:33, size 32 x 16.
    const EXIF_JPEG: &[u8] =
        include_bytes!("../../../../crates/expose-raw/tests/fixtures/pillow_exif.jpg");

    #[test]
    fn import_stores_capture_time_and_size() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("exif.jpg"), EXIF_JPEG).unwrap();
        std::fs::write(dir.path().join("plain.png"), capture::tiny_png()).unwrap();
        std::fs::write(dir.path().join("broken.jpg"), b"not an image").unwrap();

        let state = open_state();
        let path = dir.path().to_str().unwrap();
        let report = import_folder(&state, &expose_catalog::StdFileSystem, path, false).unwrap();
        assert_eq!(report.added, 3);

        let by_name = |name: &str| {
            let id = report
                .added_ids
                .iter()
                .copied()
                .find(|&id| get_photo(&state, id).unwrap().file_name == name)
                .unwrap();
            get_photo(&state, id).unwrap()
        };
        let exif = by_name("exif.jpg");
        assert_eq!(exif.capture_time_ms, Some(1_689_368_733_000));
        assert_eq!((exif.width, exif.height), (Some(32), Some(16)));

        // No EXIF is normal: no capture time, but the size is known.
        let plain = by_name("plain.png");
        assert_eq!(plain.capture_time_ms, None);
        assert_eq!((plain.width, plain.height), (Some(1), Some(1)));

        // A corrupt file stays in the catalog and is reported.
        let broken = by_name("broken.jpg");
        assert_eq!(broken.capture_time_ms, None);
        assert_eq!((broken.width, broken.height), (None, None));
        assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
        assert_eq!(report.errors[0].path, dir.path().join("broken.jpg"));
        assert!(report.errors[0].message.contains("metadata"));
    }

    #[test]
    fn import_of_missing_files_reports_errors_but_succeeds() {
        // The in-memory file system has no real files to read. Each fake
        // file must give one metadata error, and no information is stored.
        let state = open_state();
        let report = import_folder(&state, &sample_fs(), "/photos", true).unwrap();
        assert_eq!(report.added_ids.len(), 3);
        assert_eq!(report.errors.len(), 3, "{:?}", report.errors);
        for issue in &report.errors {
            assert!(issue.message.contains("metadata"), "{issue:?}");
        }
        for &id in &report.added_ids {
            let photo = get_photo(&state, id).unwrap();
            assert_eq!(photo.capture_time_ms, None);
            assert_eq!((photo.width, photo.height), (None, None));
        }
    }

    fn outcome(id: PhotoId, name: &str, info: Result<ImageInfo, String>) -> MetadataOutcome {
        MetadataOutcome {
            id,
            path: PathBuf::from(name),
            info,
        }
    }

    fn info(width: Option<u32>, height: Option<u32>) -> ImageInfo {
        ImageInfo {
            capture_time_ms: Some(5),
            width,
            height,
        }
    }

    #[test]
    fn info_batch_filters_zero_sizes_and_read_errors() {
        let (_, ids) = imported_state();
        let plan = plan_info_batch(vec![
            outcome(ids[0], "good.jpg", Ok(info(Some(4), Some(3)))),
            outcome(ids[1], "zero_w.jpg", Ok(info(Some(0), Some(3)))),
            outcome(ids[2], "zero_h.jpg", Ok(info(Some(4), Some(0)))),
            outcome(ids[0], "unread.jpg", Err("cannot read the metadata".into())),
        ]);
        assert_eq!(plan.batch.len(), 1);
        assert_eq!(plan.batch[0].0, ids[0]);
        assert_eq!(plan.paths[&ids[0]], PathBuf::from("good.jpg"));
        let issues: Vec<_> = plan
            .issues
            .iter()
            .map(|i| (i.path.to_str().unwrap(), i.message.as_str()))
            .collect();
        assert_eq!(
            issues,
            [
                ("zero_w.jpg", "image width must be at least 1"),
                ("zero_h.jpg", "image height must be at least 1"),
                ("unread.jpg", "cannot read the metadata"),
            ]
        );
    }

    #[test]
    fn info_batch_with_zero_size_still_stores_the_other_photos() {
        let (state, ids) = imported_state();
        let plan = plan_info_batch(vec![
            outcome(ids[0], "a", Ok(info(Some(4), Some(3)))),
            outcome(ids[1], "b", Ok(info(Some(0), Some(0)))),
            outcome(ids[2], "c", Ok(info(Some(8), Some(6)))),
        ]);
        let written = state.lock().unwrap().set_image_infos(&plan.batch).unwrap();
        assert_eq!(written.written, 2);
        assert!(written.unknown.is_empty());
        let size = |id| {
            let p = get_photo(&state, id).unwrap();
            (p.width, p.height)
        };
        assert_eq!(size(ids[0]), (Some(4), Some(3)));
        assert_eq!(size(ids[1]), (None, None));
        assert_eq!(size(ids[2]), (Some(8), Some(6)));
    }

    #[test]
    fn import_does_not_read_known_files_again() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("exif.jpg"), EXIF_JPEG).unwrap();
        let state = open_state();
        let path = dir.path().to_str().unwrap();
        let fs = expose_catalog::StdFileSystem;
        import_folder(&state, &fs, path, false).unwrap();
        let second = import_folder(&state, &fs, path, false).unwrap();
        assert_eq!(second.added, 0);
        assert!(second.errors.is_empty());
    }

    #[test]
    fn import_rejects_relative_and_parent_paths() {
        let state = open_state();
        let fs = sample_fs();
        for path in ["photos", "./photos", ""] {
            let error = import_folder(&state, &fs, path, true).unwrap_err();
            assert_eq!(error.kind, ErrorKind::InvalidInput, "{path:?}");
        }
        let error = import_folder(&state, &fs, "/photos/../photos", true).unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidInput);
    }

    #[test]
    fn import_of_missing_or_unreadable_folder_fails() {
        let state = open_state();
        let mut fs = sample_fs();
        fs.add_unreadable_dir("/locked");
        let missing = import_folder(&state, &fs, "/missing", true).unwrap_err();
        assert_eq!(missing.kind, ErrorKind::NotFound);
        let file = import_folder(&state, &fs, "/photos/a.jpg", true).unwrap_err();
        assert_eq!(file.kind, ErrorKind::InvalidInput);
        let locked = import_folder(&state, &fs, "/locked", true).unwrap_err();
        assert_eq!(locked.kind, ErrorKind::Io);
        // A failed import does not add a folder.
        assert!(folder_tree(&state).unwrap().is_empty());
    }

    #[test]
    fn commands_fail_when_catalog_is_unavailable() {
        let state = unavailable_state();
        let fs = sample_fs();
        let ids = [PhotoId(1)];
        let errors = [
            import_folder(&state, &fs, "/photos", true).unwrap_err(),
            folder_tree(&state).unwrap_err(),
            list_photos(&state, &PhotoQuery::new(FolderId(1))).unwrap_err(),
            get_photo(&state, PhotoId(1)).unwrap_err(),
            set_rating(&state, &ids, 3).unwrap_err(),
            set_flag(&state, &ids, "pick").unwrap_err(),
            set_color_label(&state, &ids, "red").unwrap_err(),
            get_develop_settings(&state, PhotoId(1)).unwrap_err(),
            set_develop_settings(&state, PhotoId(1), serde_json::json!({})).unwrap_err(),
        ];
        for error in errors {
            assert_eq!(error.kind, ErrorKind::Unavailable, "{error}");
            assert!(error.message.contains("disk full"), "{error}");
        }
    }

    #[test]
    fn develop_settings_default_when_none_stored() {
        let (state, ids) = imported_state();
        let value = get_develop_settings(&state, ids[0]).unwrap();
        let settings: DevelopSettings = serde_json::from_value(value).unwrap();
        assert_eq!(settings, DevelopSettings::default());
        let missing = get_develop_settings(&state, PhotoId(999)).unwrap_err();
        assert_eq!(missing.kind, ErrorKind::NotFound);
    }

    #[test]
    fn develop_settings_round_trip_and_clamp() {
        let (state, ids) = imported_state();
        let stored = set_develop_settings(
            &state,
            ids[0],
            serde_json::json!({ "tone": { "exposure": 1.5, "contrast": 400.0 } }),
        )
        .unwrap();
        assert_eq!(stored["tone"]["exposure"], 1.5);
        assert_eq!(stored["tone"]["contrast"], 100.0);
        assert_eq!(get_develop_settings(&state, ids[0]).unwrap(), stored);
        // Other photos do not change.
        let other: DevelopSettings =
            serde_json::from_value(get_develop_settings(&state, ids[1]).unwrap()).unwrap();
        assert_eq!(other, DevelopSettings::default());
    }

    #[test]
    fn set_develop_settings_rejects_bad_input() {
        let (state, ids) = imported_state();
        let good = set_develop_settings(
            &state,
            ids[0],
            serde_json::json!({ "tone": { "exposure": 1.0 } }),
        )
        .unwrap();
        for bad in [
            serde_json::json!({ "tone": { "exposure": "high" } }),
            serde_json::json!({ "version": 99 }),
            serde_json::json!(5),
        ] {
            let error = set_develop_settings(&state, ids[0], bad).unwrap_err();
            assert_eq!(error.kind, ErrorKind::InvalidInput, "{error}");
        }
        assert_eq!(get_develop_settings(&state, ids[0]).unwrap(), good);
        let missing =
            set_develop_settings(&state, PhotoId(999), serde_json::json!({})).unwrap_err();
        assert_eq!(missing.kind, ErrorKind::NotFound);
    }

    #[test]
    fn stored_develop_settings_that_do_not_parse_give_an_error() {
        let (state, ids) = imported_state();
        state
            .lock()
            .unwrap()
            .set_develop_settings(ids[0], 1, &serde_json::json!({ "version": 99 }))
            .unwrap();
        let error = get_develop_settings(&state, ids[0]).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Internal);
        assert!(error.message.contains("not valid"), "{error}");
    }

    #[test]
    fn folder_tree_has_counts() {
        let (state, _) = imported_state();
        let tree = folder_tree(&state).unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].folder.path, Path::new("/photos"));
        assert_eq!(tree[0].photo_count, 2);
        assert_eq!(tree[0].children[0].folder.name, "trip");
        assert_eq!(tree[0].children[0].photo_count, 1);
    }

    #[test]
    fn list_photos_pages_and_counts() {
        let (state, _) = imported_state();
        let mut query = PhotoQuery::new(root_folder_id(&state));
        query.recursive = true;
        query.limit = Some(2);
        let page = list_photos(&state, &query).unwrap();
        assert_eq!(page.photos.len(), 2);
        assert_eq!(page.total, 3);

        let missing = list_photos(&state, &PhotoQuery::new(FolderId(999))).unwrap_err();
        assert_eq!(missing.kind, ErrorKind::NotFound);
    }

    #[test]
    fn list_photos_query_json_from_ui() {
        let (state, _) = imported_state();
        let json = serde_json::json!({
            "folderId": root_folder_id(&state).0,
            "recursive": false,
            "sort": { "field": "fileName", "direction": "descending" },
        });
        let query: PhotoQuery = serde_json::from_value(json).unwrap();
        let page = list_photos(&state, &query).unwrap();
        let names: Vec<_> = page.photos.iter().map(|p| p.file_name.as_str()).collect();
        assert_eq!(names, ["b.jpg", "a.jpg"]);
    }

    #[test]
    fn get_photo_finds_photo_or_fails() {
        let (state, ids) = imported_state();
        let photo = get_photo(&state, ids[0]).unwrap();
        assert_eq!(photo.id, ids[0]);
        let error = get_photo(&state, PhotoId(999)).unwrap_err();
        assert_eq!(error.kind, ErrorKind::NotFound);
    }

    #[test]
    fn set_rating_changes_all_or_fails() {
        let (state, ids) = imported_state();
        set_rating(&state, &ids, 4).unwrap();
        for &id in &ids {
            assert_eq!(get_photo(&state, id).unwrap().rating.stars(), 4);
        }

        let error = set_rating(&state, &ids, 6).unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidInput);

        // A missing photo stops the full change.
        let error = set_rating(&state, &[ids[0], PhotoId(999)], 1).unwrap_err();
        assert_eq!(error.kind, ErrorKind::NotFound);
        assert_eq!(get_photo(&state, ids[0]).unwrap().rating.stars(), 4);

        // An empty selection changes nothing.
        set_rating(&state, &[], 2).unwrap();
    }

    #[test]
    fn set_flag_parses_value() {
        let (state, ids) = imported_state();
        set_flag(&state, &ids[..1], "pick").unwrap();
        assert_eq!(get_photo(&state, ids[0]).unwrap().flag, Flag::Pick);
        assert_eq!(get_photo(&state, ids[1]).unwrap().flag, Flag::None);

        let error = set_flag(&state, &ids, "maybe").unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidInput);
        assert!(error.message.contains("none, pick, reject"), "{error}");
        let error = set_flag(&state, &[PhotoId(999)], "reject").unwrap_err();
        assert_eq!(error.kind, ErrorKind::NotFound);
    }

    #[test]
    fn set_color_label_parses_value() {
        let (state, ids) = imported_state();
        set_color_label(&state, &ids, "purple").unwrap();
        assert_eq!(
            get_photo(&state, ids[2]).unwrap().color_label,
            ColorLabel::Purple
        );

        let error = set_color_label(&state, &ids, "Orange").unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidInput);
        let error = set_color_label(&state, &[PhotoId(999)], "red").unwrap_err();
        assert_eq!(error.kind, ErrorKind::NotFound);
    }
}
