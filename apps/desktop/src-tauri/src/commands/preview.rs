//! Develop preview commands: open a session, render frames, close.
//!
//! The frame transport is a raw binary IPC response: `preview_render`
//! returns `tauri::ipc::Response`, and the UI gets an `ArrayBuffer` with
//! the frame header and RGBA8 pixels (see [`crate::preview::frame`]).
//!
//! `preview_open_test_pattern` and `preview_bench_channel` exist for the
//! transport benchmark of the UI (`?bench=transport`).

use std::path::PathBuf;
use std::time::Instant;

use expose_catalog::{CatalogStore, PhotoId};
use expose_core::develop::DevelopSettings;
use expose_develop::{DisplaySpace, SourceInfo};
use serde::{Deserialize, Serialize};
use tauri::ipc::{Channel, InvokeResponseBody, Response};
use tauri::{AppHandle, Manager, Runtime, State};

use super::{run_blocking, with_catalog};
use crate::error::{CommandError, ErrorKind};
use crate::preview::{
    DEFAULT_LONG_EDGE, FrameFormat, OpenRequest, PreviewEngine, PreviewError, RenderRequest,
    SessionId, SessionInfo, test_pattern,
};

/// The display color space argument of the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplaySpaceArg {
    /// sRGB.
    #[default]
    Srgb,
    /// Display P3.
    DisplayP3,
}

impl From<DisplaySpaceArg> for DisplaySpace {
    fn from(value: DisplaySpaceArg) -> Self {
        match value {
            DisplaySpaceArg::Srgb => Self::Srgb,
            DisplaySpaceArg::DisplayP3 => Self::DisplayP3,
        }
    }
}

/// Opens a preview session for a photo: decodes the file, downsamples it to
/// `max_long_edge` (default 2560), and uploads it to the GPU once.
///
/// Errors: `notFound` (photo or file), `io`/`internal` (decode),
/// `unavailable` (no GPU), `invalidInput` (size).
#[tauri::command]
pub async fn preview_open<R: Runtime>(
    app: AppHandle<R>,
    engine: State<'_, PreviewEngine>,
    photo_id: PhotoId,
    max_long_edge: Option<u32>,
    display_space: Option<DisplaySpaceArg>,
) -> Result<SessionInfo, CommandError> {
    // Hold the catalog lock only to read the path.
    let path: PathBuf = with_catalog(app, move |state| {
        let store = state.lock()?;
        store
            .photo_path(photo_id)?
            .ok_or_else(|| CommandError::new(ErrorKind::NotFound, "the photo does not exist"))
    })
    .await?;
    let started = Instant::now();
    let decoded = run_blocking(move || {
        expose_raw::decode_path(&path).map_err(|error| {
            let kind = if path.exists() {
                ErrorKind::Internal
            } else {
                ErrorKind::NotFound
            };
            CommandError::new(kind, format!("cannot decode {}: {error}", path.display()))
        })
    })
    .await?;
    tracing::debug!(
        photo = photo_id.0,
        ms = started.elapsed().as_millis(),
        "preview source decoded"
    );
    // The decoders apply the as-shot white balance of RAW files, so the
    // decoded image is a rendered source (as for thumbnails).
    let request = OpenRequest {
        image: decoded.image,
        source: SourceInfo::RENDERED,
        max_long_edge: max_long_edge.unwrap_or(DEFAULT_LONG_EDGE),
        space: display_space.unwrap_or_default().into(),
    };
    Ok(engine.open_async(request).await?)
}

/// Opens a preview session with a synthetic test image of
/// `width` x `height` pixels (for the transport benchmark).
#[tauri::command]
pub async fn preview_open_test_pattern(
    engine: State<'_, PreviewEngine>,
    width: u32,
    height: u32,
    max_long_edge: Option<u32>,
    display_space: Option<DisplaySpaceArg>,
) -> Result<SessionInfo, CommandError> {
    const MAX_SIDE: u32 = 12_000;
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(CommandError::invalid_input(format!(
            "the test image size {width}x{height} is not in 1..={MAX_SIDE}"
        )));
    }
    let image = run_blocking(move || {
        test_pattern(width, height).map_err(|error| CommandError::internal(error.to_string()))
    })
    .await?;
    let request = OpenRequest {
        image,
        source: SourceInfo::RENDERED,
        max_long_edge: max_long_edge.unwrap_or(DEFAULT_LONG_EDGE),
        space: display_space.unwrap_or_default().into(),
    };
    Ok(engine.open_async(request).await?)
}

