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
//! - The catalog lock is held only to read the path and the settings of a
//!   photo, never during decode or encode.

pub mod job;
pub mod manager;
pub mod naming;
pub mod request;

use std::path::Path;

use expose_catalog::{CatalogStore, PhotoId};

pub use job::{ExportEvent, PhotoSource, run_job};
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
                run_job(&plan, control, &load, &mut |event| tx.send(event).unwrap());
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
                run_job(&plan, control, &load, &mut |event| tx.send(event).unwrap());
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
