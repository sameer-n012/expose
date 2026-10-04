//! The preview engine: one render thread that owns the GPU context and the
//! open preview sessions.
//!
//! # Threads
//!
//! The engine starts one thread at application start. The thread makes the
//! GPU context at the first session open (lazy), so the application start
//! does not wait for the GPU. All GPU work runs on this thread, never on
//! the main thread or an async worker thread.
//!
//! # Coalescing (latest wins)
//!
//! The engine keeps at most one waiting frame request. When a new request
//! arrives before the thread starts the waiting one, the new request
//! replaces it, and the old request gets [`PreviewError::Superseded`].
//! Thus a fast slider never builds a queue of stale frames: the next frame
//! after the current one is always the newest request.
//!
//! Open and close jobs do not coalesce. They run in order, before a waiting
//! frame request.
//!
//! # Pipelining
//!
//! The UI client keeps up to two frame requests in flight. While the UI
//! receives frame N, this thread renders frame N + 1 (the waiting
//! request). Thus render and transfer overlap. The single waiting slot is
//! the backpressure: a third request supersedes the waiting one, so the
//! engine never holds more than one running and one waiting frame.
//!
//! # Sessions
//!
//! A session is one photo: the downsampled source in CPU memory (the
//! "base", at the long edge of the open), and one GPU view of it (source,
//! develop output, and display encoder). The engine keeps at most
//! [`MAX_SESSIONS`] sessions; an open beyond this closes the session that
//! was used least recently. One 2560 px session uses about 160 MB of GPU
//! memory (two `rgba32float` images and one `rgba8unorm` image) and 70 MB
//! of CPU memory (the base).
//!
//! # Frame size
//!
//! A frame request can ask for a smaller long edge than the base (for
//! example, the device-pixel size of the canvas). The engine clamps the
//! value to [`MIN_LONG_EDGE`]..=base long edge (see
//! [`effective_long_edge`]). When the clamped size is not the size of the
//! GPU view, the engine makes a new view from the base. It frees the old
//! view first, so the GPU memory does not increase.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use expose_core::buffer::WorkingImage;
use expose_core::develop::DevelopSettings;
use expose_develop::{
    DevelopError, DisplayEncoder, DisplaySpace, Pipeline, PreviewRenderer, SourceInfo,
    downsample_area, preview_size,
};
use expose_gpu::{GpuContext, GpuContextOptions, GpuError};
use expose_raw::{BitDepth, EncodeError, EncodeOptions, encode};
use serde::{Deserialize, Serialize};

use super::frame::{FrameHeader, FrameTimings, rgba8_body_len};
use crate::error::{CommandError, ErrorKind};

/// The default long edge of a preview, in pixels.
pub const DEFAULT_LONG_EDGE: u32 = 2560;

/// The largest preview long edge that a caller can request.
pub const MAX_LONG_EDGE: u32 = 8192;

/// The smallest long edge of a frame. A smaller requested value is
/// increased to this value. Reason: very small frames give no useful
/// preview, and a resize of the window to near zero must not make a 1 px
/// GPU view.
pub const MIN_LONG_EDGE: u32 = 64;

/// The maximum number of open sessions.
pub const MAX_SESSIONS: usize = 2;

/// The JPEG quality of encoded frames (benchmark candidate only).
pub const FRAME_JPEG_QUALITY: u8 = 85;

/// The id of a preview session.
pub type SessionId = u64;

/// The result of a session open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    /// The session id.
    pub session: SessionId,
    /// The preview width in pixels.
    pub width: u32,
    /// The preview height in pixels.
    pub height: u32,
}

/// The form of a frame body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FrameFormat {
    /// The frame header and raw RGBA8 pixels (see [`super::frame`]). The
    /// production format.
    #[default]
    Rgba8,
    /// A baseline JPEG file in sRGB (benchmark candidate).
    Jpeg,
    /// An 8-bit RGB PNG file in sRGB (benchmark candidate).
    Png,
}

impl FrameFormat {
    /// The MIME type of the body.
    pub const fn mime(self) -> &'static str {
        match self {
            Self::Rgba8 => "application/octet-stream",
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
        }
    }

    /// Parses the `format` query value of the `preview` URI scheme.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "rgba8" => Some(Self::Rgba8),
            "jpeg" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            _ => None,
        }
    }
}

/// One rendered frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// The body form.
    pub format: FrameFormat,
    /// The frame header data. For [`FrameFormat::Rgba8`], the body starts
    /// with the same data.
    pub header: FrameHeader,
    /// The body: header and pixels, or an encoded file.
    pub body: Vec<u8>,
}

