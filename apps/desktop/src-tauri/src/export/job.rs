//! The export of the photos of one job, and the events of the job.

use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use expose_catalog::DevelopSettings as StoredSettings;
use expose_core::buffer::{AlphaMode, resize};
use expose_core::develop::DevelopSettings;
use expose_develop::{Pipeline, SourceInfo};
use serde::Serialize;

use super::manager::{JobControl, JobId};
use super::naming::{JobNames, Resolution, base_name};
use super::request::{ExportPlan, output_size};

/// The reason text of a [`ExportEvent::PhotoSkipped`] event when the
/// output file exists.
pub const SKIP_REASON_EXISTS: &str = "a file with this name exists in the destination";

/// An event of an export job. The UI receives it as JSON with a `kind`
/// field (camelCase).
///
/// The order is: one `started`, then one `photoDone`, `photoSkipped`, or
/// `photoFailed` for each photo that the job processes, then one
/// `finished`. `finished` is always the last event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ExportEvent {
    /// The job started.
    Started {
        /// The job.
        job_id: JobId,
        /// The number of photos (duplicates removed).
        total: u32,
    },
    /// The job wrote the file of a photo.
    PhotoDone {
        /// The job.
        job_id: JobId,
        /// The photo.
        photo_id: i64,
        /// The absolute path of the file.
        output_path: String,
        /// The position of the photo in the job, from 0.
        index: u32,
        /// The number of photos.
        total: u32,
    },
    /// The job did not write the file of a photo, because of the conflict
    /// policy.
    PhotoSkipped {
        /// The job.
        job_id: JobId,
        /// The photo.
        photo_id: i64,
        /// The path of the existing file.
        output_path: String,
        /// Why the photo was skipped.
        reason: String,
    },
    /// The export of a photo failed. The job continues with the next
    /// photo.
    PhotoFailed {
        /// The job.
        job_id: JobId,
        /// The photo.
        photo_id: i64,
        /// The cause.
        message: String,
    },
    /// The job stopped. This is the last event.
    Finished {
        /// The job.
        job_id: JobId,
        /// The number of written files.
        written: u32,
        /// The number of skipped photos.
        skipped: u32,
        /// The number of failed photos.
        failed: u32,
        /// `true` if a cancel stopped the job before all photos.
        cancelled: bool,
    },
}

/// The catalog data of one photo that export needs.
#[derive(Debug, Clone, PartialEq)]
pub struct PhotoSource {
    /// The original file.
    pub path: PathBuf,
    /// The stored develop settings, or `None` for the default settings.
    pub settings: Option<StoredSettings>,
}

/// Reads the [`PhotoSource`] of a photo from the catalog.
///
/// Returns `Err` with a message for the user if the photo does not exist
/// or the catalog fails. The function must hold the catalog lock only
/// while it reads.
pub type PhotoLoader<'a> = dyn Fn(i64) -> Result<PhotoSource, String> + 'a;

/// Returns the file names of the original files of all catalog photos in
/// a folder. Export never writes to these names.
///
/// Returns `Err` with a message for the user if the catalog fails.
pub type OriginalsLoader<'a> = dyn Fn(&Path) -> Result<HashSet<String>, String> + 'a;

/// The result of the export of one photo.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Written(PathBuf),
    Skipped(PathBuf),
}

