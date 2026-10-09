//! The editor engine: one render thread that owns the open documents, the
//! GPU context, and one GPU compositor.
//!
//! # Threads
//!
//! The engine starts one thread at application start. The thread makes
//! its GPU context and the compositor at the first document open (lazy).
//! All document changes and all GPU work run on this thread. The decode
//! and the develop of a new document run before the open, on a blocking
//! thread of the command.
//!
//! # Order and coalescing
//!
//! Jobs (open, edit, undo, redo, close) run in order. Waiting frame
//! requests run after all waiting jobs, so a frame always shows the newest
//! edits. The engine keeps at most one waiting frame request for each
//! document: a newer request for the same document replaces it, and the
//! old request gets [`EditorEngineError::Superseded`] (command error kind
//! `superseded`). The UI keeps up to two requests in flight, so render
//! and transfer overlap, as for the develop preview.
//!
//! # GPU memory
//!
//! The compositor is shared by all documents. Its cache keys are layer IDs
//! with process-unique version stamps (ADR 0014), so entries of different
//! documents do not mix. When the last document closes, the engine clears
//! the compositor cache. Each document has its own display encoder of the
//! last view size.

use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use expose_develop::{DisplayEncoder, DisplaySpace};
use expose_editor::{Document, GpuCompositor, ViewSpec};
use expose_gpu::{GpuContext, GpuContextOptions};

use super::EditorEngineError;
use super::dto::{DocumentId, DocumentInfo, EditDto};
use super::session::EditSession;
use crate::preview::frame::{FRAME_HEADER_BYTES, FrameHeader, FrameTimings, rgba8_body_len};

/// The largest view width and height in pixels. The compositor can also
/// reject a smaller size if the device limit is smaller.
pub const MAX_VIEW_SIDE: u32 = 8192;

/// A function that receives the result of a job. It runs on the render
/// thread, so it must return fast.
pub type Reply<T> = Box<dyn FnOnce(Result<T, EditorEngineError>) + Send + 'static>;

/// One frame request: a view of a document.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderRequest {
    /// The document.
    pub document: DocumentId,
    /// The view: canvas coordinate of the top-left output pixel, output
    /// pixels per canvas pixel, and output size.
    pub view: ViewSpec,
}

enum Job {
    Open {
        document: Box<Document>,
        space: DisplaySpace,
        reply: Reply<DocumentInfo>,
    },
    Edit {
        document: DocumentId,
        edit: EditDto,
        gesture: Option<u64>,
        reply: Reply<DocumentInfo>,
    },
    Undo {
        document: DocumentId,
        reply: Reply<DocumentInfo>,
    },
    Redo {
        document: DocumentId,
        reply: Reply<DocumentInfo>,
    },
    Close(DocumentId),
}

struct RenderJob {
    request: RenderRequest,
    queued_at: Instant,
    reply: Reply<Vec<u8>>,
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    /// At most one waiting frame request for each document, oldest first.
    renders: VecDeque<RenderJob>,
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

/// The editor engine. Keep one in the Tauri managed state.
pub struct EditorEngine {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for EditorEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorEngine").finish_non_exhaustive()
    }
}