/// An error of the preview engine.
#[derive(Debug, thiserror::Error)]
pub enum PreviewError {
    /// A newer frame request replaced this request before it started.
    /// This is not a failure; the UI drops the request.
    #[error("a newer frame request replaced this request")]
    Superseded,
    /// The session is not open (closed, or closed by a newer open).
    #[error("preview session {0} is not open")]
    SessionNotFound(SessionId),
    /// The requested preview size is not valid.
    #[error("the preview long edge {0} is not in 1..={MAX_LONG_EDGE}")]
    InvalidSize(u32),
    /// The GPU context cannot be made.
    #[error("the GPU is not available: {0}")]
    GpuUnavailable(#[source] GpuError),
    /// The develop pipeline or a GPU step failed.
    #[error("cannot develop the preview: {0}")]
    Develop(#[from] DevelopError),
    /// The file encode of a frame failed.
    #[error("cannot encode the frame: {0}")]
    Encode(#[from] EncodeError),
    /// The frame does not fit the header (a bug).
    #[error("internal error: {0}")]
    Internal(&'static str),
    /// The render thread stopped or panicked.
    #[error("the preview engine stopped: {0}")]
    Stopped(String),
}

impl From<PreviewError> for CommandError {
    fn from(error: PreviewError) -> Self {
        let kind = match &error {
            PreviewError::Superseded => ErrorKind::Superseded,
            PreviewError::SessionNotFound(_) => ErrorKind::NotFound,
            PreviewError::InvalidSize(_) | PreviewError::Develop(DevelopError::Settings(_)) => {
                ErrorKind::InvalidInput
            }
            PreviewError::GpuUnavailable(_) => ErrorKind::Unavailable,
            PreviewError::Develop(_)
            | PreviewError::Encode(_)
            | PreviewError::Internal(_)
            | PreviewError::Stopped(_) => ErrorKind::Internal,
        };
        Self::new(kind, error.to_string())
    }
}

/// A function that receives the result of a job. It runs on the render
/// thread, so it must return fast.
pub type Reply<T> = Box<dyn FnOnce(Result<T, PreviewError>) + Send + 'static>;

/// The input of a session open.
pub struct OpenRequest {
    /// The full source image (linear Rec. 2020, straight alpha).
    pub image: WorkingImage,
    /// The source information for white balance.
    pub source: SourceInfo,
    /// The preview long edge. Smaller images are not enlarged.
    pub max_long_edge: u32,
    /// The display color space of the frames.
    pub space: DisplaySpace,
}

impl std::fmt::Debug for OpenRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenRequest")
            .field("width", &self.image.width())
            .field("height", &self.image.height())
            .field("max_long_edge", &self.max_long_edge)
            .field("space", &self.space)
            .finish_non_exhaustive()
    }
}

/// One frame request.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderRequest {
    /// The session.
    pub session: SessionId,
    /// The develop settings of this frame.
    pub settings: DevelopSettings,
    /// The body form.
    pub format: FrameFormat,
    /// The requested long edge of the frame in pixels. `None` gives the
    /// base size of the session. See [`effective_long_edge`].
    pub long_edge: Option<u32>,
}

/// Returns the long edge of a frame for a session with a base long edge
/// of `base_long_edge`, when the caller requests `requested`.
///
/// - `None` gives `base_long_edge`.
/// - The value is clamped to `MIN_LONG_EDGE..=base_long_edge`. Frames are
///   never larger than the base (no upscale). If the base is smaller than
///   [`MIN_LONG_EDGE`], the base size applies.
///
/// # Errors
///
/// [`PreviewError::InvalidSize`] if `requested` is 0 or larger than
/// [`MAX_LONG_EDGE`]. These values are a bug in the caller, so the engine
/// does not clamp them.
pub fn effective_long_edge(
    base_long_edge: u32,
    requested: Option<u32>,
) -> Result<u32, PreviewError> {
    let Some(requested) = requested else {
        return Ok(base_long_edge);
    };
    if requested == 0 || requested > MAX_LONG_EDGE {
        return Err(PreviewError::InvalidSize(requested));
    }
    Ok(requested.max(MIN_LONG_EDGE).min(base_long_edge))
}

enum Job {
    Open(OpenRequest, Reply<SessionInfo>),
    Close(SessionId),
}

struct RenderJob {
    request: RenderRequest,
    queued_at: Instant,
    reply: Reply<Frame>,
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    render: Option<RenderJob>,
    stop: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
}

impl Shared {
    /// Locks the queue. The queue is a plain container: a panic while the
    /// lock is held cannot leave it half-changed, so a poisoned lock is
    /// recovered.
    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The preview engine. Keep one in the Tauri managed state.
pub struct PreviewEngine {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for PreviewEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewEngine").finish_non_exhaustive()
    }
}

impl PreviewEngine {
    /// Starts the render thread. The GPU context is made later, at the
    /// first open.
    ///
    /// # Errors
    ///
    /// Returns the error of the operating system if the thread does not
    /// start.
    pub fn start() -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
        });
        let worker = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("expose-preview".into())
            .spawn(move || render_loop(&worker))?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    /// Queues a session open. `reply` gets the session size.
    pub fn open(&self, request: OpenRequest, reply: Reply<SessionInfo>) {
        self.push_job(Job::Open(request, reply));
    }

    /// Queues a session close. A close of an unknown session does nothing.
    pub fn close(&self, session: SessionId) {
        self.push_job(Job::Close(session));
    }

    /// Queues a frame request. A waiting older request gets
    /// [`PreviewError::Superseded`] (see the module docs).
    pub fn render(&self, request: RenderRequest, reply: Reply<Frame>) {
        let job = RenderJob {
            request,
            queued_at: Instant::now(),
            reply,
        };
        let replaced = {
            let mut queue = self.shared.lock();
            if queue.stop {
                drop(queue);
                (job.reply)(Err(stopped()));
                return;
            }
            queue.render.replace(job)
        };
        self.shared.wake.notify_one();
        // Reply outside the lock: the reply can be slow (for example, an
        // HTTP response).
        if let Some(old) = replaced {
            (old.reply)(Err(PreviewError::Superseded));
        }
    }

    /// Opens a session and waits for the result.
    ///
    /// # Errors
    ///
    /// The open errors (see [`PreviewError`]).
    pub async fn open_async(&self, request: OpenRequest) -> Result<SessionInfo, PreviewError> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.open(request, reply_to(sender));
        receive(receiver).await
    }

    /// Requests a frame and waits for it.
    ///
    /// # Errors
    ///
    /// [`PreviewError::Superseded`] when a newer request replaced this one,
    /// and the render errors.
    pub async fn render_async(&self, request: RenderRequest) -> Result<Frame, PreviewError> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.render(request, reply_to(sender));
        receive(receiver).await
    }

    /// Requests a frame and blocks the calling thread until it is ready.
    /// Do not call this on the main thread or an async worker thread.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::render_async`].
    pub fn render_blocking(&self, request: RenderRequest) -> Result<Frame, PreviewError> {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.render(
            request,
            Box::new(move |result| {
                // The receiver waits below; a send error means it is gone.
                drop(sender.send(result));
            }),
        );
        receiver.recv().unwrap_or_else(|_| Err(stopped()))
    }

    fn push_job(&self, job: Job) {
        let mut queue = self.shared.lock();
        if queue.stop {
            drop(queue);
            fail_job(job, &stopped());
            return;
        }
        queue.jobs.push_back(job);
        drop(queue);
        self.shared.wake.notify_one();
    }
}

