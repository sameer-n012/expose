//! Catalog commands: status, import, folder tree, photo list, and labels.
//!
//! Each command runs [`crate::service`] on a blocking thread. See the
//! service functions for the rules and errors.

use expose_catalog::{FolderNode, ImportReport, Photo, PhotoId, PhotoQuery, StdFileSystem};
use tauri::{AppHandle, Runtime};

use super::with_catalog;
use crate::error::CommandError;
use crate::service::{self, CatalogStatus, PhotoPage};

/// Returns the catalog status: ready (path, schema version, photo count)
/// or unavailable (the startup error).
#[tauri::command]
pub async fn catalog_status<R: Runtime>(app: AppHandle<R>) -> Result<CatalogStatus, CommandError> {
    with_catalog(app, service::catalog_status).await
}

/// Imports the photos in the absolute folder `path` in place.
#[tauri::command]
pub async fn import_folder<R: Runtime>(
    app: AppHandle<R>,
    path: String,
    recursive: bool,
) -> Result<ImportReport, CommandError> {
    with_catalog(app, move |state| {
        service::import_folder(state, &StdFileSystem, &path, recursive)
    })
    .await
}

/// Returns the folder tree with the direct photo count of each folder.
#[tauri::command]
pub async fn folder_tree<R: Runtime>(app: AppHandle<R>) -> Result<Vec<FolderNode>, CommandError> {
    with_catalog(app, service::folder_tree).await
}

/// Returns one page of the photos that match `query`, and the total.
#[tauri::command]
pub async fn list_photos<R: Runtime>(
    app: AppHandle<R>,
    query: PhotoQuery,
) -> Result<PhotoPage, CommandError> {
    with_catalog(app, move |state| service::list_photos(state, &query)).await
}

/// Returns the photo with this id.
#[tauri::command]
pub async fn get_photo<R: Runtime>(app: AppHandle<R>, id: PhotoId) -> Result<Photo, CommandError> {
    with_catalog(app, move |state| service::get_photo(state, id)).await
}

/// Sets the star rating (0 to 5) of the photos.
#[tauri::command]
pub async fn set_rating<R: Runtime>(
    app: AppHandle<R>,
    ids: Vec<PhotoId>,
    rating: u8,
) -> Result<(), CommandError> {
    with_catalog(app, move |state| service::set_rating(state, &ids, rating)).await
}

/// Sets the flag (`none`, `pick`, `reject`) of the photos.
#[tauri::command]
pub async fn set_flag<R: Runtime>(
    app: AppHandle<R>,
    ids: Vec<PhotoId>,
    flag: String,
) -> Result<(), CommandError> {
    with_catalog(app, move |state| service::set_flag(state, &ids, &flag)).await
}

/// Returns the develop settings of the photo (the default settings if
/// none are stored).
#[tauri::command]
pub async fn get_develop_settings<R: Runtime>(
    app: AppHandle<R>,
    photo_id: PhotoId,
) -> Result<serde_json::Value, CommandError> {
    with_catalog(app, move |state| {
        service::get_develop_settings(state, photo_id)
    })
    .await
}

/// Validates and stores the develop settings of the photo. Returns the
/// stored settings.
#[tauri::command]
pub async fn set_develop_settings<R: Runtime>(
    app: AppHandle<R>,
    photo_id: PhotoId,
    settings: serde_json::Value,
) -> Result<serde_json::Value, CommandError> {
    with_catalog(app, move |state| {
        service::set_develop_settings(state, photo_id, settings)
    })
    .await
}

/// Sets the color label of the photos.
#[tauri::command]
pub async fn set_color_label<R: Runtime>(
    app: AppHandle<R>,
    ids: Vec<PhotoId>,
    label: String,
) -> Result<(), CommandError> {
    with_catalog(app, move |state| {
        service::set_color_label(state, &ids, &label)
    })
    .await
}
