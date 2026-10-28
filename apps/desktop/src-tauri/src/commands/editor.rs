//! Editor commands: open a library photo as a layered document, render the
//! canvas view, edit, undo, redo, save, and close (ADR 0013, ADR 0015).
//!
//! `editor_render` returns the same binary `EXF1` frame as
//! `preview_render` (see [`crate::preview::frame`]), so the UI reuses its
//! frame parser.

use std::path::PathBuf;
use std::time::Instant;

use expose_catalog::{CatalogStore, PhotoId};
use expose_core::develop::DevelopSettings;
use expose_develop::{Pipeline, SourceInfo};
use expose_editor::{Document, ViewSpec};
use tauri::ipc::Response;
use tauri::{AppHandle, Manager, Runtime, State};

use super::preview::DisplaySpaceArg;
use super::{run_blocking, with_catalog};
use crate::editor::save::{read_linked_document, save_snapshot};
use crate::editor::{
    DocumentId, DocumentInfo, DocumentOrigin, EditDto, EditorEngine, EditorEngineError,
    NewDocumentSpec, RenderRequest, SaveGuard, SaveResult, SavedFiles, build_document,
};
use crate::error::{CommandError, ErrorKind};
use crate::state::CatalogState;
use crate::thumbnails::{ThumbnailInput, ThumbnailService};

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

/// What `editor_open` reads for a photo.
#[derive(Debug, Clone, PartialEq)]
enum OpenSource {
    /// The photo is the flattened image of a save: read the layered file.
    Linked {
        /// The layered document file.
        document_path: PathBuf,
        /// The origin of the document (the link).
        origin: DocumentOrigin,
    },
    /// A normal photo: decode and develop it.
    Photo {
        /// The photo file.
        path: PathBuf,
        /// The stored develop settings, if any.
        stored: Option<serde_json::Value>,
    },
}

/// Reads from the catalog what an open of `photo_id` needs.
fn open_source<S: CatalogStore + ?Sized>(
    store: &S,
    photo_id: PhotoId,
) -> Result<OpenSource, CommandError> {
    let path = store
        .photo_path(photo_id)?
        .ok_or_else(|| CommandError::new(ErrorKind::NotFound, "the photo does not exist"))?;
    if let Some(link) = store.editor_output(photo_id)? {
        return Ok(OpenSource::Linked {
            document_path: link.document_path.clone(),
            origin: DocumentOrigin {
                source_photo_id: link.source_photo_id,
                saved: Some(SavedFiles {
                    photo_id,
                    document_path: link.document_path,
                    image_path: path,
                }),
                name: None,
            },
        });
    }
    let stored = store.get_develop_settings(photo_id)?.map(|s| s.settings);
    Ok(OpenSource::Photo { path, stored })
}

/// Opens a photo of the library as a document.
///
/// - A photo that is the flattened image of an editor save opens its
///   layered `.exdoc` file. A missing or damaged file is an error; the
///   command does not open the flattened photo in its place.
/// - Any other photo opens as a document with one pixel layer
///   "Background": the photo after the full develop at source size (the
///   same develop as export).
///
/// `displaySpace` (`null` gives sRGB) is the color space of the frames.
///
/// Errors: `notFound` (photo, file, or layered file), `internal` (decode,
/// develop, or stored settings), `invalidInput` (the photo is larger than
/// the editor limits, or the layered file is damaged), `unavailable` (no
/// GPU).
#[tauri::command]
pub async fn editor_open<R: Runtime>(
    app: AppHandle<R>,
    engine: State<'_, EditorEngine>,
    photo_id: PhotoId,
    display_space: Option<DisplaySpaceArg>,
) -> Result<DocumentInfo, CommandError> {
    // Hold the catalog lock only to read the path, the link, and the
    // settings.
    let source = with_catalog(app, move |state| open_source(&*state.lock()?, photo_id)).await?;
    let started = Instant::now();
    let (document, origin) = match source {
        OpenSource::Linked {
            document_path,
            origin,
        } => {
            let document = run_blocking(move || read_linked_document(&document_path)).await?;
            (document, origin)
        }
        OpenSource::Photo { path, stored } => {
            let document = develop_document(path, stored).await?;
            let origin = DocumentOrigin {
                source_photo_id: Some(photo_id),
                saved: None,
                name: None,
            };
            (document, origin)
        }
    };
    tracing::debug!(
        photo = photo_id.0,
        ms = started.elapsed().as_millis(),
        "editor document read"
    );
    Ok(engine
        .open_async(document, origin, display_space.unwrap_or_default().into())
        .await?)
}

