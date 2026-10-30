//! Error type for `expose-editor`.

use std::fmt;
use std::sync::Arc;

use expose_core::CoreError;
use expose_gpu::GpuError;
use thiserror::Error;

use crate::file::FileError;
use crate::layer::{LayerId, LockKind};

/// All errors that `expose-editor` can return.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum EditorError {
    /// An error from `expose-core` (for example, a bad image size).
    #[error(transparent)]
    Core(#[from] CoreError),

    /// A document or layer size is larger than the permitted limit.
    #[error("size {width}x{height} is too large: {reason}")]
    TooLarge {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// The limit that the size breaks.
        reason: &'static str,
    },

    /// A numeric value is outside its permitted range.
    #[error("{name} = {value} is outside the range {min}..={max}")]
    OutOfRange {
        /// Name of the value.
        name: &'static str,
        /// The value.
        value: f64,
        /// Lowest permitted value.
        min: f64,
        /// Highest permitted value.
        max: f64,
    },

    /// A numeric value is NaN or infinite.
    #[error("{name} is not a finite number")]
    NonFinite {
        /// Name of the value.
        name: &'static str,
    },

    /// No layer has this ID.
    #[error("layer {0} does not exist")]
    LayerNotFound(LayerId),

    /// The operation needs a group, but the layer is not a group.
    #[error("layer {0} is not a group")]
    NotAGroup(LayerId),

    /// The operation needs a pixel layer, but the layer is of another kind.
    #[error("layer {0} is not a pixel layer")]
    NotAPixelLayer(LayerId),

    /// The operation needs an adjustment layer, but the layer is of another kind.
    #[error("layer {0} is not an adjustment layer")]
    NotAnAdjustmentLayer(LayerId),

    /// The layer has no mask.
    #[error("layer {0} has no mask")]
    NoMask(LayerId),

    /// A move would put a group inside itself or inside one of its descendants.
    #[error("cannot move layer {0} into itself or into one of its descendants")]
    Cycle(LayerId),

    /// An insert position is outside the list of children.
    #[error("index {index} is outside the range 0..={len}")]
    IndexOutOfRange {
        /// The index that the caller supplied.
        index: usize,
        /// The number of children (the highest permitted insert index).
        len: usize,
    },

    /// A lock on the layer (or on a parent group) prevents the operation.
    #[error("layer {id} has a {lock} lock")]
    Locked {
        /// The layer that the caller wants to change.
        id: LayerId,
        /// The lock that prevents the change.
        lock: LockKind,
    },

    /// A pixel write would change alpha on a layer with a transparency lock.
    #[error("layer {0} has a transparency lock and the write changes alpha")]
    TransparencyLocked(LayerId),

    /// A GPU operation failed (see [`crate::gpu`]).
    #[error("GPU operation failed: {0}")]
    Gpu(SharedGpuError),

    /// A rectangle is empty or is not fully inside the target buffer.
    #[error("rectangle {x},{y} {width}x{height} is empty or outside the target")]
    InvalidRect {
        /// Left edge.
        x: u32,
        /// Top edge.
        y: u32,
        /// Width.
        width: u32,
        /// Height.
        height: u32,
    },

    /// The data length does not agree with the rectangle size.
    #[error("data length {actual} does not match the expected length {expected}")]
    LengthMismatch {
        /// Number of values that the rectangle needs.
        expected: usize,
        /// Number of values that the caller supplied.
        actual: usize,
    },

    /// A layer offset would go outside the `i32` range.
    #[error("layer offset overflows the i32 range")]
    OffsetOverflow,

    /// A curve has a structural problem.
    #[error("invalid curve for channel {channel}: {reason}")]
    InvalidCurve {
        /// Curve channel name (`master`, `red`, `green`, or `blue`).
        channel: &'static str,
        /// Description of the problem.
        reason: &'static str,
    },

    /// A layer ID in an inserted subtree is already in the document.
    #[error("layer {0} is already in the document")]
    DuplicateId(LayerId),

    /// The history has a value that is not valid (for example, zero steps).
    #[error("invalid history setting: {0}")]
    InvalidHistory(&'static str),

    /// A view specification is not valid (for example, a scale that is not
    /// a positive finite number, or an output size of 0).
    #[error("invalid view: {0}")]
    InvalidView(&'static str),

    /// The operation needs a selection, but the document has no selection
    /// (see [`crate::selection`]).
    #[error("the document has no selection")]
    NoSelection,

    /// The layer of a brush stroke changed outside the stroke (for example,
    /// an undo, a move, or a resize during the stroke). The stroke cannot
    /// continue, because the original values that it keeps are not correct
    /// now (see [`crate::brush`]).
    #[error("layer {0} changed outside the brush stroke")]
    StrokeConflict(LayerId),

    /// A brush setting is not valid for the stroke target (for example, a
    /// blend mode other than normal for an eraser or a mask stroke, see
    /// [`crate::brush`]).
    #[error("invalid brush setting: {0}")]
    InvalidBrush(&'static str),

    /// A fill setting is not valid (for example, the dissolve blend mode
    /// for the paint bucket, see [`crate::flood`]).
    #[error("invalid fill setting: {0}")]
    InvalidFill(&'static str),

    /// A gradient setting is not valid (for example, fewer than two stops,
    /// stops that are not in order, a line that is too short, or a blend
    /// mode other than normal on a mask, see [`crate::gradient`]).
    #[error("invalid gradient: {0}")]
    InvalidGradient(&'static str),

    /// A document file cannot be read or written (see [`crate::file`]).
    #[error("document file: {0}")]
    File(#[from] FileError),
}

/// Result type with [`EditorError`].
pub type EditorResult<T> = Result<T, EditorError>;

/// A shared [`GpuError`].
///
/// [`EditorError`] is `Clone` and `PartialEq`, but [`GpuError`] is not.
/// This wrapper keeps the error in an [`Arc`]. Two values are equal only if
/// they hold the same error object.
#[derive(Debug, Clone)]
pub struct SharedGpuError(Arc<GpuError>);

impl SharedGpuError {
    /// Returns the GPU error.
    pub fn get(&self) -> &GpuError {
        &self.0
    }
}

impl PartialEq for SharedGpuError {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Display for SharedGpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for SharedGpuError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

impl From<GpuError> for EditorError {
    fn from(error: GpuError) -> Self {
        Self::Gpu(SharedGpuError(Arc::new(error)))
    }
}

/// Returns an error if `value` is not finite or is outside `min..=max`.
pub(crate) fn check_range(name: &'static str, value: f32, min: f32, max: f32) -> EditorResult<()> {
    if !value.is_finite() {
        return Err(EditorError::NonFinite { name });
    }
    if value < min || value > max {
        return Err(EditorError::OutOfRange {
            name,
            value: f64::from(value),
            min: f64::from(min),
            max: f64::from(max),
        });
    }
    Ok(())
}
