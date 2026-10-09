//! Editor commands: open a library photo as a layered document, render the
//! canvas view, edit, undo, redo, and close (ADR 0013).
//!
//! `editor_render` returns the same binary `EXF1` frame as
//! `preview_render` (see [`crate::preview::frame`]), so the UI reuses its
//! frame parser.

use std::path::PathBuf;
use std::time::Instant;

use expose_catalog::{CatalogStore, PhotoId};
use expose_core::develop::DevelopSettings;
use expose_develop::{Pipeline, SourceInfo};
use expose_editor::ViewSpec;
use tauri::ipc::Response;
use tauri::{AppHandle, Runtime, State};

use super::preview::DisplaySpaceArg;
use super::{run_blocking, with_catalog};
use crate::editor::{
    DocumentId, DocumentInfo, EditDto, EditorEngine, RenderRequest, build_document,
};
use crate::error::{CommandError, ErrorKind};

/// Parses stored develop settings. A stored document that does not parse
/// or validate is an error (as for export): the editor must not show the
/// photo without the edits of the user.
fn parse_settings(stored: Option<serde_json::Value>) -> Result<DevelopSettings, CommandError> {
    let Some(value) = stored else {
        return Ok(DevelopSettings::default());
    };
    let mut settings = serde_json::from_value::<DevelopSettings>(value).map_err(|error| {
        CommandError::internal(format!(
            "the stored develop settings cannot be read: {error}"
        ))
    })?;
    settings.validate().map_err(|error| {
        CommandError::internal(format!(
            "the stored develop settings are not valid: {error}"
        ))
    })?;
    Ok(settings)
}

/// Opens a photo of the library as a document with one pixel layer
/// "Background": the photo after the full develop at source size (the
/// same develop as export).
///
/// `displaySpace` (`null` gives sRGB) is the color space of the frames.
///
/// Errors: `notFound` (photo or file), `internal` (decode, develop, or
/// stored settings), `invalidInput` (the photo is larger than the editor
/// limits), `unavailable` (no GPU).
#[tauri::command]
pub async fn editor_open<R: Runtime>(
    app: AppHandle<R>,
    engine: State<'_, EditorEngine>,
    photo_id: PhotoId,
    display_space: Option<DisplaySpaceArg>,
) -> Result<DocumentInfo, CommandError> {
    // Hold the catalog lock only to read the path and the settings.
    let (path, stored): (PathBuf, Option<serde_json::Value>) = with_catalog(app, move |state| {
        let store = state.lock()?;
        let path = store
            .photo_path(photo_id)?
            .ok_or_else(|| CommandError::new(ErrorKind::NotFound, "the photo does not exist"))?;
        let stored = store.get_develop_settings(photo_id)?.map(|s| s.settings);
        Ok((path, stored))
    })
    .await?;
    let started = Instant::now();
    let document = run_blocking(move || {
        let settings = parse_settings(stored)?;
        let decoded = expose_raw::decode_path(&path).map_err(|error| {
            let kind = if path.exists() {
                ErrorKind::Internal
            } else {
                ErrorKind::NotFound
            };
            CommandError::new(kind, format!("cannot decode {}: {error}", path.display()))
        })?;
        let mut image = decoded.image;
        // The decoders apply the as-shot white balance, so the decoded image
        // is a rendered source (as for export and the preview).
        let pipeline = Pipeline::new(&settings, &SourceInfo::RENDERED).map_err(|error| {
            CommandError::internal(format!("cannot apply the develop settings: {error}"))
        })?;
        if !pipeline.is_identity() {
            // All stages at source resolution (pixel scale 1).
            image = pipeline
                .develop_cpu_owned(image, 1.0)
                .map_err(|error| CommandError::internal(format!("cannot develop: {error}")))?;
        }
        Ok(build_document(image)?)
    })
    .await?;
    tracing::debug!(
        photo = photo_id.0,
        ms = started.elapsed().as_millis(),
        "editor document decoded and developed"
    );
    Ok(engine
        .open_async(document, display_space.unwrap_or_default().into())
        .await?)
}

/// Renders a view of a document and returns the frame body (`EXF1` header
/// and RGBA8 pixels, straight alpha, display color space) as raw binary
/// data. Pixels outside the canvas are transparent.
///
/// `originX`, `originY`: the canvas coordinate at the top-left output
/// pixel. `scale`: output pixels per canvas pixel. `width`, `height`: the
/// output size in pixels, `1..=8192`.
///
/// If a newer `editor_render` call for the same document arrives before
/// this one starts, this call fails with the kind `superseded` (latest
/// wins). The UI drops it.
///
/// Errors: `notFound` (document), `invalidInput` (view), `superseded`,
/// `unavailable` (no GPU), `internal`.
#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "the arguments are the fields of the view that the UI sends"
)]
pub async fn editor_render(
    engine: State<'_, EditorEngine>,
    document_id: DocumentId,
    origin_x: f64,
    origin_y: f64,
    scale: f64,
    width: u32,
    height: u32,
) -> Result<Response, CommandError> {
    let body = engine
        .render_async(RenderRequest {
            document: document_id,
            view: ViewSpec {
                origin_x,
                origin_y,
                scale,
                width,
                height,
            },
        })
        .await?;
    Ok(Response::new(body))
}

/// Applies one edit to a document and returns the new document info.
///
/// `gesture` (`null` or a number): edits with the same gesture number in
/// sequence are one undo step (for example, one slider drag). Use a new
/// number for each gesture.
///
/// Errors: `notFound` (document or layer), `invalidInput` (a value out of
/// range, a lock, or a move into the layer itself), `internal`.
#[tauri::command]
pub async fn editor_edit(
    engine: State<'_, EditorEngine>,
    document_id: DocumentId,
    edit: EditDto,
    gesture: Option<u64>,
) -> Result<DocumentInfo, CommandError> {
    Ok(engine.edit_async(document_id, edit, gesture).await?)
}

/// Undoes the last step of a document. With no step to undo, the document
/// does not change. Returns the new document info.
///
/// Errors: `notFound` (document), `internal`.
#[tauri::command]
pub async fn editor_undo(
    engine: State<'_, EditorEngine>,
    document_id: DocumentId,
) -> Result<DocumentInfo, CommandError> {
    Ok(engine.undo_async(document_id).await?)
}

/// Redoes the next step of a document. With no step to redo, the document
/// does not change. Returns the new document info.
///
/// Errors: `notFound` (document), `internal`.
#[tauri::command]
pub async fn editor_redo(
    engine: State<'_, EditorEngine>,
    document_id: DocumentId,
) -> Result<DocumentInfo, CommandError> {
    Ok(engine.redo_async(document_id).await?)
}

/// Closes a document and frees its memory. The edits are lost (the editor
/// cannot save yet). Closing an unknown document does nothing.
///
/// This command only queues the close, so it can run on the main thread.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri passes managed state by value"
)]
pub fn editor_close(engine: State<'_, EditorEngine>, document_id: DocumentId) {
    engine.close(document_id);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn missing_settings_give_the_defaults() {
        assert_eq!(parse_settings(None), Ok(DevelopSettings::default()));
    }

    #[test]
    fn bad_stored_settings_are_an_error() {
        let error = parse_settings(Some(json!({ "tone": "bright" }))).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Internal);
        assert!(
            error.message.contains("cannot be read"),
            "{}",
            error.message
        );
    }

    #[test]
    fn stored_settings_parse() {
        let mut settings = DevelopSettings::default();
        settings.tone.exposure = 1.0;
        let value = serde_json::to_value(&settings).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(parse_settings(Some(value)), Ok(settings));
    }
}