impl Drop for PreviewEngine {
    fn drop(&mut self) {
        self.shared.lock().stop = true;
        self.shared.wake.notify_all();
        if let Some(thread) = self.thread.take() {
            // A panic of the thread is already logged by the loop.
            drop(thread.join());
        }
    }
}

fn stopped() -> PreviewError {
    PreviewError::Stopped("the engine is shutting down".into())
}

fn reply_to<T: Send + 'static>(
    sender: tokio::sync::oneshot::Sender<Result<T, PreviewError>>,
) -> Reply<T> {
    Box::new(move |result| {
        // The receiver is gone if the command was cancelled; drop the result.
        drop(sender.send(result));
    })
}

async fn receive<T>(
    receiver: tokio::sync::oneshot::Receiver<Result<T, PreviewError>>,
) -> Result<T, PreviewError> {
    receiver.await.unwrap_or_else(|_| {
        Err(PreviewError::Stopped(
            "the render thread dropped the request".into(),
        ))
    })
}

fn fail_job(job: Job, error: &PreviewError) {
    match job {
        Job::Open(_, reply) => reply(Err(PreviewError::Stopped(error.to_string()))),
        Job::Close(_) => {}
    }
}

/// The work that the render thread takes from the queue.
enum Work {
    Job(Job),
    Render(RenderJob),
    Stop(Vec<Job>, Option<RenderJob>),
}

