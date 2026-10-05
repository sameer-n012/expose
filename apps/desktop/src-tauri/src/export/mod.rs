//! Photo export: decode, develop at full size, resize, and encode to
//! JPEG, PNG, or TIFF in a destination folder.
//!
//! # Flow
//!
//! 1. The `export_start` command parses and validates the request
//!    ([`request::parse`], [`request::validate`]) and makes the
//!    destination folder ([`prepare_destination`]). A request that is not
//!    valid gives an error, and no events.
//! 2. The command adds the job to the [`ExportManager`] queue and returns
//!    the job id at once.
//! 3. The worker thread runs [`job::run_job`]. It sends the events to the
//!    UI through a channel.
//!
//! # Decisions
//!
//! - One job runs at a time, and a job exports one photo at a time. A
//!   24 MP image in the working format is approximately 384 MB, and the
//!   develop and resize steps need a second buffer. Two photos at a time
//!   could need more than 1.5 GB.
//! - Export uses the CPU develop pipeline. The GPU belongs to the preview
//!   render thread, and an export must not make the preview slow.
//! - The develop runs all stages at source resolution (geometry first,
//!   sharpening last) before the resize. Thus the output size and the
//!   pixel limit apply to the cropped image.
//! - The catalog lock is held only to read the path and the settings of a
//!   photo, never during decode or encode.
//! - Original files are read-only. Export never writes to the path of a
//!   catalog photo, with all conflict policies: such a name is reserved,
//!   and the output gets the next free `-N` name ([`original_names_in`]).

pub mod job;
pub mod manager;
pub mod naming;
pub mod request;

use std::collections::HashSet;
use std::path::Path;

use expose_catalog::{CatalogStore, FolderId, PhotoId};

pub use job::{ExportEvent, OriginalsLoader, PhotoLoader, PhotoSource, run_job};
pub use manager::{ExportManager, JobControl, JobId};
pub use request::{ExportPlan, ExportRequest};

use crate::error::{CommandError, ErrorKind};
use crate::state::CatalogState;

/// Makes sure that `destination` is a folder. Makes the folder (and its
/// parents) if it does not exist.
///
/// # Errors
///
/// - `InvalidInput` if `destination` exists and is not a folder.
/// - `Io` if the folder cannot be made.
pub fn prepare_destination(destination: &Path) -> Result<(), CommandError> {
    match std::fs::metadata(destination) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(CommandError::invalid_input(format!(
            "the destination {} is not a folder",
            destination.display()
        ))),
        Err(_) => std::fs::create_dir_all(destination).map_err(|error| {
            CommandError::new(
                ErrorKind::Io,
                format!(
                    "cannot make the destination folder {}: {error}",
                    destination.display()
                ),
            )
        }),
    }
}

/// Reads the path and the develop settings of a photo from the catalog.
/// The catalog lock is held only during this read.
///
/// # Errors
///
/// Returns a message for the user if the catalog is not available, the
/// photo does not exist, or the read fails.
pub fn load_photo(state: &CatalogState, photo_id: i64) -> Result<PhotoSource, String> {
    let id = PhotoId(photo_id);
    let store = state.lock().map_err(|error| error.message)?;
    let path = store
        .photo_path(id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("the photo {photo_id} does not exist"))?;
    let settings = store
        .get_develop_settings(id)
        .map_err(|error| error.to_string())?;
    Ok(PhotoSource { path, settings })
}

