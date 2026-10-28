//! The editor: open layered documents, their history, and the GPU render
//! of the canvas view (ADR 0013, ADR 0014).
//!
//! # Data path
//!
//! 1. `editor_open` reads the photo path and the develop settings from the
//!    catalog, decodes the file, and runs the full develop on the CPU (as
//!    export does). [`session::build_document`] makes a document with one
//!    pixel layer "Background" that holds the photo. If the photo is the
//!    flattened image of an earlier save (a catalog link), the command reads
//!    the layered `.exdoc` file instead (ADR 0015). This work runs on a
//!    blocking thread, not on the render thread.
//! 2. The [`engine`] render thread keeps the open documents. It applies
//!    the edits, undo, and redo ([`session::EditSession`]), and renders
//!    the canvas view with one shared `GpuCompositor`.
//! 3. `editor_render` returns the view as an `EXF1` frame (see
//!    [`crate::preview::frame`]): 8-bit RGBA, straight alpha, display color
//!    space, transparent outside the canvas.
//! 4. `editor_save` takes a [`SaveSnapshot`] on the render thread (a GPU
//!    composite and a copy of the document), and then writes the files and
//!    updates the catalog on a blocking thread ([`save`]).
//!
//! # Memory
//!
//! A document holds the photo at full size in `f32` RGBA (16 bytes per
//! pixel, about 380 MB for 24 MP). The compositor caches each pixel layer
//! on the GPU with the same size, and the full composite and its pyramid
//! (ADR 0014). The history keeps the removed layers of delete steps.

pub mod dto;
pub mod engine;
pub mod outline;
pub mod save;
pub mod session;

use expose_develop::DevelopError;
use expose_editor::EditorError;
use expose_gpu::GpuError;

pub use dto::{
    AdjustmentDto, BackgroundId, BlendModeId, CurvePointDto, DocumentId, DocumentInfo, EditDto,
    LayerInfo, LayerKindId, LevelsChannelDto, LocksDto, NewDocumentSpec, SaveResult,
    SelectionModeId, SelectionOutline, SelectionShapeDto,
};
pub use engine::{EditorEngine, MAX_VIEW_SIDE, RenderRequest, SaveGuard};
pub use outline::{MAX_OUTLINE_POINTS, selection_outline};
pub use save::{DocumentOrigin, SaveSnapshot, SavedFiles};
pub use session::{EditSession, HISTORY_STEPS, build_document};

use crate::error::{CommandError, ErrorKind};