/// Exports the photos of `plan` one after the other and sends the events
/// to `emit`.
///
/// The function checks for a cancel before each photo. It calls
/// [`JobControl::retire`] before it sends the `finished` event, so that a
/// cancel after `finished` gives "not found".
///
/// A panic in the export of one photo (for example, in a decoder) gives a
/// `photoFailed` event, and the job continues.
///
/// Before the first photo, the function reads the names of the catalog
/// originals in the destination with `originals`, one time. If this read
/// fails, each photo gives a `photoFailed` event: without the list, an
/// export could replace an original file.
pub fn run_job(
    plan: &ExportPlan,
    control: &JobControl,
    load: &PhotoLoader<'_>,
    originals: &OriginalsLoader<'_>,
    emit: &mut dyn FnMut(ExportEvent),
) {
    let job_id = control.id();
    // The request has at most `MAX_PHOTOS` photos, so the count fits.
    let total = u32::try_from(plan.photo_ids.len()).unwrap_or(u32::MAX);
    emit(ExportEvent::Started { job_id, total });

    // The destination folder in canonical form, to find the case where
    // the output is in the folder of the original.
    let destination = plan
        .destination
        .canonicalize()
        .unwrap_or_else(|_| plan.destination.clone());
    let mut names = originals(&plan.destination)
        .map(JobNames::with_reserved)
        .map_err(|error| {
            format!("cannot read the catalog photos of the destination folder: {error}")
        });
    let (mut written, mut skipped, mut failed) = (0_u32, 0_u32, 0_u32);
    let mut cancelled = false;

    for (index, &photo_id) in plan.photo_ids.iter().enumerate() {
        if control.is_cancelled() {
            cancelled = true;
            break;
        }
        let result = catch_unwind(AssertUnwindSafe(|| match &mut names {
            Ok(names) => export_photo(plan, &destination, names, photo_id, load),
            Err(message) => Err(message.clone()),
        }))
        .unwrap_or_else(|_| Err("an internal error occurred (a panic); see the log".to_owned()));
        match result {
            Ok(Outcome::Written(path)) => {
                written += 1;
                emit(ExportEvent::PhotoDone {
                    job_id,
                    photo_id,
                    output_path: path.to_string_lossy().into_owned(),
                    index: u32::try_from(index).unwrap_or(u32::MAX),
                    total,
                });
            }
            Ok(Outcome::Skipped(path)) => {
                skipped += 1;
                emit(ExportEvent::PhotoSkipped {
                    job_id,
                    photo_id,
                    output_path: path.to_string_lossy().into_owned(),
                    reason: SKIP_REASON_EXISTS.to_owned(),
                });
            }
            Err(message) => {
                failed += 1;
                tracing::warn!(job = job_id, photo = photo_id, %message, "the export of a photo failed");
                emit(ExportEvent::PhotoFailed {
                    job_id,
                    photo_id,
                    message,
                });
            }
        }
    }

    control.retire();
    tracing::info!(
        job = job_id,
        written,
        skipped,
        failed,
        cancelled,
        "export job finished"
    );
    emit(ExportEvent::Finished {
        job_id,
        written,
        skipped,
        failed,
        cancelled,
    });
}

/// Exports one photo: decode, develop at full size, resize, encode.
fn export_photo(
    plan: &ExportPlan,
    destination: &Path,
    names: &mut JobNames,
    photo_id: i64,
    load: &PhotoLoader<'_>,
) -> Result<Outcome, String> {
    let source = load(photo_id)?;
    let settings = parse_settings(source.settings.as_ref())?;

    // Select the name before the decode, so that a skip costs nothing.
    let base = base_name(&source.path, &plan.naming, photo_id);
    let protected = original_name_in(destination, &source.path);
    let resolution = names
        .resolve(
            &base,
            plan.encode.format.extension(),
            plan.on_conflict,
            protected.as_deref(),
            |name| std::fs::symlink_metadata(plan.destination.join(name)).is_ok(),
        )
        .map_err(|error| error.to_string())?;
    let output = match resolution {
        Resolution::Write(name) | Resolution::Overwrite(name) => plan.destination.join(name),
        Resolution::Skip(name) => return Ok(Outcome::Skipped(plan.destination.join(name))),
    };

    let decoded = expose_raw::decode_path(&source.path)
        .map_err(|error| format!("cannot read {}: {error}", source.path.display()))?;
    let mut image = decoded.image;

    let pipeline = Pipeline::new(&settings, &SourceInfo::RENDERED)
        .map_err(|error| format!("cannot apply the develop settings: {error}"))?;
    if !pipeline.is_identity() {
        pipeline.apply_cpu(&mut image);
    }

    let (width, height) = output_size(image.width(), image.height(), plan.size, plan.allow_upscale)
        .map_err(|error| error.to_string())?;
    let pixels = u64::from(width) * u64::from(height);
    if pixels > expose_raw::DEFAULT_MAX_PIXELS {
        return Err(format!(
            "the output size {width} x {height} has more than {} pixels",
            expose_raw::DEFAULT_MAX_PIXELS
        ));
    }
    if (width, height) != (image.width(), image.height()) {
        image = resize(&image, width, height, AlphaMode::Straight)
            .map_err(|error| format!("cannot resize the image: {error}"))?;
    }

    expose_raw::encode_path(&image, &plan.encode, &output)
        .map_err(|error| format!("cannot write {}: {error}", output.display()))?;
    Ok(Outcome::Written(output))
}

