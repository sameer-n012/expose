//! Commands for photo export.
//!
//! The logic is in [`crate::export`]. See that module for the flow and
//! the event order.

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, Runtime};

use super::run_blocking;
use crate::error::CommandError;
use crate::export::{self, ExportEvent, ExportManager, JobId};
use crate::state::CatalogState;

/// Validates an export request, adds the job to the export queue, and
/// returns the job id at once. The job sends its events to `on_event`.
///
/// The request is a JSON value, so that a value of the wrong type also
/// gives an `invalidInput` error (and not a Tauri argument error).
///
/// # Errors
///
/// - `invalidInput`: a value of the request is not valid, or the
///   destination is not an absolute path or is a file. No events are sent.
/// - `io`: the destination folder cannot be made.
/// - `unavailable`: the catalog did not open.
/// - `internal`: the export worker is not available.
#[tauri::command]
pub async fn export_start<R: Runtime>(
    app: AppHandle<R>,
    request: serde_json::Value,
    on_event: Channel<ExportEvent>,
) -> Result<JobId, CommandError> {
    let plan = export::request::validate(export::request::parse(request)?)?;
    let checker = app.clone();
    let plan = run_blocking(move || {
        checker
            .try_state::<CatalogState>()
            .ok_or_else(|| CommandError::internal("the catalog state is not registered"))?
            .ensure_available()?;
        export::prepare_destination(&plan.destination)?;
        Ok(plan)
    })
    .await?;

    let worker_app = app.clone();
    let manager = app
        .try_state::<ExportManager>()
        .ok_or_else(|| CommandError::internal("the export worker is not available"))?;
    manager.submit(Box::new(move |control| {
        let app = worker_app;
        let load = |photo_id| match app.try_state::<CatalogState>() {
            Some(state) => export::load_photo(state.inner(), photo_id),
            None => Err("the catalog state is not registered".to_owned()),
        };
        let mut emit = |event: ExportEvent| {
            if let Err(error) = on_event.send(event) {
                // The window can close while a job runs. The job continues,
                // because the files are still useful.
                tracing::warn!(job = control.id(), %error, "cannot send an export event");
            }
        };
        export::run_job(&plan, control, &load, &mut emit);
    }))
}

/// Requests a cancel of an export job. The job stops before its next
/// photo and then sends `finished` with `cancelled: true`.
///
/// # Errors
///
/// `notFound` if the job is not known or is finished.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri passes the application handle by value"
)]
pub fn export_cancel<R: Runtime>(app: AppHandle<R>, job_id: JobId) -> Result<(), CommandError> {
    app.try_state::<ExportManager>()
        .ok_or_else(|| CommandError::internal("the export worker is not available"))?
        .cancel(job_id)
}