/// Renders one frame of a session with `settings` and returns the frame
/// body (header and RGBA8 pixels) as raw binary data.
///
/// `long_edge` is the target long edge of the frame in pixels (for
/// example, the device-pixel size of the canvas). The engine clamps it to
/// the session size and to a minimum; `null` gives the session size. The
/// frame header has the real size.
///
/// If a newer `preview_render` call arrives before this one starts, this
/// call fails with the kind `superseded` (latest wins). The UI drops it.
///
/// Errors: `invalidInput` (settings, or `long_edge` 0 or above 8192),
/// `notFound` (session), `superseded`, `internal`.
#[tauri::command]
pub async fn preview_render(
    engine: State<'_, PreviewEngine>,
    session: SessionId,
    settings: DevelopSettings,
    long_edge: Option<u32>,
) -> Result<Response, CommandError> {
    let frame = engine
        .render_async(RenderRequest {
            session,
            settings,
            format: FrameFormat::Rgba8,
            long_edge,
        })
        .await?;
    Ok(Response::new(frame.body))
}

/// Closes a session and frees its GPU memory. Closing an unknown session
/// does nothing.
///
/// This command only queues the close, so it can run on the main thread.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri passes managed state by value"
)]
pub fn preview_close(engine: State<'_, PreviewEngine>, session: SessionId) {
    engine.close(session);
}

/// The result of [`preview_bench_channel`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelBenchReport {
    /// The number of frames sent.
    pub frames: u32,
    /// The time from the first render to the last send, in milliseconds.
    pub elapsed_ms: f64,
}

/// Transport benchmark: renders `frames` frames of a session (exposure
/// steps of 0.02 EV) and pushes each frame body into `channel` as raw
/// binary data, with no request from the UI between frames.
#[tauri::command]
pub async fn preview_bench_channel<R: Runtime>(
    app: AppHandle<R>,
    session: SessionId,
    frames: u32,
    channel: Channel<InvokeResponseBody>,
) -> Result<ChannelBenchReport, CommandError> {
    const MAX_FRAMES: u32 = 1000;
    if frames == 0 || frames > MAX_FRAMES {
        return Err(CommandError::invalid_input(format!(
            "the frame count {frames} is not in 1..={MAX_FRAMES}"
        )));
    }
    run_blocking(move || {
        let engine = app
            .try_state::<PreviewEngine>()
            .ok_or_else(|| CommandError::internal("the preview engine is not registered"))?;
        let started = Instant::now();
        for i in 0..frames {
            let mut settings = DevelopSettings::default();
            // `i` is at most 1000, which `f32` represents exactly.
            #[allow(clippy::cast_precision_loss)]
            let ev = (i % 100) as f32 * 0.02;
            settings.tone.exposure = ev;
            let frame = engine.render_blocking(RenderRequest {
                session,
                settings,
                format: FrameFormat::Rgba8,
                long_edge: None,
            });
            let frame = match frame {
                Ok(frame) => frame,
                // Another caller can take the render slot; skip that frame.
                Err(PreviewError::Superseded) => continue,
                Err(error) => return Err(error.into()),
            };
            channel
                .send(InvokeResponseBody::Raw(frame.body))
                .map_err(|error| CommandError::internal(format!("channel send failed: {error}")))?;
        }
        Ok(ChannelBenchReport {
            frames,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_space_arg_parses_camel_case() {
        let parse = |text: &str| serde_json::from_str::<DisplaySpaceArg>(text).unwrap();
        assert_eq!(DisplaySpace::from(parse("\"srgb\"")), DisplaySpace::Srgb);
        assert_eq!(
            DisplaySpace::from(parse("\"displayP3\"")),
            DisplaySpace::DisplayP3
        );
        assert!(serde_json::from_str::<DisplaySpaceArg>("\"p3\"").is_err());
    }
}
