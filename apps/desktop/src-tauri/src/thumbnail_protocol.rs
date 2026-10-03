//! The `thumb` URI scheme: serves the thumbnails of the
//! [`ThumbnailService`] to the web view.
//!
//! # URL form
//!
//! - macOS and Linux: `thumb://localhost/<photo id>`
//! - Windows: `http://thumb.localhost/<photo id>`
//!
//! A query string (for example `?v=<file modified ms>`) is ignored. The UI
//! adds it so that the web view does not use an old image after a file
//! change.
//!
//! # Responses
//!
//! - `200`: `image/jpeg` bytes, with an `ETag` (the cache key) and
//!   `Cache-Control: no-cache`, so that the web view asks again and gets
//!   `304` while the thumbnail is current.
//! - `400`: the path is not a photo id.
//! - `404`: the photo is not in the catalog, or its file does not exist.
//! - `503`: the catalog or the thumbnail cache is not available.
//! - `500`: the decode or another step failed. The body is a short text.

use expose_catalog::{CatalogStore, PhotoId};
use tauri::http::{HeaderValue, Request, Response, StatusCode, header};
use tauri::{AppHandle, Manager, Runtime, UriSchemeContext, UriSchemeResponder};

use crate::error::ErrorKind;
use crate::state::CatalogState;
use crate::thumbnails::{ThumbnailError, ThumbnailInput, ThumbnailResult, ThumbnailService};

/// The name of the URI scheme.
pub const SCHEME: &str = "thumb";

/// Handles one request of the `thumb` scheme. The work runs on a blocking
/// thread and on the thumbnail workers, never on the calling thread.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri requires this handler signature"
)]
pub fn handle<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let Some(id) = parse_photo_id(request.uri().path()) else {
        responder.respond(text(StatusCode::BAD_REQUEST, "the path is not a photo id"));
        return;
    };
    let if_none_match = request
        .headers()
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let app = ctx.app_handle().clone();
    // The join handle is not needed: the task always answers `responder`.
    drop(tauri::async_runtime::spawn_blocking(move || {
        serve(&app, id, if_none_match.as_deref(), responder);
    }));
}

/// Reads the photo data, then answers from the cache or queues the work.
fn serve<R: Runtime>(
    app: &AppHandle<R>,
    id: PhotoId,
    if_none_match: Option<&str>,
    responder: UriSchemeResponder,
) {
    let input = match lookup(app, id) {
        Ok(input) => input,
        Err(error) => {
            responder.respond(error_response(id, &error));
            return;
        }
    };
    let etag = format!("\"{:016x}\"", input.key);
    if if_none_match == Some(etag.as_str()) {
        responder.respond(with_etag(empty(StatusCode::NOT_MODIFIED), &etag));
        return;
    }
    let Some(service) = app.try_state::<ThumbnailService>() else {
        responder.respond(text(
            StatusCode::SERVICE_UNAVAILABLE,
            "the thumbnail cache is not available",
        ));
        return;
    };
    service.request(
        input,
        Box::new(move |result: ThumbnailResult| {
            let response = match result {
                Ok(bytes) => with_etag(jpeg(bytes.to_vec()), &etag),
                Err(error) => error_response(id, &error),
            };
            responder.respond(response);
        }),
    );
}

/// Reads the file path and the develop settings of a photo. The catalog
/// lock is held only for these two reads.
fn lookup<R: Runtime>(app: &AppHandle<R>, id: PhotoId) -> Result<ThumbnailInput, ThumbnailError> {
    let state = app
        .try_state::<CatalogState>()
        .ok_or_else(|| ThumbnailError::Worker("the catalog state is not registered".into()))?;
    let (path, stored) = {
        let store = state.lock()?;
        let path = store
            .photo_path(id)
            .map_err(crate::error::CommandError::from)?
            .ok_or(ThumbnailError::PhotoNotFound(id.0))?;
        let stored = store
            .get_develop_settings(id)
            .map_err(crate::error::CommandError::from)?;
        (path, stored)
    };
    ThumbnailInput::from_catalog(id, path, stored.as_ref())
}

/// Parses the photo id from a URL path such as `/42`.
pub fn parse_photo_id(path: &str) -> Option<PhotoId> {
    let text = path.strip_prefix('/').unwrap_or(path);
    let text = text.strip_suffix('/').unwrap_or(text);
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<i64>().ok().map(PhotoId)
}

/// Returns the HTTP status of an error.
pub fn status_of(error: &ThumbnailError) -> StatusCode {
    match error {
        ThumbnailError::PhotoNotFound(_) | ThumbnailError::FileNotFound { .. } => {
            StatusCode::NOT_FOUND
        }
        ThumbnailError::Catalog(e) => match e.kind {
            ErrorKind::NotFound => StatusCode::NOT_FOUND,
            ErrorKind::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            ErrorKind::InvalidInput
            | ErrorKind::Database
            | ErrorKind::Io
            | ErrorKind::Superseded
            | ErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        },
        ThumbnailError::Io { .. }
        | ThumbnailError::Decode(_)
        | ThumbnailError::Develop(_)
        | ThumbnailError::Core(_)
        | ThumbnailError::Encode(_)
        | ThumbnailError::Worker(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Logs an error and makes its response.
fn error_response(id: PhotoId, error: &ThumbnailError) -> Response<Vec<u8>> {
    let status = status_of(error);
    if status == StatusCode::NOT_FOUND {
        tracing::debug!(photo = id.0, %error, "no thumbnail");
    } else {
        tracing::error!(photo = id.0, %error, "thumbnail request failed");
    }
    text(status, &error.to_string())
}

fn empty(status: StatusCode) -> Response<Vec<u8>> {
    let mut response = Response::new(Vec::new());
    *response.status_mut() = status;
    response
}

fn text(status: StatusCode, message: &str) -> Response<Vec<u8>> {
    let mut response = Response::new(message.as_bytes().to_vec());
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

fn jpeg(bytes: Vec<u8>) -> Response<Vec<u8>> {
    let mut response = Response::new(bytes);
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

fn with_etag(mut response: Response<Vec<u8>>, etag: &str) -> Response<Vec<u8>> {
    // The tag is quoted hexadecimal text, so it is always a valid value.
    if let Ok(value) = HeaderValue::from_str(etag) {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::error::CommandError;

    #[test]
    fn parses_photo_ids() {
        assert_eq!(parse_photo_id("/42"), Some(PhotoId(42)));
        assert_eq!(parse_photo_id("42/"), Some(PhotoId(42)));
        assert_eq!(parse_photo_id("/"), None);
        assert_eq!(parse_photo_id("/-1"), None);
        assert_eq!(parse_photo_id("/4a"), None);
        assert_eq!(parse_photo_id("/99999999999999999999"), None);
    }

    #[test]
    fn maps_errors_to_status() {
        assert_eq!(
            status_of(&ThumbnailError::PhotoNotFound(1)),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status_of(&ThumbnailError::FileNotFound {
                path: PathBuf::from("/x")
            }),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status_of(&ThumbnailError::Catalog(CommandError::new(
                ErrorKind::Unavailable,
                "x"
            ))),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            status_of(&ThumbnailError::Worker("x".into())),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn responses_have_types() {
        let r = with_etag(jpeg(vec![1, 2]), "\"00ff\"");
        assert_eq!(r.headers()[header::CONTENT_TYPE], "image/jpeg");
        assert_eq!(r.headers()[header::ETAG], "\"00ff\"");
        let r = text(StatusCode::NOT_FOUND, "no");
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        assert_eq!(r.body(), b"no");
    }
}