/// An error of the editor engine.
#[derive(Debug, thiserror::Error)]
pub enum EditorEngineError {
    /// A newer frame request for the same document replaced this request
    /// before it started. This is not a failure; the UI drops the request.
    #[error("a newer frame request replaced this request")]
    Superseded,
    /// The document is not open.
    #[error("editor document {0} is not open")]
    DocumentNotFound(DocumentId),
    /// The document has no layer with this ID.
    #[error("the document has no layer {0}")]
    LayerNotFound(u64),
    /// The sample step of a selection outline is 0.
    #[error("the outline step must be 1 or more")]
    InvalidStep,
    /// The layer has a mask already. The edit does not replace it.
    #[error("the layer {0} has a mask already")]
    MaskExists(u64),
    /// The requested view size is not valid.
    #[error("the view size {width}x{height} is not in 1..={MAX_VIEW_SIDE}")]
    InvalidViewSize {
        /// The requested width.
        width: u32,
        /// The requested height.
        height: u32,
    },
    /// An editor operation or the compositor failed.
    #[error(transparent)]
    Editor(#[from] EditorError),
    /// An undo or redo failed. The history restored the state before the
    /// step, but the failure is a bug.
    #[error("the history step failed: {0}")]
    History(#[source] EditorError),
    /// The GPU context cannot be made.
    #[error("the GPU is not available: {0}")]
    GpuUnavailable(#[source] GpuError),
    /// The display encode failed.
    #[error("cannot encode the view: {0}")]
    Display(#[from] DevelopError),
    /// A save of the document runs already.
    #[error("a save of editor document {0} is already running")]
    SaveInProgress(DocumentId),
    /// A state that must not occur (a bug).
    #[error("internal error: {0}")]
    Internal(&'static str),
    /// The render thread stopped or panicked.
    #[error("the editor engine stopped: {0}")]
    Stopped(String),
}

/// Returns the command error kind of an error of the editor crate.
fn editor_error_kind(error: &EditorError) -> ErrorKind {
    match error {
        EditorError::LayerNotFound(_) => ErrorKind::NotFound,
        EditorError::Gpu(_) | EditorError::InvalidHistory(_) => ErrorKind::Internal,
        EditorError::File(expose_editor::FileError::Io { .. }) => ErrorKind::Io,
        // Any other file error means the document file is damaged or not
        // supported.
        EditorError::File(_)
        | EditorError::Core(_)
        | EditorError::TooLarge { .. }
        | EditorError::OutOfRange { .. }
        | EditorError::NonFinite { .. }
        | EditorError::NotAGroup(_)
        | EditorError::NotAPixelLayer(_)
        | EditorError::NotAnAdjustmentLayer(_)
        | EditorError::NoMask(_)
        | EditorError::NoSelection
        | EditorError::Cycle(_)
        | EditorError::IndexOutOfRange { .. }
        | EditorError::Locked { .. }
        | EditorError::TransparencyLocked(_)
        | EditorError::InvalidRect { .. }
        | EditorError::LengthMismatch { .. }
        | EditorError::OffsetOverflow
        | EditorError::InvalidCurve { .. }
        | EditorError::DuplicateId(_)
        | EditorError::InvalidView(_) => ErrorKind::InvalidInput,
    }
}

impl From<EditorEngineError> for CommandError {
    fn from(error: EditorEngineError) -> Self {
        let kind = match &error {
            EditorEngineError::Superseded => ErrorKind::Superseded,
            EditorEngineError::DocumentNotFound(_) | EditorEngineError::LayerNotFound(_) => {
                ErrorKind::NotFound
            }
            EditorEngineError::InvalidViewSize { .. }
            | EditorEngineError::InvalidStep
            | EditorEngineError::MaskExists(_)
            | EditorEngineError::SaveInProgress(_) => ErrorKind::InvalidInput,
            EditorEngineError::Editor(inner) => editor_error_kind(inner),
            EditorEngineError::GpuUnavailable(_) => ErrorKind::Unavailable,
            EditorEngineError::History(_)
            | EditorEngineError::Display(_)
            | EditorEngineError::Internal(_)
            | EditorEngineError::Stopped(_) => ErrorKind::Internal,
        };
        Self::new(kind, error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use expose_editor::LayerId;

    use super::*;

    #[test]
    fn errors_map_to_command_kinds() {
        let kind = |e: EditorEngineError| CommandError::from(e).kind;
        assert_eq!(kind(EditorEngineError::Superseded), ErrorKind::Superseded);
        assert_eq!(
            CommandError::from(EditorEngineError::Superseded),
            CommandError::new(
                ErrorKind::Superseded,
                "a newer frame request replaced this request"
            )
        );
        assert_eq!(
            kind(EditorEngineError::DocumentNotFound(4)),
            ErrorKind::NotFound
        );
        assert_eq!(
            kind(EditorEngineError::LayerNotFound(4)),
            ErrorKind::NotFound
        );
        assert_eq!(
            kind(EditorEngineError::MaskExists(4)),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            kind(EditorEngineError::InvalidStep),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            kind(EditorEngineError::InvalidViewSize {
                width: 0,
                height: 1
            }),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            kind(EditorEngineError::Editor(EditorError::OutOfRange {
                name: "opacity",
                value: 2.0,
                min: 0.0,
                max: 1.0
            })),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            kind(EditorEngineError::Editor(EditorError::InvalidView("scale"))),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            kind(EditorEngineError::History(EditorError::InvalidHistory("x"))),
            ErrorKind::Internal
        );
        assert_eq!(kind(EditorEngineError::Internal("x")), ErrorKind::Internal);
        assert_eq!(
            kind(EditorEngineError::Stopped("x".into())),
            ErrorKind::Internal
        );
        // `LayerId` values cannot be made outside the editor crate, so the
        // not-found case of the crate uses a real document ID.
        let doc = expose_editor::Document::new(expose_editor::DocumentSpec::new(1, 1));
        let id: LayerId = doc.map_or_else(|e| panic!("{e}"), |d| d.layer_ids()[0]);
        assert_eq!(
            kind(EditorEngineError::Editor(EditorError::LayerNotFound(id))),
            ErrorKind::NotFound
        );
    }
}