impl EditorEngine {
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
            .name("expose-editor".into())
            .spawn(move || render_loop(&worker))?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    /// Opens `document` (frames in the display space `space`) and waits
    /// for the result. The open is not an undo step.
    ///
    /// # Errors
    ///
    /// [`EditorEngineError::GpuUnavailable`] if the GPU context cannot be
    /// made, and the errors of a stopped engine.
    pub async fn open_async(
        &self,
        document: Document,
        space: DisplaySpace,
    ) -> Result<DocumentInfo, EditorEngineError> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.push_job(Job::Open {
            document: Box::new(document),
            space,
            reply: reply_to(sender),
        });
        receive(receiver).await
    }

    /// Applies an edit (see [`EditSession::edit`]) and waits for the new
    /// document info.
    ///
    /// # Errors
    ///
    /// [`EditorEngineError::DocumentNotFound`] and the edit errors.
    pub async fn edit_async(
        &self,
        document: DocumentId,
        edit: EditDto,
        gesture: Option<u64>,
    ) -> Result<DocumentInfo, EditorEngineError> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.push_job(Job::Edit {
            document,
            edit,
            gesture,
            reply: reply_to(sender),
        });
        receive(receiver).await
    }

    /// Undoes the last step of a document and waits for the new info.
    ///
    /// # Errors
    ///
    /// [`EditorEngineError::DocumentNotFound`] and history errors.
    pub async fn undo_async(
        &self,
        document: DocumentId,
    ) -> Result<DocumentInfo, EditorEngineError> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.push_job(Job::Undo {
            document,
            reply: reply_to(sender),
        });
        receive(receiver).await
    }

    /// Redoes the next step of a document and waits for the new info.
    ///
    /// # Errors
    ///
    /// [`EditorEngineError::DocumentNotFound`] and history errors.
    pub async fn redo_async(
        &self,
        document: DocumentId,
    ) -> Result<DocumentInfo, EditorEngineError> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.push_job(Job::Redo {
            document,
            reply: reply_to(sender),
        });
        receive(receiver).await
    }

    /// Queues a document close. A close of an unknown document does
    /// nothing. When no document stays open, the engine frees the GPU
    /// cache of the compositor.
    pub fn close(&self, document: DocumentId) {
        self.push_job(Job::Close(document));
    }

    /// Queues a frame request. A waiting older request for the same
    /// document gets [`EditorEngineError::Superseded`].
    pub fn render(&self, request: RenderRequest, reply: Reply<Vec<u8>>) {
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
            let waiting = queue
                .renders
                .iter_mut()
                .find(|w| w.request.document == job.request.document);
            if let Some(slot) = waiting {
                Some(std::mem::replace(slot, job))
            } else {
                queue.renders.push_back(job);
                None
            }
        };
        self.shared.wake.notify_one();
        // Reply outside the lock.
        if let Some(old) = replaced {
            (old.reply)(Err(EditorEngineError::Superseded));
        }
    }

    /// Requests a frame and waits for the frame body (`EXF1` header and
    /// RGBA8 pixels).
    ///
    /// # Errors
    ///
    /// [`EditorEngineError::Superseded`] when a newer request for the
    /// same document replaced this one, and the render errors.
    pub async fn render_async(&self, request: RenderRequest) -> Result<Vec<u8>, EditorEngineError> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.render(request, reply_to(sender));
        receive(receiver).await
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

impl Drop for EditorEngine {
    fn drop(&mut self) {
        self.shared.lock().stop = true;
        self.shared.wake.notify_all();
        if let Some(thread) = self.thread.take() {
            // A panic of the thread is already logged by the loop.
            drop(thread.join());
        }
    }
}

fn stopped() -> EditorEngineError {
    EditorEngineError::Stopped("the engine is shutting down".into())
}

fn reply_to<T: Send + 'static>(
    sender: tokio::sync::oneshot::Sender<Result<T, EditorEngineError>>,
) -> Reply<T> {
    Box::new(move |result| {
        // The receiver is gone if the command was cancelled; drop the result.
        drop(sender.send(result));
    })
}

async fn receive<T>(
    receiver: tokio::sync::oneshot::Receiver<Result<T, EditorEngineError>>,
) -> Result<T, EditorEngineError> {
    receiver.await.unwrap_or_else(|_| {
        Err(EditorEngineError::Stopped(
            "the render thread dropped the request".into(),
        ))
    })
}

fn fail_job(job: Job, error: &EditorEngineError) {
    let error = || EditorEngineError::Stopped(error.to_string());
    match job {
        Job::Open { reply, .. }
        | Job::Edit { reply, .. }
        | Job::Undo { reply, .. }
        | Job::Redo { reply, .. } => reply(Err(error())),
        Job::Close(_) => {}
    }
}

/// The work that the render thread takes from the queue.
enum Work {
    Job(Job),
    Render(RenderJob),
    Stop(Vec<Job>, Vec<RenderJob>),
}