fn next_work(shared: &Shared) -> Work {
    let mut queue = shared.lock();
    loop {
        if queue.stop {
            let jobs = queue.jobs.drain(..).collect();
            return Work::Stop(jobs, queue.render.take());
        }
        if let Some(job) = queue.jobs.pop_front() {
            return Work::Job(job);
        }
        if let Some(render) = queue.render.take() {
            return Work::Render(render);
        }
        queue = shared
            .wake
            .wait(queue)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
}

/// The GPU objects of a session at one frame size.
struct View {
    renderer: PreviewRenderer,
    encoder: DisplayEncoder,
}

impl View {
    /// Downsamples `base` to `long_edge` and uploads it.
    fn new(
        ctx: &GpuContext,
        base: &WorkingImage,
        long_edge: u32,
        space: DisplaySpace,
    ) -> Result<Self, PreviewError> {
        pollster::block_on(async {
            let renderer = PreviewRenderer::new(ctx, base, long_edge).await?;
            let (width, height) = renderer.size();
            let encoder = DisplayEncoder::new(ctx, width, height, space).await?;
            Ok::<_, DevelopError>(Self { renderer, encoder })
        })
        .map_err(PreviewError::from)
    }
}

struct Session {
    id: SessionId,
    /// The source, downsampled to the long edge of the open. Views of
    /// other sizes are made from it.
    base: WorkingImage,
    /// The GPU view. `None` only after a failed resize; the next frame
    /// tries again.
    view: Option<View>,
    space: DisplaySpace,
    source: SourceInfo,
    next_seq: u32,
}

impl Session {
    fn base_long_edge(&self) -> u32 {
        self.base.width().max(self.base.height())
    }

    /// Returns a view of the size for `long_edge`. Makes a new view if the
    /// current view has another size; frees the old view first.
    fn view_for(&mut self, ctx: &GpuContext, long_edge: u32) -> Result<&View, PreviewError> {
        let wanted = preview_size(self.base.width(), self.base.height(), long_edge)?;
        let fits = self
            .view
            .as_ref()
            .is_some_and(|view| view.renderer.size() == wanted);
        if !fits {
            // Free the GPU memory of the old view before the new upload.
            self.view = None;
            let view = View::new(ctx, &self.base, long_edge, self.space)?;
            tracing::debug!(
                session = self.id,
                width = wanted.0,
                height = wanted.1,
                "preview view resized"
            );
            self.view = Some(view);
        }
        self.view
            .as_ref()
            .ok_or(PreviewError::Internal("the session view is missing"))
    }
}

/// The state of the render thread.
#[derive(Default)]
struct Renderer {
    gpu: Option<GpuContext>,
    /// Open sessions, the most recently used last.
    sessions: Vec<Session>,
    next_id: SessionId,
}

impl Renderer {
    fn gpu(&mut self) -> Result<&GpuContext, PreviewError> {
        if self.gpu.is_none() {
            let started = Instant::now();
            let ctx = pollster::block_on(GpuContext::new(GpuContextOptions::default()))
                .map_err(PreviewError::GpuUnavailable)?;
            tracing::info!(
                adapter = %ctx.adapter_summary().name,
                ms = started.elapsed().as_millis(),
                "preview GPU context created"
            );
            self.gpu = Some(ctx);
        }
        self.gpu
            .as_ref()
            .ok_or(PreviewError::Internal("the GPU context is missing"))
    }

    fn open(&mut self, request: &OpenRequest) -> Result<SessionInfo, PreviewError> {
        if request.max_long_edge == 0 || request.max_long_edge > MAX_LONG_EDGE {
            return Err(PreviewError::InvalidSize(request.max_long_edge));
        }
        let ctx = self.gpu()?;
        let base = downsample_area(&request.image, request.max_long_edge)?;
        let base_long_edge = base.width().max(base.height());
        let view = View::new(ctx, &base, base_long_edge, request.space)?;
        if self.sessions.len() >= MAX_SESSIONS {
            let closed = self.sessions.remove(0);
            tracing::debug!(
                session = closed.id,
                "preview session closed by a newer open"
            );
        }
        self.next_id += 1;
        let (width, height) = view.renderer.size();
        let id = self.next_id;
        self.sessions.push(Session {
            id,
            base,
            view: Some(view),
            space: request.space,
            source: request.source,
            next_seq: 0,
        });
        Ok(SessionInfo {
            session: id,
            width,
            height,
        })
    }

    fn close(&mut self, id: SessionId) {
        self.sessions.retain(|session| session.id != id);
    }

    fn render(&mut self, job: &RenderJob) -> Result<Frame, PreviewError> {
        let started = Instant::now();
        let id = job.request.session;
        let index = self
            .sessions
            .iter()
            .position(|session| session.id == id)
            .ok_or(PreviewError::SessionNotFound(id))?;
        // Move the session to the end: it is now the most recently used.
        let session = self.sessions.remove(index);
        self.sessions.push(session);
        let ctx = self
            .gpu
            .as_ref()
            .ok_or(PreviewError::Internal("a session exists without a GPU"))?;
        let session = self
            .sessions
            .last_mut()
            .ok_or(PreviewError::Internal("the session list is empty"))?;
        let long_edge = effective_long_edge(session.base_long_edge(), job.request.long_edge)?;
        let pipeline = Pipeline::new(&job.request.settings, &session.source)?;
        let seq = session.next_seq;
        session.next_seq = seq.wrapping_add(1);
        let view = session.view_for(ctx, long_edge)?;
        let (width, height) = view.renderer.size();
        let mut header = FrameHeader {
            width,
            height,
            seq,
            timings: FrameTimings::default(),
        };
        let mut body = match job.request.format {
            FrameFormat::Rgba8 => pollster::block_on(async {
                view.renderer.develop(ctx, &pipeline).await?;
                view.encoder.encode(ctx, view.renderer.output()).await?;
                let mut body = vec![0_u8; rgba8_body_len(width, height)];
                view.encoder
                    .read_into(ctx, &mut body[super::frame::FRAME_HEADER_BYTES..])
                    .await?;
                Ok::<_, DevelopError>(body)
            })?,
            format @ (FrameFormat::Jpeg | FrameFormat::Png) => {
                let image = pollster::block_on(view.renderer.render(ctx, &pipeline))?;
                let options = if format == FrameFormat::Jpeg {
                    EncodeOptions::jpeg(FRAME_JPEG_QUALITY)
                } else {
                    EncodeOptions::png(BitDepth::Eight, false)
                };
                encode(&image, &options)?
            }
        };
        header.timings = FrameTimings {
            queue: started.duration_since(job.queued_at),
            render: started.elapsed(),
        };
        if job.request.format == FrameFormat::Rgba8 && !header.write(&mut body) {
            return Err(PreviewError::Internal(
                "the frame body is shorter than its header",
            ));
        }
        Ok(Frame {
            format: job.request.format,
            header,
            body,
        })
    }
}

fn render_loop(shared: &Shared) {
    let mut renderer = Renderer::default();
    loop {
        match next_work(shared) {
            Work::Stop(jobs, render) => {
                for job in jobs {
                    fail_job(job, &stopped());
                }
                if let Some(render) = render {
                    (render.reply)(Err(stopped()));
                }
                return;
            }
            Work::Job(Job::Close(id)) => renderer.close(id),
            Work::Job(Job::Open(request, reply)) => {
                let result = guarded(&mut renderer, |r| r.open(&request));
                reply(result);
            }
            Work::Render(job) => {
                let result = guarded(&mut renderer, |r| r.render(&job));
                if let Err(error) = &result {
                    tracing::warn!(session = job.request.session, %error, "preview frame failed");
                }
                (job.reply)(result);
            }
        }
    }
}

/// Runs `op` and turns a panic into [`PreviewError::Stopped`]. After a
/// panic the GPU state is unknown, so all sessions and the context are
/// dropped; the next open makes a new context.
fn guarded<T>(
    renderer: &mut Renderer,
    op: impl FnOnce(&mut Renderer) -> Result<T, PreviewError>,
) -> Result<T, PreviewError> {
    match catch_unwind(AssertUnwindSafe(|| op(renderer))) {
        Ok(result) => result,
        Err(panic) => {
            let message = panic
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_owned());
            tracing::error!(%message, "the preview render thread panicked; GPU state reset");
            *renderer = Renderer {
                next_id: renderer.next_id,
                ..Renderer::default()
            };
            Err(PreviewError::Stopped(format!(
                "a render step panicked: {message}"
            )))
        }
    }
}

