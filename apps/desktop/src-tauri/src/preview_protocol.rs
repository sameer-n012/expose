//! The `preview` URI scheme: serves preview frames to the web view with
//! `fetch()` or `<img>`. The production transport is the `preview_render`
//! command; this scheme is the alternative transport, and serves the
//! encoded-frame candidates of the transport benchmark.
//!
//! # Request
//!
//! - macOS and Linux: `POST preview://localhost/<session>?format=<f>`
//! - Windows: `POST http://preview.localhost/<session>?format=<f>`
//!
//! `<f>` is `rgba8` (default), `jpeg`, or `png`. An optional `long=<n>`
//! sets the target long edge of the frame (see
//! [`crate::preview::effective_long_edge`]). The body is the develop
//! settings as JSON. Send it with `Content-Type: text/plain`, so that the
//! request is a CORS "simple request" with no preflight. A `GET` with no
//! body renders the default settings (for `<img src>`).
//!
//! # Responses
//!
//! - `200`: the frame body. `rgba8`: `application/octet-stream`, header
//!   and pixels (see [`crate::preview::frame`]). `jpeg`/`png`: the file.
//!   The `X-Expose-Frame` header has `width,height,seq,queue_us,render_us`.
//! - `400`: the path is not a session id, the format is unknown, or the
//!   settings are not valid.
//! - `404`: the session is not open.
//! - `409`: a newer request replaced this request (latest wins).
//! - `503`: no GPU, or the engine is not registered.
//! - `500`: the render failed. The body is a short text.
//!
//! All responses allow any origin (`Access-Control-Allow-Origin: *`): the
//! UI origin (`tauri://localhost` or `http://localhost:3100` in dev) is
//! not the scheme origin.

use expose_core::develop::DevelopSettings;
use tauri::http::{HeaderValue, Method, Request, Response, StatusCode, header};
use tauri::{Manager, Runtime, UriSchemeContext, UriSchemeResponder};

use crate::preview::{Frame, FrameFormat, PreviewEngine, PreviewError, RenderRequest, SessionId};

/// The name of the URI scheme.
pub const SCHEME: &str = "preview";

/// The name of the frame information header.
pub const FRAME_HEADER_NAME: &str = "x-expose-frame";

/// A parsed request.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameQuery {
    /// The session.
    pub session: SessionId,
    /// The body form.
    pub format: FrameFormat,
    /// The target long edge (`long=<n>`), or `None`.
    pub long_edge: Option<u32>,
}

/// Handles one request. The render runs on the preview thread; the
/// responder is called from there.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri requires this handler signature"
)]
pub fn handle<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    if request.method() == Method::OPTIONS {
        responder.respond(preflight());
        return;
    }
    let query = match parse_request(request.uri().path(), request.uri().query()) {
        Ok(query) => query,
        Err(message) => {
            responder.respond(text(StatusCode::BAD_REQUEST, &message));
            return;
        }
    };
    let settings = match parse_settings(request.body()) {
        Ok(settings) => settings,
        Err(message) => {
            responder.respond(text(StatusCode::BAD_REQUEST, &message));
            return;
        }
    };
    let Some(engine) = ctx.app_handle().try_state::<PreviewEngine>() else {
        responder.respond(text(
            StatusCode::SERVICE_UNAVAILABLE,
            "the preview engine is not available",
        ));
        return;
    };
    engine.render(
        RenderRequest {
            session: query.session,
            settings,
            format: query.format,
            long_edge: query.long_edge,
        },
        Box::new(move |result| {
            let response = match result {
                Ok(frame) => frame_response(frame),
                Err(error) => error_response(&error),
            };
            responder.respond(response);
        }),
    );
}

/// Parses the session id from the path (`/<id>`), and the format and the
/// target long edge from the query (`format=<f>`, `long=<n>`; other keys
/// are ignored).
///
/// # Errors
///
/// A message for the `400` response.
pub fn parse_request(path: &str, query: Option<&str>) -> Result<FrameQuery, String> {
    let text = path.strip_prefix('/').unwrap_or(path);
    let text = text.strip_suffix('/').unwrap_or(text);
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err("the path is not a session id".into());
    }
    let session = text
        .parse::<SessionId>()
        .map_err(|_| "the session id is too large".to_owned())?;
    let mut format = FrameFormat::Rgba8;
    let mut long_edge = None;
    for pair in query.unwrap_or_default().split('&') {
        if let Some(value) = pair.strip_prefix("format=") {
            format = FrameFormat::parse(value)
                .ok_or_else(|| format!("unknown frame format `{value}`"))?;
        } else if let Some(value) = pair.strip_prefix("long=") {
            let edge = value
                .parse::<u32>()
                .map_err(|_| format!("the long edge `{value}` is not a number"))?;
            long_edge = Some(edge);
        }
    }
    Ok(FrameQuery {
        session,
        format,
        long_edge,
    })
}

