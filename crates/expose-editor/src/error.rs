//! Error type for `expose-editor`.

use expose_core::CoreError;
use thiserror::Error;

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
}

/// Result type with [`EditorError`].
pub type EditorResult<T> = Result<T, EditorError>;

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