/// Makes a synthetic test image (linear Rec. 2020): smooth ramps, a gray
/// wedge, and saturated patches. The transport benchmark uses it, so that
/// it needs no photo in the catalog.
pub fn test_pattern(width: u32, height: u32) -> Result<WorkingImage, expose_core::CoreError> {
    let (w, h) = (width.max(1), height.max(1));
    #[allow(
        clippy::cast_precision_loss,
        reason = "pixel coordinates are far below 2^24"
    )]
    let pixels = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x as f32 / w as f32, y as f32 / h as f32)))
        .map(|(u, v)| {
            if v < 0.1 {
                // Gray wedge in 16 steps.
                let step = (u * 16.0).floor() / 15.0;
                let lin = step * step;
                [lin, lin, lin, 1.0]
            } else if v > 0.9 {
                // Saturated patches.
                const PATCHES: [[f32; 4]; 6] = [
                    [0.6, 0.05, 0.05, 1.0],
                    [0.05, 0.6, 0.05, 1.0],
                    [0.05, 0.05, 0.6, 1.0],
                    [0.6, 0.6, 0.05, 1.0],
                    [0.05, 0.6, 0.6, 1.0],
                    [0.6, 0.05, 0.6, 1.0],
                ];
                let index = PATCHES
                    .iter()
                    .enumerate()
                    .position(|(i, _)| u * 6.0 < (i + 1) as f32)
                    .unwrap_or(PATCHES.len() - 1);
                PATCHES[index]
            } else {
                [u * u, v * 0.8, (1.0 - u) * 0.5 * v, 1.0]
            }
        })
        .collect();
    WorkingImage::from_pixels(w, h, pixels)
}

#[cfg(test)]
mod tests {
    // Test code: a failed setup is a test failure.
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::time::Duration;

    use super::*;

    /// Starts an engine and opens a test pattern session. Returns `None`
    /// when the machine has no GPU.
    fn engine_with_session(width: u32, height: u32) -> Option<(PreviewEngine, SessionInfo)> {
        let engine = PreviewEngine::start().unwrap();
        let request = OpenRequest {
            image: test_pattern(width, height).unwrap(),
            source: SourceInfo::RENDERED,
            max_long_edge: DEFAULT_LONG_EDGE,
            space: DisplaySpace::Srgb,
        };
        match pollster::block_on(engine.open_async(request)) {
            Ok(info) => Some((engine, info)),
            Err(PreviewError::GpuUnavailable(error)) => {
                eprintln!("SKIPPED: no GPU ({error})");
                None
            }
            Err(error) => panic!("open failed: {error}"),
        }
    }

    fn exposure(ev: f32) -> DevelopSettings {
        let mut settings = DevelopSettings::default();
        settings.tone.exposure = ev;
        settings
    }

    #[test]
    fn frame_format_parses_query_values() {
        assert_eq!(FrameFormat::parse("rgba8"), Some(FrameFormat::Rgba8));
        assert_eq!(FrameFormat::parse("jpeg"), Some(FrameFormat::Jpeg));
        assert_eq!(FrameFormat::parse("png"), Some(FrameFormat::Png));
        assert_eq!(FrameFormat::parse("gif"), None);
        assert_eq!(FrameFormat::Png.mime(), "image/png");
    }

    #[test]
    fn errors_map_to_command_kinds() {
        let kind = |e: PreviewError| CommandError::from(e).kind;
        assert_eq!(kind(PreviewError::Superseded), ErrorKind::Superseded);
        assert_eq!(kind(PreviewError::SessionNotFound(3)), ErrorKind::NotFound);
        assert_eq!(kind(PreviewError::InvalidSize(0)), ErrorKind::InvalidInput);
        assert_eq!(kind(PreviewError::Stopped("x".into())), ErrorKind::Internal);
    }