fn next_work(shared: &Shared) -> Work {
    let mut queue = shared.lock();
    loop {
        if queue.stop {
            let jobs = queue.jobs.drain(..).collect();
            let renders = queue.renders.drain(..).collect();
            return Work::Stop(jobs, renders);
        }
        if let Some(job) = queue.jobs.pop_front() {
            return Work::Job(job);
        }
        if let Some(render) = queue.renders.pop_front() {
            return Work::Render(render);
        }
        queue = shared
            .wake
            .wait(queue)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
}

/// The GPU objects of the thread.
struct Gpu {
    ctx: GpuContext,
    compositor: GpuCompositor,
}

/// An open document on the render thread.
struct OpenDoc {
    session: EditSession,
    space: DisplaySpace,
    /// The display encoder of the last view size; made again when the view
    /// size changes.
    display: Option<DisplayEncoder>,
    next_seq: u32,
}

/// The state of the render thread.
#[derive(Default)]
struct Renderer {
    gpu: Option<Gpu>,
    documents: HashMap<DocumentId, OpenDoc>,
    next_id: DocumentId,
}

/// Returns the GPU objects and makes them at the first call.
fn ensure_gpu(slot: &mut Option<Gpu>) -> Result<&mut Gpu, EditorEngineError> {
    if slot.is_none() {
        let started = Instant::now();
        let ctx = pollster::block_on(GpuContext::new(GpuContextOptions::default()))
            .map_err(EditorEngineError::GpuUnavailable)?;
        let compositor = pollster::block_on(GpuCompositor::new(&ctx))?;
        tracing::info!(
            adapter = %ctx.adapter_summary().name,
            ms = started.elapsed().as_millis(),
            "editor GPU context created"
        );
        *slot = Some(Gpu { ctx, compositor });
    }
    slot.as_mut()
        .ok_or(EditorEngineError::Internal("the GPU context is missing"))
}

impl Renderer {
    fn open(
        &mut self,
        document: Document,
        space: DisplaySpace,
    ) -> Result<DocumentInfo, EditorEngineError> {
        // Make the GPU context now, so that a machine with no GPU gets the
        // error at the open and not at the first frame.
        ensure_gpu(&mut self.gpu)?;
        let session = EditSession::new(document)?;
        self.next_id += 1;
        let id = self.next_id;
        let info = session.info(id);
        self.documents.insert(
            id,
            OpenDoc {
                session,
                space,
                display: None,
                next_seq: 0,
            },
        );
        tracing::debug!(
            document = id,
            width = info.width,
            height = info.height,
            "editor document opened"
        );
        Ok(info)
    }

    fn session(&mut self, id: DocumentId) -> Result<&mut EditSession, EditorEngineError> {
        self.documents
            .get_mut(&id)
            .map(|doc| &mut doc.session)
            .ok_or(EditorEngineError::DocumentNotFound(id))
    }

    fn close(&mut self, id: DocumentId) {
        if self.documents.remove(&id).is_some() {
            tracing::debug!(document = id, "editor document closed");
        }
        if self.documents.is_empty() {
            if let Some(gpu) = &mut self.gpu {
                gpu.compositor.clear_cache();
            }
        }
    }

    fn render(&mut self, job: &RenderJob) -> Result<Vec<u8>, EditorEngineError> {
        let started = Instant::now();
        let RenderRequest { document: id, view } = job.request;
        if view.width == 0
            || view.height == 0
            || view.width > MAX_VIEW_SIDE
            || view.height > MAX_VIEW_SIDE
        {
            return Err(EditorEngineError::InvalidViewSize {
                width: view.width,
                height: view.height,
            });
        }
        let Self { gpu, documents, .. } = self;
        let doc = documents
            .get_mut(&id)
            .ok_or(EditorEngineError::DocumentNotFound(id))?;
        let Gpu { ctx, compositor } = ensure_gpu(gpu)?;
        let size = (view.width, view.height);
        let mut body = vec![0_u8; rgba8_body_len(size.0, size.1)];
        pollster::block_on(async {
            if doc.display.as_ref().map(DisplayEncoder::size) != Some(size) {
                // Free the old encoder first, so that the GPU memory does
                // not increase.
                doc.display = None;
                doc.display = Some(DisplayEncoder::new(ctx, size.0, size.1, doc.space).await?);
            }
            let display = doc.display.as_ref().ok_or(EditorEngineError::Internal(
                "the display encoder is missing",
            ))?;
            let image = compositor
                .render_view(ctx, doc.session.document(), &view)
                .await?;
            display.encode(ctx, image).await?;
            display
                .read_into(ctx, &mut body[FRAME_HEADER_BYTES..])
                .await?;
            Ok::<_, EditorEngineError>(())
        })?;
        let seq = doc.next_seq;
        doc.next_seq = seq.wrapping_add(1);
        let header = FrameHeader {
            width: size.0,
            height: size.1,
            seq,
            timings: FrameTimings {
                queue: started.duration_since(job.queued_at),
                render: started.elapsed(),
            },
        };
        if !header.write(&mut body) {
            return Err(EditorEngineError::Internal(
                "the frame body is shorter than its header",
            ));
        }
        Ok(body)
    }

    fn run_job(&mut self, job: Job) {
        match job {
            Job::Close(id) => self.close(id),
            Job::Open {
                document,
                space,
                reply,
            } => {
                let result = guarded(self, |r| r.open(*document, space));
                reply(result);
            }
            Job::Edit {
                document,
                edit,
                gesture,
                reply,
            } => {
                let result = guarded(self, |r| {
                    let session = r.session(document)?;
                    session.edit(edit, gesture)?;
                    Ok(session.info(document))
                });
                reply(result);
            }
            Job::Undo { document, reply } => {
                let result = guarded(self, |r| {
                    let session = r.session(document)?;
                    session.undo()?;
                    Ok(session.info(document))
                });
                reply(result);
            }
            Job::Redo { document, reply } => {
                let result = guarded(self, |r| {
                    let session = r.session(document)?;
                    session.redo()?;
                    Ok(session.info(document))
                });
                reply(result);
            }
        }
    }
}

fn render_loop(shared: &Shared) {
    let mut renderer = Renderer::default();
    loop {
        match next_work(shared) {
            Work::Stop(jobs, renders) => {
                for job in jobs {
                    fail_job(job, &stopped());
                }
                for render in renders {
                    (render.reply)(Err(stopped()));
                }
                return;
            }
            Work::Job(job) => renderer.run_job(job),
            Work::Render(job) => {
                let result = guarded(&mut renderer, |r| r.render(&job));
                if let Err(error) = &result {
                    tracing::warn!(document = job.request.document, %error, "editor frame failed");
                }
                (job.reply)(result);
            }
        }
    }
}

/// Runs `op` and turns a panic into [`EditorEngineError::Stopped`].
///
/// After a panic the GPU state is unknown, so the GPU context, the
/// compositor, and the display encoders are dropped; the next frame makes
/// them again. The documents stay open: the editor operations change a
/// document only after all checks, so a document is not left half-changed,
/// and a reset would lose the edits of the user.
fn guarded<T>(
    renderer: &mut Renderer,
    op: impl FnOnce(&mut Renderer) -> Result<T, EditorEngineError>,
) -> Result<T, EditorEngineError> {
    match catch_unwind(AssertUnwindSafe(|| op(renderer))) {
        Ok(result) => result,
        Err(panic) => {
            let message = panic
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_owned());
            tracing::error!(%message, "the editor render thread panicked; GPU state reset");
            renderer.gpu = None;
            for doc in renderer.documents.values_mut() {
                doc.display = None;
            }
            Err(EditorEngineError::Stopped(format!(
                "an editor step panicked: {message}"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    // Test code: a failed setup is a test failure.
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::mpsc;

    use expose_core::buffer::WorkingImage;

    use super::*;
    use crate::editor::dto::{AdjustmentDto, BlendModeId};
    use crate::editor::session::build_document;

    /// Starts an engine and opens a `width` x `height` document of one
    /// color. Returns `None` when the machine has no GPU.
    fn engine_with_document(
        width: u32,
        height: u32,
        color: [f32; 4],
    ) -> Option<(EditorEngine, DocumentInfo)> {
        let engine = EditorEngine::start().unwrap();
        let image = WorkingImage::new_filled(width, height, color).unwrap();
        let document = build_document(image).unwrap();
        match pollster::block_on(engine.open_async(document, DisplaySpace::Srgb)) {
            Ok(info) => Some((engine, info)),
            Err(EditorEngineError::GpuUnavailable(error)) => {
                eprintln!("SKIPPED: no GPU ({error})");
                None
            }
            Err(error) => panic!("open failed: {error}"),
        }
    }

    fn view(origin: f64, scale: f64, width: u32, height: u32) -> ViewSpec {
        ViewSpec {
            origin_x: origin,
            origin_y: origin,
            scale,
            width,
            height,
        }
    }

    fn pixel(body: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let i = FRAME_HEADER_BYTES + ((y * width + x) as usize) * 4;
        [body[i], body[i + 1], body[i + 2], body[i + 3]]
    }

    #[test]
    fn renders_a_view_with_header_and_transparent_outside() {
        let Some((engine, info)) = engine_with_document(8, 6, [1.0, 1.0, 1.0, 1.0]) else {
            return;
        };
        assert_eq!((info.width, info.height), (8, 6));
        assert_eq!(info.layers.len(), 1);
        // Origin -2: the first two columns and rows are outside the canvas.
        let request = RenderRequest {
            document: info.document_id,
            view: view(-2.0, 1.0, 12, 10),
        };
        let body = pollster::block_on(engine.render_async(request)).unwrap();
        assert_eq!(body.len(), rgba8_body_len(12, 10));
        let header = FrameHeader::read(&body).unwrap();
        assert_eq!((header.width, header.height, header.seq), (12, 10, 0));
        assert_eq!(pixel(&body, 12, 0, 0)[3], 0);
        assert_eq!(pixel(&body, 12, 4, 4), [255, 255, 255, 255]);
        assert_eq!(pixel(&body, 12, 11, 9)[3], 0);

        // An edit changes the next frame: hide the only layer.
        let layer_id = info.layers[0].id;
        let edited = pollster::block_on(engine.edit_async(
            info.document_id,
            EditDto::SetVisible {
                layer_id,
                visible: false,
            },
            None,
        ))
        .unwrap();
        assert!(edited.can_undo);
        let body = pollster::block_on(engine.render_async(request)).unwrap();
        assert_eq!(FrameHeader::read(&body).unwrap().seq, 1);
        assert_eq!(pixel(&body, 12, 4, 4)[3], 0);
        let undone = pollster::block_on(engine.undo_async(info.document_id)).unwrap();
        assert!(!undone.can_undo && undone.can_redo);
        let body = pollster::block_on(engine.render_async(request)).unwrap();
        assert_eq!(pixel(&body, 12, 4, 4), [255, 255, 255, 255]);
    }

    #[test]
    fn adjustment_and_blend_edits_render() {
        let Some((engine, info)) = engine_with_document(4, 4, [0.18, 0.18, 0.18, 1.0]) else {
            return;
        };
        let id = info.document_id;
        let request = RenderRequest {
            document: id,
            view: view(0.0, 0.5, 2, 2),
        };
        let before = pollster::block_on(engine.render_async(request)).unwrap();
        let info = pollster::block_on(engine.edit_async(
            id,
            EditDto::AddAdjustment {
                adjustment: AdjustmentDto::BrightnessContrast {
                    brightness: 0.5,
                    contrast: 0.0,
                },
                above_layer_id: None,
            },
            None,
        ))
        .unwrap();
        let adjust = info.layers[1].id;
        pollster::block_on(engine.edit_async(
            id,
            EditDto::SetBlendMode {
                layer_id: adjust,
                mode: BlendModeId::Normal,
            },
            Some(1),
        ))
        .unwrap();
        let after = pollster::block_on(engine.render_async(request)).unwrap();
        assert!(pixel(&after, 2, 1, 1)[0] > pixel(&before, 2, 1, 1)[0]);
    }

    #[test]
    fn unknown_document_and_bad_views_fail() {
        let Some((engine, info)) = engine_with_document(4, 4, [0.5; 4]) else {
            return;
        };
        let missing = pollster::block_on(engine.render_async(RenderRequest {
            document: info.document_id + 100,
            view: view(0.0, 1.0, 4, 4),
        }));
        assert!(matches!(
            missing,
            Err(EditorEngineError::DocumentNotFound(_))
        ));
        let zero = pollster::block_on(engine.render_async(RenderRequest {
            document: info.document_id,
            view: view(0.0, 1.0, 0, 4),
        }));
        assert!(matches!(
            zero,
            Err(EditorEngineError::InvalidViewSize { .. })
        ));
        let bad_scale = pollster::block_on(engine.render_async(RenderRequest {
            document: info.document_id,
            view: view(0.0, f64::NAN, 4, 4),
        }));
        assert!(matches!(bad_scale, Err(EditorEngineError::Editor(_))));
        let edit = pollster::block_on(engine.edit_async(
            info.document_id + 100,
            EditDto::Delete { layer_id: 1 },
            None,
        ));
        assert!(matches!(edit, Err(EditorEngineError::DocumentNotFound(_))));
        // After the close, the document is gone.
        engine.close(info.document_id);
        let closed = pollster::block_on(engine.undo_async(info.document_id));
        assert!(matches!(
            closed,
            Err(EditorEngineError::DocumentNotFound(_))
        ));
    }

    /// An engine with no thread, so that requests stay in the queue.
    fn engine_without_thread() -> EditorEngine {
        EditorEngine {
            shared: Arc::new(Shared {
                queue: Mutex::new(Queue::default()),
                wake: Condvar::new(),
            }),
            thread: None,
        }
    }

    #[test]
    fn newer_request_supersedes_only_the_same_document() {
        let engine = engine_without_thread();
        let (sender, receiver) = mpsc::channel();
        let request = |document, tag: u32| {
            let sender = sender.clone();
            let reply: Reply<Vec<u8>> = Box::new(move |result| {
                drop(sender.send((tag, result.map(|_| ()))));
            });
            (
                RenderRequest {
                    document,
                    view: view(0.0, 1.0, 1, 1),
                },
                reply,
            )
        };
        let (r, reply) = request(1, 0);
        engine.render(r, reply);
        let (r, reply) = request(2, 1);
        engine.render(r, reply);
        let (r, reply) = request(1, 2);
        engine.render(r, reply);
        // Only the first request of document 1 has a reply.
        let (tag, result) = receiver.try_recv().unwrap();
        assert_eq!(tag, 0);
        assert!(matches!(result, Err(EditorEngineError::Superseded)));
        assert!(receiver.try_recv().is_err());
        {
            let queue = engine.shared.lock();
            let documents: Vec<DocumentId> =
                queue.renders.iter().map(|j| j.request.document).collect();
            // The newest request of document 1 keeps the place of the old one.
            assert_eq!(documents, [1, 2]);
        }
        // A drop with waiting requests fails them with `Stopped`.
        engine.shared.lock().stop = true;
        let Work::Stop(jobs, renders) = next_work(&engine.shared) else {
            panic!("the queue does not stop");
        };
        assert!(jobs.is_empty());
        assert_eq!(renders.len(), 2);
    }

    #[test]
    fn jobs_run_before_waiting_frames() {
        let engine = engine_without_thread();
        engine.render(
            RenderRequest {
                document: 1,
                view: view(0.0, 1.0, 1, 1),
            },
            Box::new(drop),
        );
        engine.close(1);
        assert!(matches!(
            next_work(&engine.shared),
            Work::Job(Job::Close(1))
        ));
        assert!(matches!(next_work(&engine.shared), Work::Render(_)));
    }
}