/// Parses the stored develop settings. Unlike the thumbnail cache, export
/// does not use the default settings in place of a document that is not
/// valid: the output would not show the edits of the user.
fn parse_settings(stored: Option<&StoredSettings>) -> Result<DevelopSettings, String> {
    let Some(stored) = stored else {
        return Ok(DevelopSettings::default());
    };
    let mut settings = serde_json::from_value::<DevelopSettings>(stored.settings.clone())
        .map_err(|error| format!("the stored develop settings cannot be read: {error}"))?;
    settings
        .validate()
        .map_err(|error| format!("the stored develop settings are not valid: {error}"))?;
    Ok(settings)
}

/// Returns the file name of `source` if `source` is in the folder
/// `destination` (canonical form). Export never replaces this file.
fn original_name_in(destination: &Path, source: &Path) -> Option<String> {
    let parent = source.parent()?.canonicalize().ok()?;
    (parent == destination).then(|| {
        source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    })?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_serialize_as_camel_case() {
        let event = ExportEvent::PhotoDone {
            job_id: 4,
            photo_id: 9,
            output_path: "/out/a.jpg".into(),
            index: 0,
            total: 2,
        };
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            serde_json::json!({
                "kind": "photoDone", "jobId": 4, "photoId": 9,
                "outputPath": "/out/a.jpg", "index": 0, "total": 2
            })
        );
        let finished = ExportEvent::Finished {
            job_id: 4,
            written: 1,
            skipped: 0,
            failed: 1,
            cancelled: false,
        };
        assert_eq!(
            serde_json::to_value(&finished).unwrap(),
            serde_json::json!({
                "kind": "finished", "jobId": 4, "written": 1,
                "skipped": 0, "failed": 1, "cancelled": false
            })
        );
        let skipped = ExportEvent::PhotoSkipped {
            job_id: 1,
            photo_id: 2,
            output_path: "/o".into(),
            reason: "r".into(),
        };
        assert_eq!(
            serde_json::to_value(&skipped).unwrap()["kind"],
            "photoSkipped"
        );
        let failed = ExportEvent::PhotoFailed {
            job_id: 1,
            photo_id: 2,
            message: "m".into(),
        };
        assert_eq!(
            serde_json::to_value(&failed).unwrap()["kind"],
            "photoFailed"
        );
        let started = ExportEvent::Started {
            job_id: 1,
            total: 3,
        };
        assert_eq!(
            serde_json::to_value(&started).unwrap(),
            serde_json::json!({ "kind": "started", "jobId": 1, "total": 3 })
        );
    }

    #[test]
    fn bad_stored_settings_are_an_error() {
        let stored = |settings| StoredSettings {
            photo_id: expose_catalog::PhotoId(1),
            schema_version: 1,
            settings,
            updated_ms: 0,
        };
        assert!(parse_settings(None).is_ok());
        assert!(parse_settings(Some(&stored(serde_json::json!("nonsense")))).is_err());
        let default = serde_json::to_value(DevelopSettings::default()).unwrap();
        assert!(parse_settings(Some(&stored(default))).is_ok());
    }
}