    #[test]
    fn renders_rgba8_frames_with_header() {
        let Some((engine, info)) = engine_with_session(640, 427) else {
            return;
        };
        assert_eq!((info.width, info.height), (640, 427));
        for seq in 0..3 {
            let frame = engine
                .render_blocking(RenderRequest {
                    session: info.session,
                    settings: exposure(0.5),
                    format: FrameFormat::Rgba8,
                    long_edge: None,
                })
                .unwrap();
            assert_eq!(frame.body.len(), rgba8_body_len(640, 427));
            let header = FrameHeader::read(&frame.body).unwrap();
            // The body keeps whole microseconds only.
            assert_eq!(
                header.timings.render.as_micros(),
                frame.header.timings.render.as_micros()
            );
            assert_eq!((header.width, header.height, header.seq), (640, 427, seq));
            assert_eq!(frame.header.seq, seq);
            // Alpha is opaque everywhere.
            assert!(frame.body[32..].chunks_exact(4).all(|px| px[3] == 255));
        }
    }

    #[test]
    fn renders_encoded_frames() {
        let Some((engine, info)) = engine_with_session(320, 200) else {
            return;
        };
        for (format, magic) in [
            (FrameFormat::Jpeg, &[0xFF_u8, 0xD8][..]),
            (FrameFormat::Png, &[0x89, b'P', b'N', b'G'][..]),
        ] {
            let frame = engine
                .render_blocking(RenderRequest {
                    session: info.session,
                    settings: DevelopSettings::default(),
                    format,
                    long_edge: None,
                })
                .unwrap();
            assert!(frame.body.starts_with(magic), "{format:?}");
        }
    }

    #[test]
    fn unknown_session_and_bad_settings_fail() {
        let Some((engine, info)) = engine_with_session(64, 64) else {
            return;
        };
        let error = engine
            .render_blocking(RenderRequest {
                session: info.session + 100,
                settings: DevelopSettings::default(),
                format: FrameFormat::Rgba8,
                long_edge: None,
            })
            .unwrap_err();
        assert!(matches!(error, PreviewError::SessionNotFound(_)), "{error}");

        let error = engine
            .render_blocking(RenderRequest {
                session: info.session,
                settings: exposure(f32::NAN),
                format: FrameFormat::Rgba8,
                long_edge: None,
            })
            .unwrap_err();
        assert_eq!(CommandError::from(error).kind, ErrorKind::InvalidInput);

        engine.close(info.session);
        let error = engine
            .render_blocking(RenderRequest {
                session: info.session,
                settings: DevelopSettings::default(),
                format: FrameFormat::Rgba8,
                long_edge: None,
            })
            .unwrap_err();
        assert!(matches!(error, PreviewError::SessionNotFound(_)), "{error}");
    }

    #[test]
    fn open_rejects_bad_size_and_evicts_oldest_session() {
        let Some((engine, first)) = engine_with_session(64, 64) else {
            return;
        };
        let open = |edge| {
            pollster::block_on(engine.open_async(OpenRequest {
                image: test_pattern(64, 64).unwrap(),
                source: SourceInfo::RENDERED,
                max_long_edge: edge,
                space: DisplaySpace::DisplayP3,
            }))
        };
        assert!(matches!(open(0), Err(PreviewError::InvalidSize(0))));
        let second = open(32).unwrap();
        assert_eq!((second.width, second.height), (32, 32));
        let third = open(64).unwrap();
        assert!(third.session > second.session);
        // MAX_SESSIONS = 2: the first session is closed.
        let error = engine
            .render_blocking(RenderRequest {
                session: first.session,
                settings: DevelopSettings::default(),
                format: FrameFormat::Rgba8,
                long_edge: None,
            })
            .unwrap_err();
        assert!(matches!(error, PreviewError::SessionNotFound(_)), "{error}");
    }

    #[test]
    fn newer_request_supersedes_waiting_request() {
        let Some((engine, info)) = engine_with_session(1024, 683) else {
            return;
        };
        // Send many requests at once. Only the request that is running and
        // the last one can complete; all others must be superseded.
        let (sender, receiver) = std::sync::mpsc::channel();
        let count = 20_u32;
        for i in 0..count {
            let sender = sender.clone();
            #[allow(clippy::cast_precision_loss, reason = "i < 20")]
            let settings = exposure(i as f32 * 0.1);
            engine.render(
                RenderRequest {
                    session: info.session,
                    settings,
                    format: FrameFormat::Rgba8,
                    long_edge: None,
                },
                Box::new(move |result| drop(sender.send((i, result)))),
            );
        }
        drop(sender);
        let results: Vec<_> = receiver.iter().collect();
        assert_eq!(results.len(), count as usize);
        let completed: Vec<u32> = results
            .iter()
            .filter(|(_, r)| r.is_ok())
            .map(|(i, _)| *i)
            .collect();
        let superseded = results
            .iter()
            .filter(|(_, r)| matches!(r, Err(PreviewError::Superseded)))
            .count();
        assert!(completed.contains(&(count - 1)), "{completed:?}");
        assert!(completed.len() <= 2, "{completed:?}");
        assert_eq!(completed.len() + superseded, count as usize);
    }

    #[test]
    fn drop_stops_the_thread_and_fails_new_requests() {
        let engine = PreviewEngine::start().unwrap();
        let shared = Arc::clone(&engine.shared);
        drop(engine);
        assert!(shared.lock().stop);
        // An engine that has stopped answers at once.
        let engine = PreviewEngine {
            shared,
            thread: None,
        };
        let error = engine
            .render_blocking(RenderRequest {
                session: 1,
                settings: DevelopSettings::default(),
                format: FrameFormat::Rgba8,
                long_edge: None,
            })
            .unwrap_err();
        assert!(matches!(error, PreviewError::Stopped(_)), "{error}");
    }