/// Parses the develop settings from the request body. An empty body gives
/// the default settings.
///
/// # Errors
///
/// A message for the `400` response.
pub fn parse_settings(body: &[u8]) -> Result<DevelopSettings, String> {
    if body.is_empty() {
        return Ok(DevelopSettings::default());
    }
    serde_json::from_slice(body).map_err(|error| format!("the settings are not valid: {error}"))
}

/// Returns the HTTP status of an engine error.
pub fn status_of(error: &PreviewError) -> StatusCode {
    match error {
        PreviewError::SessionNotFound(_) => StatusCode::NOT_FOUND,
        PreviewError::Superseded => StatusCode::CONFLICT,
        PreviewError::InvalidSize(_)
        | PreviewError::Develop(expose_develop::DevelopError::Settings(_)) => {
            StatusCode::BAD_REQUEST
        }
        PreviewError::GpuUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        PreviewError::Develop(_)
        | PreviewError::Encode(_)
        | PreviewError::Internal(_)
        | PreviewError::Stopped(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn frame_response(frame: Frame) -> Response<Vec<u8>> {
    let info = frame.header.to_header_value();
    let mut response = Response::new(frame.body);
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(frame.format.mime()),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // The value is ASCII digits and commas, so it is always valid.
    if let Ok(value) = HeaderValue::from_str(&info) {
        headers.insert(FRAME_HEADER_NAME, value);
    }
    allow_any_origin(response)
}

fn error_response(error: &PreviewError) -> Response<Vec<u8>> {
    let status = status_of(error);
    if status.is_server_error() {
        tracing::error!(%error, "preview frame request failed");
    }
    text(status, &error.to_string())
}

fn preflight() -> Response<Vec<u8>> {
    let mut response = Response::new(Vec::new());
    *response.status_mut() = StatusCode::NO_CONTENT;
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("content-type"),
    );
    allow_any_origin(response)
}

fn text(status: StatusCode, message: &str) -> Response<Vec<u8>> {
    let mut response = Response::new(message.as_bytes().to_vec());
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    allow_any_origin(response)
}

fn allow_any_origin(mut response: Response<Vec<u8>>) -> Response<Vec<u8>> {
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static(FRAME_HEADER_NAME),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::{FrameHeader, FrameTimings};

    #[test]
    fn parses_session_and_format() {
        let ok = |path, query| parse_request(path, query).unwrap();
        assert_eq!(
            ok("/7", None),
            FrameQuery {
                session: 7,
                format: FrameFormat::Rgba8,
                long_edge: None,
            }
        );
        assert_eq!(ok("/7/", Some("format=jpeg")).format, FrameFormat::Jpeg);
        assert_eq!(ok("/7", Some("t=5&format=png")).format, FrameFormat::Png);
        assert_eq!(ok("/7", Some("long=1200")).long_edge, Some(1200));
        // The engine validates the range; the parser accepts any u32.
        assert_eq!(ok("/7", Some("long=0")).long_edge, Some(0));
        assert!(parse_request("/7", Some("long=")).is_err());
        assert!(parse_request("/7", Some("long=-5")).is_err());
        assert!(parse_request("/7", Some("long=12px")).is_err());
        assert!(parse_request("/", None).is_err());
        assert!(parse_request("/x1", None).is_err());
        assert!(parse_request("/99999999999999999999999", None).is_err());
        assert!(parse_request("/1", Some("format=gif")).is_err());
    }

    #[test]
    fn parses_settings_body() {
        assert_eq!(parse_settings(b"").unwrap(), DevelopSettings::default());
        let settings = parse_settings(br#"{"tone":{"exposure":1.5}}"#).unwrap();
        assert!((settings.tone.exposure - 1.5).abs() < f32::EPSILON);
        assert!(parse_settings(b"{").is_err());
    }

    #[test]
    fn maps_errors_to_status() {
        assert_eq!(
            status_of(&PreviewError::SessionNotFound(1)),
            StatusCode::NOT_FOUND
        );
        assert_eq!(status_of(&PreviewError::Superseded), StatusCode::CONFLICT);
        assert_eq!(
            status_of(&PreviewError::Stopped("x".into())),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn frame_response_has_type_info_and_cors() {
        let frame = Frame {
            format: FrameFormat::Jpeg,
            header: FrameHeader {
                width: 4,
                height: 3,
                seq: 9,
                timings: FrameTimings::default(),
            },
            body: vec![0xFF, 0xD8],
        };
        let response = frame_response(frame);
        let headers = response.headers();
        assert_eq!(headers[header::CONTENT_TYPE], "image/jpeg");
        assert_eq!(headers[FRAME_HEADER_NAME], "4,3,9,0,0");
        assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        assert_eq!(
            headers[header::ACCESS_CONTROL_EXPOSE_HEADERS],
            FRAME_HEADER_NAME
        );
        assert_eq!(preflight().status(), StatusCode::NO_CONTENT);
        assert_eq!(
            text(StatusCode::CONFLICT, "x").headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            "*"
        );
    }
}