/// Returns the file names of the original files of all catalog photos in
/// the folder `destination`.
///
/// A catalog folder matches if its path is equal to `destination`, with
/// both paths in canonical form (if possible) and compared without case.
/// The comparison without case can match more folders than necessary on a
/// case-sensitive file system; this only reserves more names, so it is
/// safe.
///
/// The function locks the catalog two times for a short time: to list the
/// folders, and to read the names. The canonical form of each folder path
/// is made between the two locks, because it needs file system access.
///
/// # Errors
///
/// Returns a message for the user if the catalog is not available or a
/// read fails.
pub fn original_names_in(
    state: &CatalogState,
    destination: &Path,
) -> Result<HashSet<String>, String> {
    let key = |path: &Path| {
        path.canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .to_lowercase()
    };
    let target = key(destination);
    let folders = state
        .lock()
        .map_err(|error| error.message)?
        .list_folders()
        .map_err(|error| error.to_string())?;
    let matching: Vec<FolderId> = folders
        .into_iter()
        .filter(|folder| key(&folder.path) == target)
        .map(|folder| folder.id)
        .collect();
    if matching.is_empty() {
        return Ok(HashSet::new());
    }
    let store = state.lock().map_err(|error| error.message)?;
    let mut names = HashSet::new();
    for id in matching {
        names.extend(
            store
                .known_file_names(id)
                .map_err(|error| error.to_string())?,
        );
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use expose_catalog::SqliteStore;

    use super::*;
    use crate::{capture, service};

    fn request(destination: &Path, ids: &[i64], format: &serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "photoIds": ids,
            "destination": destination.to_str().unwrap(),
            "format": format,
            "colorSpace": "srgb",
            "size": { "mode": "percent", "percent": 400 },
            "allowUpscale": true,
            "naming": { "mode": "suffix", "suffix": "_out" },
            "onConflict": "uniqueName",
            "dither": false
        })
    }

    /// Runs one job on the manager and returns its events.
    fn run(
        manager: &ExportManager,
        state: &std::sync::Arc<CatalogState>,
        value: serde_json::Value,
    ) -> Vec<ExportEvent> {
        let plan = request::validate(request::parse(value).unwrap()).unwrap();
        prepare_destination(&plan.destination).unwrap();
        let (tx, rx) = mpsc::channel();
        let state = std::sync::Arc::clone(state);
        manager
            .submit(Box::new(move |control| {
                let load = |id| load_photo(&state, id);
                let originals = |folder: &Path| original_names_in(&state, folder);
                run_job(&plan, control, &load, &originals, &mut |event| {
                    tx.send(event).unwrap();
                });
            }))
            .unwrap();
        let mut events = Vec::new();
        loop {
            let event = rx.recv_timeout(Duration::from_secs(30)).unwrap();
            let last = matches!(event, ExportEvent::Finished { .. });
            events.push(event);
            if last {
                return events;
            }
        }
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one linear scenario with several jobs is easier to read"
    )]
    fn exports_a_photo_end_to_end() {
        let source = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("plain.png"), capture::tiny_png()).unwrap();
        std::fs::write(source.path().join("edited.png"), capture::tiny_png()).unwrap();

        let state = std::sync::Arc::new(CatalogState::from_store(
            None,
            SqliteStore::open_in_memory().unwrap(),
        ));
        let report = service::import_folder(
            &state,
            &expose_catalog::StdFileSystem,
            source.path().to_str().unwrap(),
            false,
        )
        .unwrap();
        let ids: Vec<i64> = report.added_ids.iter().map(|id| id.0).collect();
        assert_eq!(ids.len(), 2);
        let path_of = |id: i64| {
            state
                .lock()
                .unwrap()
                .photo_path(PhotoId(id))
                .unwrap()
                .unwrap()
        };
        let (plain, edited) = if path_of(ids[0]).ends_with("plain.png") {
            (ids[0], ids[1])
        } else {
            (ids[1], ids[0])
        };
        // A stored document that is not valid makes the photo fail.
        state
            .lock()
            .unwrap()
            .set_develop_settings(PhotoId(edited), 1, &serde_json::json!({ "version": "x" }))
            .unwrap();

        let manager = ExportManager::start().unwrap();
        let dest = out.path().join("new folder");
        let events = run(
            &manager,
            &state,
            request(
                &dest,
                &[plain, 999, plain, edited],
                &serde_json::json!({ "kind": "png", "bitDepth": 16 }),
            ),
        );
        assert_eq!(events.len(), 5, "{events:?}");
        let job_id = 1;
        assert_eq!(events[0], ExportEvent::Started { job_id, total: 3 });
        let expected_path = dest.join("plain_out.png");
        assert_eq!(
            events[1],
            ExportEvent::PhotoDone {
                job_id,
                photo_id: plain,
                output_path: expected_path.to_string_lossy().into_owned(),
                index: 0,
                total: 3
            }
        );
        assert!(
            matches!(&events[2], ExportEvent::PhotoFailed { photo_id: 999, message, .. } if message.contains("does not exist")),
            "{:?}",
            events[2]
        );
        assert!(
            matches!(&events[3], ExportEvent::PhotoFailed { photo_id, message, .. } if *photo_id == edited && message.contains("develop settings")),
            "{:?}",
            events[3]
        );
        assert_eq!(
            events[4],
            ExportEvent::Finished {
                job_id,
                written: 1,
                skipped: 0,
                failed: 2,
                cancelled: false
            }
        );
        let decoded = expose_raw::decode_path(&expected_path).unwrap();
        assert_eq!((decoded.image.width(), decoded.image.height()), (4, 4));
        assert_eq!(
            manager.cancel(job_id).unwrap_err().kind,
            ErrorKind::NotFound
        );

        // A second job with the same name gets a unique name; a third job
        // with "skip" skips the existing file.
        let jpeg = serde_json::json!({ "kind": "jpeg", "quality": 80, "subsampling": "420" });
        let events = run(&manager, &state, request(&dest, &[plain], &jpeg));
        assert!(
            matches!(&events[1], ExportEvent::PhotoDone { output_path, .. } if output_path.ends_with("plain_out.jpg")),
            "{events:?}"
        );
        let events = run(&manager, &state, request(&dest, &[plain], &jpeg));
        assert!(
            matches!(&events[1], ExportEvent::PhotoDone { output_path, .. } if output_path.ends_with("plain_out-2.jpg")),
            "{events:?}"
        );
        let mut skip = request(&dest, &[plain], &jpeg);
        skip["onConflict"] = serde_json::json!("skip");
        let events = run(&manager, &state, skip);
        assert!(
            matches!(&events[1], ExportEvent::PhotoSkipped { output_path, .. } if output_path.ends_with("plain_out.jpg")),
            "{events:?}"
        );
        assert!(matches!(
            events[2],
            ExportEvent::Finished { skipped: 1, .. }
        ));
    }

    #[test]
    fn cropped_photo_exports_at_the_cropped_size() {
        let source = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let mut image = expose_core::buffer::WorkingImage::new(40, 20).unwrap();
        for y in 0..20 {
            for x in 0..40 {
                #[allow(clippy::cast_precision_loss, reason = "small test values")]
                let v = x as f32 / 40.0;
                *image.pixel_mut(x, y).unwrap() = [v, 0.5, 1.0 - v, 1.0];
            }
        }
        let png = expose_raw::EncodeOptions::png(expose_raw::BitDepth::Eight, false);
        expose_raw::encode_path(&image, &png, source.path().join("wide.png")).unwrap();
        let state = std::sync::Arc::new(CatalogState::from_store(
            None,
            SqliteStore::open_in_memory().unwrap(),
        ));
        let report = service::import_folder(
            &state,
            &expose_catalog::StdFileSystem,
            source.path().to_str().unwrap(),
            false,
        )
        .unwrap();
        let id = report.added_ids[0];
        // The left half (20 x 20), with sharpening.
        state
            .lock()
            .unwrap()
            .set_develop_settings(
                id,
                1,
                &serde_json::json!({
                    "crop": { "rect": { "left": 0.0, "top": 0.0, "width": 0.5, "height": 1.0 } },
                    "sharpening": { "amount": 40.0 }
                }),
            )
            .unwrap();
        let mut value = request(
            out.path(),
            &[id.0],
            &serde_json::json!({ "kind": "png", "bitDepth": 8 }),
        );
        // 50 % of the cropped size, not of the source size.
        value["size"] = serde_json::json!({ "mode": "percent", "percent": 50 });
        let manager = ExportManager::start().unwrap();
        let events = run(&manager, &state, value);
        let ExportEvent::PhotoDone { output_path, .. } = &events[1] else {
            panic!("{events:?}");
        };
        let decoded = expose_raw::decode_path(Path::new(output_path)).unwrap();
        assert_eq!((decoded.image.width(), decoded.image.height()), (10, 10));
    }

    #[test]
    fn export_into_source_folder_keeps_the_original() {
        let source = tempfile::tempdir().unwrap();
        let original = capture::tiny_png();
        std::fs::write(source.path().join("a.png"), &original).unwrap();
        let state = std::sync::Arc::new(CatalogState::from_store(
            None,
            SqliteStore::open_in_memory().unwrap(),
        ));
        let report = service::import_folder(
            &state,
            &expose_catalog::StdFileSystem,
            source.path().to_str().unwrap(),
            false,
        )
        .unwrap();
        let mut value = request(
            source.path(),
            &[report.added_ids[0].0],
            &serde_json::json!({ "kind": "png", "bitDepth": 8 }),
        );
        value["naming"] = serde_json::json!({ "mode": "original" });
        value["onConflict"] = serde_json::json!("overwrite");
        let manager = ExportManager::start().unwrap();
        let events = run(&manager, &state, value);
        assert!(
            matches!(&events[1], ExportEvent::PhotoDone { output_path, .. } if output_path.ends_with("a-2.png")),
            "{events:?}"
        );
        assert_eq!(
            std::fs::read(source.path().join("a.png")).unwrap(),
            original
        );
    }

    #[test]
    fn export_never_writes_to_any_catalog_original() {
        let source = tempfile::tempdir().unwrap();
        let png = capture::tiny_png();
        // `a_out.png` is a catalog photo, and also the output name of `a`.
        // `a_out-2.png` is a catalog photo with another case.
        std::fs::write(source.path().join("a.png"), &png).unwrap();
        std::fs::write(source.path().join("a_out.png"), &png).unwrap();
        std::fs::write(source.path().join("A_OUT-2.png"), &png).unwrap();
        let state = std::sync::Arc::new(CatalogState::from_store(
            None,
            SqliteStore::open_in_memory().unwrap(),
        ));
        let report = service::import_folder(
            &state,
            &expose_catalog::StdFileSystem,
            source.path().to_str().unwrap(),
            false,
        )
        .unwrap();
        assert_eq!(report.added, 3);
        let a = report
            .added_ids
            .iter()
            .find(|id| {
                state
                    .lock()
                    .unwrap()
                    .photo_path(**id)
                    .unwrap()
                    .unwrap()
                    .ends_with("a.png")
            })
            .unwrap()
            .0;

        let names = original_names_in(&state, source.path()).unwrap();
        assert_eq!(names.len(), 3, "{names:?}");

        let manager = ExportManager::start().unwrap();
        for policy in ["uniqueName", "overwrite", "skip"] {
            // A destination path that is not in canonical form must match.
            let dest = source.path().join(".");
            let mut value = request(
                &dest,
                &[a],
                &serde_json::json!({ "kind": "png", "bitDepth": 8 }),
            );
            value["size"] = serde_json::json!({ "mode": "original" });
            value["onConflict"] = serde_json::json!(policy);
            let events = run(&manager, &state, value);
            let ExportEvent::PhotoDone { output_path, .. } = &events[1] else {
                panic!("{policy}: {events:?}");
            };
            assert!(
                output_path.ends_with("a_out-3.png"),
                "{policy}: {output_path}"
            );
            std::fs::remove_file(output_path).unwrap();
        }
        for name in ["a.png", "a_out.png", "A_OUT-2.png"] {
            assert_eq!(
                std::fs::read(source.path().join(name)).unwrap(),
                png,
                "{name}"
            );
        }
    }

    #[test]
    fn cancelled_job_finishes_with_cancelled() {
        let out = tempfile::tempdir().unwrap();
        let state = std::sync::Arc::new(CatalogState::from_store(
            None,
            SqliteStore::open_in_memory().unwrap(),
        ));
        let manager = ExportManager::start().unwrap();
        // Hold the worker, so that the cancel comes before the job starts.
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        manager
            .submit(Box::new(move |_| {
                gate_rx.recv_timeout(Duration::from_secs(30)).unwrap();
            }))
            .unwrap();
        let plan = request::validate(
            request::parse(request(
                out.path(),
                &[1, 2],
                &serde_json::json!({ "kind": "png", "bitDepth": 8 }),
            ))
            .unwrap(),
        )
        .unwrap();
        let (tx, rx) = mpsc::channel();
        let job = manager
            .submit(Box::new(move |control| {
                let load = |id| load_photo(&state, id);
                let originals = |folder: &Path| original_names_in(&state, folder);
                run_job(&plan, control, &load, &originals, &mut |event| {
                    tx.send(event).unwrap();
                });
            }))
            .unwrap();
        manager.cancel(job).unwrap();
        gate_tx.send(()).unwrap();
        let events: Vec<ExportEvent> = rx.iter().collect();
        assert_eq!(
            events,
            vec![
                ExportEvent::Started {
                    job_id: job,
                    total: 2
                },
                ExportEvent::Finished {
                    job_id: job,
                    written: 0,
                    skipped: 0,
                    failed: 0,
                    cancelled: true
                }
            ]
        );
    }

    /// Measures the export of one 24 MP photo. Run by hand in release:
    /// `cargo test -p expose-desktop --release export_24mp -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing measurement; run by hand"]
    fn export_24mp_timing() {
        let source = tempfile::tempdir().unwrap();
        let (width, height) = (6000_u32, 4000_u32);
        let pixels: Vec<expose_core::buffer::Rgba> = (0..width * height)
            .map(|i| {
                #[allow(clippy::cast_precision_loss, reason = "test gradient")]
                let x = (i % width) as f32 / width as f32;
                [x, 0.5 * x, 1.0 - x, 1.0]
            })
            .collect();
        let image = expose_core::buffer::WorkingImage::from_pixels(width, height, pixels).unwrap();
        let options = expose_raw::EncodeOptions::tiff(
            expose_raw::BitDepth::Sixteen,
            false,
            expose_raw::TiffCompression::None,
        );
        expose_raw::encode_path(&image, &options, source.path().join("big.tif")).unwrap();
        drop(image);

        let state = std::sync::Arc::new(CatalogState::from_store(
            None,
            SqliteStore::open_in_memory().unwrap(),
        ));
        let report = service::import_folder(
            &state,
            &expose_catalog::StdFileSystem,
            source.path().to_str().unwrap(),
            false,
        )
        .unwrap();
        let id = report.added_ids[0];
        state
            .lock()
            .unwrap()
            .set_develop_settings(
                id,
                1,
                &serde_json::json!({ "tone": { "exposure": 0.5, "contrast": 20.0 } }),
            )
            .unwrap();

        let manager = ExportManager::start().unwrap();
        let jpeg = serde_json::json!({ "kind": "jpeg", "quality": 90, "subsampling": "420" });
        for size in [
            serde_json::json!({ "mode": "original" }),
            serde_json::json!({ "mode": "longEdge", "pixels": 2048 }),
        ] {
            let out = tempfile::tempdir().unwrap();
            let mut value = request(out.path(), &[id.0], &jpeg);
            value["size"] = size.clone();
            let start = std::time::Instant::now();
            let events = run(&manager, &state, value);
            println!("24 MP export, size {size}: {:?}", start.elapsed());
            assert!(
                matches!(events[1], ExportEvent::PhotoDone { .. }),
                "{events:?}"
            );
        }
    }

    #[test]
    fn destination_must_be_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(
            prepare_destination(&file).unwrap_err().kind,
            ErrorKind::InvalidInput
        );
        assert_eq!(
            prepare_destination(&file.join("sub")).unwrap_err().kind,
            ErrorKind::Io
        );
        let nested = dir.path().join("a/b");
        prepare_destination(&nested).unwrap();
        assert!(nested.is_dir());
        prepare_destination(&nested).unwrap();
    }
}