    /// An engine with no render thread: requests stay in the queue, so the
    /// queue rules can be tested with no GPU and no timing.
    fn engine_without_thread() -> PreviewEngine {
        PreviewEngine {
            shared: Arc::new(Shared {
                queue: Mutex::new(Queue::default()),
                wake: Condvar::new(),
            }),
            thread: None,
        }
    }

    fn rgba8_request(session: SessionId, long_edge: Option<u32>) -> RenderRequest {
        RenderRequest {
            session,
            settings: DevelopSettings::default(),
            format: FrameFormat::Rgba8,
            long_edge,
        }
    }

    #[test]
    fn effective_long_edge_validates_and_clamps() {
        assert_eq!(effective_long_edge(2560, None).unwrap(), 2560);
        assert_eq!(effective_long_edge(2560, Some(1200)).unwrap(), 1200);
        // No upscale beyond the base.
        assert_eq!(effective_long_edge(2560, Some(4000)).unwrap(), 2560);
        assert_eq!(
            effective_long_edge(2560, Some(MAX_LONG_EDGE)).unwrap(),
            2560
        );
        // Small values increase to the minimum.
        assert_eq!(effective_long_edge(2560, Some(1)).unwrap(), MIN_LONG_EDGE);
        // A base below the minimum keeps its size.
        assert_eq!(effective_long_edge(40, Some(1)).unwrap(), 40);
        assert!(matches!(
            effective_long_edge(2560, Some(0)),
            Err(PreviewError::InvalidSize(0))
        ));
        assert!(matches!(
            effective_long_edge(2560, Some(MAX_LONG_EDGE + 1)),
            Err(PreviewError::InvalidSize(_))
        ));
    }

    #[test]
    fn waiting_slot_holds_only_the_newest_frame_request() {
        let engine = engine_without_thread();
        let (sender, receiver) = std::sync::mpsc::channel();
        for i in 0..3_u32 {
            let sender = sender.clone();
            engine.render(
                rgba8_request(u64::from(i), None),
                Box::new(move |result| drop(sender.send((i, result)))),
            );
        }
        drop(sender);
        // The first two requests are superseded at once, in order.
        let replies: Vec<_> = receiver.try_iter().collect();
        assert_eq!(replies.len(), 2);
        for (expected, (i, result)) in replies.iter().enumerate() {
            assert_eq!(*i, u32::try_from(expected).unwrap());
            assert!(matches!(result, Err(PreviewError::Superseded)));
        }
        // Only the newest request waits.
        let queue = engine.shared.lock();
        let waiting = queue.render.as_ref().map(|job| job.request.session);
        assert_eq!(waiting, Some(2));
    }

    #[test]
    fn open_and_close_jobs_run_before_the_waiting_frame() {
        let engine = engine_without_thread();
        engine.render(rgba8_request(1, None), Box::new(drop));
        engine.close(7);
        engine.close(8);
        let order: Vec<String> = (0..3)
            .map(|_| match next_work(&engine.shared) {
                Work::Job(Job::Close(id)) => format!("close {id}"),
                Work::Job(Job::Open(..)) => "open".to_owned(),
                Work::Render(job) => format!("render {}", job.request.session),
                Work::Stop(..) => "stop".to_owned(),
            })
            .collect();
        assert_eq!(order, ["close 7", "close 8", "render 1"]);
    }