/// Makes a new document with one background layer and opens it. The
/// document is not dirty. It has no source photo, so the first save needs
/// a folder (save as) and uses the name of the spec as the file stem.
///
/// Errors: `invalidInput` (name, size, resolution, or bit depth),
/// `unavailable` (no GPU).
#[tauri::command]
pub async fn editor_new(
    engine: State<'_, EditorEngine>,
    spec: NewDocumentSpec,
    display_space: Option<DisplaySpaceArg>,
) -> Result<DocumentInfo, CommandError> {
    let (document_spec, name) = spec.to_spec()?;
    let document = run_blocking(move || {
        Document::new(document_spec).map_err(|e| CommandError::from(EditorEngineError::from(e)))
    })
    .await?;
    let origin = DocumentOrigin {
        name: Some(name),
        ..DocumentOrigin::default()
    };
    Ok(engine
        .open_async(document, origin, display_space.unwrap_or_default().into())
        .await?)
}

/// Decodes and develops a photo, and makes a document of it (on a
/// blocking thread).
async fn develop_document(
    path: PathBuf,
    stored: Option<serde_json::Value>,
) -> Result<Document, CommandError> {
    run_blocking(move || {
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
    .await
}

/// Saves a document (ADR 0015): writes the layered `.exdoc` file and a
/// flattened 16-bit TIFF next to the source photo, imports the TIFF into
/// the library, and records the link. A later save of the same document
/// replaces the same two files and refreshes the thumbnail.
///
/// With `folder` ("save as"), the save writes a new pair of files in that
/// folder, makes a new library photo and link, and the document then
/// belongs to the new files. The earlier files stay as they are. `folder`
/// must be an absolute path of an existing directory.
///
/// The GPU composite runs on the render thread; the encode and the file
/// writes run on a blocking thread, so frames keep coming.
///
/// Errors: `notFound` (document or source photo), `invalidInput` (the save
/// would overwrite a file that Expose did not create, or a save runs
/// already), `io` (for example, a read-only folder), `unavailable` (no
/// GPU), `database`, `internal`.
#[tauri::command]
pub async fn editor_save<R: Runtime>(
    app: AppHandle<R>,
    engine: State<'_, EditorEngine>,
    document_id: DocumentId,
    folder: Option<String>,
) -> Result<SaveResult, CommandError> {
    let started = Instant::now();
    let snapshot = engine.snapshot_async(document_id).await?;
    let guard = SaveGuard::new(engine.inner(), document_id);
    let revision = snapshot.revision;
    let folder = folder.map(PathBuf::from);
    let outcome = with_catalog(app.clone(), move |state| {
        save_snapshot(state, snapshot, folder.as_deref())
    })
    .await?;
    let info = guard.finish(revision, outcome.files.clone()).await?;
    warm_thumbnail(&app, outcome.files.photo_id);
    tracing::info!(
        document = document_id,
        photo = outcome.files.photo_id.0,
        created = outcome.created,
        ms = started.elapsed().as_millis(),
        "editor document saved"
    );
    Ok(SaveResult {
        photo_id: outcome.files.photo_id,
        document_path: outcome.files.document_path.display().to_string(),
        image_path: outcome.files.image_path.display().to_string(),
        created: outcome.created,
        info,
    })
}

/// Makes the thumbnail of the saved image in the background, so that the
/// library shows the new content soon. The cache key includes the file
/// size and time, so a new file always gives a new thumbnail; the old
/// cache file is removed when the new one is made. A failure is logged.
fn warm_thumbnail<R: Runtime>(app: &AppHandle<R>, photo_id: PhotoId) {
    let app = app.clone();
    // The join handle is not needed: the task only logs its result.
    drop(tauri::async_runtime::spawn_blocking(move || {
        let (Some(service), Some(state)) = (
            app.try_state::<ThumbnailService>(),
            app.try_state::<CatalogState>(),
        ) else {
            return;
        };
        let input = (|| {
            let (path, stored) = {
                let store = state.lock()?;
                let path = store.photo_path(photo_id)?.ok_or_else(|| {
                    CommandError::new(ErrorKind::NotFound, "the photo does not exist")
                })?;
                (path, store.get_develop_settings(photo_id)?)
            };
            ThumbnailInput::from_catalog(photo_id, path, stored.as_ref())
                .map_err(|error| CommandError::internal(error.to_string()))
        })();
        match input {
            Ok(input) => service.request(
                input,
                Box::new(move |result| {
                    if let Err(error) = result {
                        tracing::warn!(photo = photo_id.0, %error, "cannot make the thumbnail of a saved image");
                    }
                }),
            ),
            Err(error) => {
                tracing::warn!(photo = photo_id.0, %error, "cannot make the thumbnail of a saved image");
            }
        }
    }));
}

/// Renders a view of a document and returns the frame body (`EXF1` header
/// and RGBA8 pixels, straight alpha, display color space) as raw binary
/// data. Pixels outside the canvas are transparent.
///
/// `originX`, `originY`: the canvas coordinate at the top-left output
/// pixel. `scale`: output pixels per canvas pixel. `width`, `height`: the
/// output size in pixels, `1..=8192`.
///
/// `showMaskLayerId` (optional, `null` or absent): the number of a layer
/// with a mask. The frame then shows the mask of this layer as a gray
/// image, with the same geometry as the composite.
///
/// If a newer `editor_render` call for the same document arrives before
/// this one starts, this call fails with the kind `superseded` (latest
/// wins). The UI drops it.
///
/// Errors: `notFound` (document or mask layer), `invalidInput` (view, or a
/// layer with no mask), `superseded`,
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
    show_mask_layer_id: Option<u64>,
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
            show_mask: show_mask_layer_id,
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

/// Closes a document and frees its memory. Edits after the last save are
/// lost (the UI asks the user first). Closing an unknown document does
/// nothing.
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
    // Test code: a failed setup is a test failure.
    #![allow(clippy::unwrap_used, clippy::expect_used)]

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

    /// A catalog with `a.jpg` and its saved `a-edit.tif` (linked to
    /// `a-edit.exdoc`) in a temporary folder.
    fn linked_catalog() -> (
        tempfile::TempDir,
        expose_catalog::SqliteStore,
        PhotoId,
        PhotoId,
    ) {
        use expose_catalog::{AddOutcome, EditorOutput, NewPhoto};
        let dir = tempfile::tempdir().unwrap();
        let mut store = expose_catalog::SqliteStore::open_in_memory().unwrap();
        let folder = store.add_folder(dir.path()).unwrap();
        let photo = |name: &str| NewPhoto {
            file_name: name.into(),
            file_size: 1,
            file_modified_ms: 1,
        };
        let ids: Vec<PhotoId> = store
            .add_photos(folder.id, &[photo("a.jpg"), photo("a-edit.tif")])
            .unwrap()
            .into_iter()
            .map(|o| match o {
                AddOutcome::Added(id) | AddOutcome::AlreadyKnown(id) => id,
            })
            .collect();
        store
            .set_editor_output(&EditorOutput {
                photo_id: ids[1],
                document_path: dir.path().join("a-edit.exdoc"),
                source_photo_id: Some(ids[0]),
            })
            .unwrap();
        (dir, store, ids[0], ids[1])
    }

    #[test]
    fn open_of_a_linked_photo_reads_the_layered_file() {
        let (dir, store, source, output) = linked_catalog();
        // A plain photo: decode and develop.
        let plain = open_source(&store, source).unwrap();
        assert_eq!(
            plain,
            OpenSource::Photo {
                path: dir.path().join("a.jpg"),
                stored: None
            }
        );
        // The saved photo: the layered file, with the link as the origin.
        let OpenSource::Linked {
            document_path,
            origin,
        } = open_source(&store, output).unwrap()
        else {
            panic!("the saved photo does not open its layered file");
        };
        assert_eq!(document_path, dir.path().join("a-edit.exdoc"));
        assert_eq!(origin.source_photo_id, Some(source));
        assert_eq!(
            origin.saved,
            Some(SavedFiles {
                photo_id: output,
                document_path: document_path.clone(),
                image_path: dir.path().join("a-edit.tif"),
            })
        );
        // Round trip of the layers through the file.
        let mut doc = build_document(
            expose_core::buffer::WorkingImage::new_filled(3, 2, [0.1, 0.2, 0.3, 1.0]).unwrap(),
        )
        .unwrap();
        let id = doc.layers()[0].id();
        doc.duplicate_layer(id).unwrap();
        doc.set_opacity(id, 0.4).unwrap();
        crate::editor::save::write_atomic(&document_path, |out| {
            expose_editor::write_document(&doc, out)
                .map_err(|e| CommandError::internal(e.to_string()))
        })
        .unwrap();
        assert_eq!(read_linked_document(&document_path).unwrap(), doc);
        // A missing layered file is an error, not the flattened photo.
        std::fs::remove_file(&document_path).unwrap();
        let error = read_linked_document(&document_path).unwrap_err();
        assert_eq!(error.kind, ErrorKind::NotFound);
        // An unknown photo.
        let error = open_source(&store, PhotoId(999)).unwrap_err();
        assert_eq!(error.kind, ErrorKind::NotFound);
    }

    #[test]
    fn stored_settings_parse() {
        let mut settings = DevelopSettings::default();
        settings.tone.exposure = 1.0;
        let value = serde_json::to_value(&settings).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(parse_settings(Some(value)), Ok(settings));
    }
}