    #[test]
    fn two_requests_in_flight_both_complete_in_order() {
        let Some((engine, info)) = engine_with_session(1024, 683) else {
            return;
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        let send = |i: u32| {
            let sender = sender.clone();
            engine.render(
                rgba8_request(info.session, None),
                Box::new(move |result| drop(sender.send((i, result)))),
            );
        };
        send(0);
        // Wait until the render thread takes the first request. Then the
        // second request waits in the slot and nothing supersedes it.
        let deadline = Instant::now() + Duration::from_secs(10);
        while engine.shared.lock().render.is_some() {
            assert!(Instant::now() < deadline, "the render thread did not start");
            std::thread::sleep(Duration::from_millis(1));
        }
        send(1);
        drop(sender);
        let results: Vec<_> = receiver.iter().collect();
        let seqs: Vec<(u32, u32)> = results
            .into_iter()
            .map(|(i, result)| (i, result.unwrap().header.seq))
            .collect();
        assert_eq!(seqs, [(0, 0), (1, 1)]);
    }

    #[test]
    fn frames_follow_the_requested_long_edge() {
        let Some((engine, info)) = engine_with_session(640, 427) else {
            return;
        };
        let size = |long_edge| {
            engine
                .render_blocking(rgba8_request(info.session, long_edge))
                .map(|frame| {
                    assert_eq!(
                        frame.body.len(),
                        rgba8_body_len(frame.header.width, frame.header.height)
                    );
                    (frame.header.width, frame.header.height, frame.header.seq)
                })
        };
        let small = preview_size(640, 427, 320).unwrap();
        assert_eq!(size(Some(320)).unwrap(), (small.0, small.1, 0));
        let min = preview_size(640, 427, MIN_LONG_EDGE).unwrap();
        assert_eq!(size(Some(3)).unwrap(), (min.0, min.1, 1));
        // Larger than the base: the base size.
        assert_eq!(size(Some(5000)).unwrap(), (640, 427, 2));
        assert!(matches!(size(Some(0)), Err(PreviewError::InvalidSize(0))));
        assert!(matches!(
            size(Some(MAX_LONG_EDGE + 1)),
            Err(PreviewError::InvalidSize(_))
        ));
        // A rejected request takes no sequence number.
        assert_eq!(size(None).unwrap(), (640, 427, 3));
        assert_eq!(size(Some(320)).unwrap(), (small.0, small.1, 4));
    }

    /// Stage timings at the default preview size. Run with:
    /// `cargo test -p expose-desktop --release -- --ignored --nocapture preview_stage_timings`
    #[test]
    #[ignore = "timing measurement; run by hand"]
    #[allow(
        clippy::too_many_lines,
        reason = "one linear measurement script is easier to read"
    )]
    fn preview_stage_timings() {
        let Ok(ctx) = pollster::block_on(GpuContext::new(GpuContextOptions::default())) else {
            eprintln!("SKIPPED: no GPU");
            return;
        };
        eprintln!("adapter: {}", ctx.adapter_summary().name);
        let source = test_pattern(6000, 4000).unwrap();
        let renderer =
            pollster::block_on(PreviewRenderer::new(&ctx, &source, DEFAULT_LONG_EDGE)).unwrap();
        let (w, h) = renderer.size();
        let encoder =
            pollster::block_on(DisplayEncoder::new(&ctx, w, h, DisplaySpace::Srgb)).unwrap();
        let mut body = vec![0_u8; rgba8_body_len(w, h)];
        let runs = 30;
        let mut stages: Vec<(&str, Vec<Duration>)> = [
            "GPU develop",
            "GPU display encode",
            "read-back RGBA8",
            "frame total (develop+encode+read)",
            "read-back RGBA f32",
            "CPU to_image sRGB 8-bit",
            "JPEG encode q85 (from f32)",
            "PNG encode 8-bit (from f32)",
        ]
        .into_iter()
        .map(|name| (name, Vec::new()))
        .collect();
        for i in 0..runs {
            #[allow(clippy::cast_precision_loss, reason = "i < 30")]
            let pipeline =
                Pipeline::new(&exposure(i as f32 * 0.05), &SourceInfo::RENDERED).unwrap();
            let t = Instant::now();
            pollster::block_on(renderer.develop(&ctx, &pipeline)).unwrap();
            ctx.wait_idle().unwrap();
            stages[0].1.push(t.elapsed());
            let t1 = Instant::now();
            pollster::block_on(encoder.encode(&ctx, renderer.output())).unwrap();
            ctx.wait_idle().unwrap();
            stages[1].1.push(t1.elapsed());
            let t2 = Instant::now();
            pollster::block_on(encoder.read_into(&ctx, &mut body[32..])).unwrap();
            stages[2].1.push(t2.elapsed());
            stages[3].1.push(t.elapsed());

            let t3 = Instant::now();
            let samples = pollster::block_on(renderer.output().read_rgba_f32(&ctx)).unwrap();
            stages[4].1.push(t3.elapsed());
            drop(samples);
            if i < 5 {
                let image = pollster::block_on(renderer.render(&ctx, &pipeline)).unwrap();
                let t4 = Instant::now();
                let bytes = image
                    .to_image::<u8>(
                        expose_core::color::ColorSpaceId::Srgb,
                        expose_core::buffer::ChannelLayout::Rgba,
                    )
                    .unwrap();
                stages[5].1.push(t4.elapsed());
                drop(bytes);
                let t5 = Instant::now();
                let jpeg = encode(&image, &EncodeOptions::jpeg(FRAME_JPEG_QUALITY)).unwrap();
                stages[6].1.push(t5.elapsed());
                let t6 = Instant::now();
                let png = encode(&image, &EncodeOptions::png(BitDepth::Eight, false)).unwrap();
                stages[7].1.push(t6.elapsed());
                if i == 0 {
                    eprintln!(
                        "sizes: RGBA8 body {} bytes, JPEG {} bytes, PNG {} bytes",
                        body.len(),
                        jpeg.len(),
                        png.len()
                    );
                }
            }
        }
        eprintln!("preview {w}x{h}, median of runs:");
        for (name, mut times) in stages {
            times.sort();
            let median = times[times.len() / 2];
            eprintln!("  {name:<36} {:>8.2} ms", median.as_secs_f64() * 1000.0);
        }

        // End to end through the engine (one request at a time).
        let Some((engine, info)) = engine_with_session(6000, 4000) else {
            return;
        };
        let frames = 60;
        let t = Instant::now();
        for i in 0..frames {
            #[allow(clippy::cast_precision_loss, reason = "i < 60")]
            let settings = exposure(i as f32 * 0.02);
            engine
                .render_blocking(RenderRequest {
                    session: info.session,
                    settings,
                    format: FrameFormat::Rgba8,
                    long_edge: None,
                })
                .unwrap();
        }
        let elapsed = t.elapsed();
        eprintln!(
            "engine RGBA8 frames: {frames} in {:.1} ms = {:.1} fps",
            elapsed.as_secs_f64() * 1000.0,
            f64::from(frames) / elapsed.as_secs_f64()
        );
    }
}
