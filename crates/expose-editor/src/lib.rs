//! Layer-based editor for Expose.
//!
//! # Responsibility
//!
//! This crate contains the layered document model, the editing tools, and
//! the edit history. Now it has:
//!
//! - [`document`]: the [`Document`] (canvas settings and the layer tree).
//! - [`layer`]: layer types, properties, locks, and masks.
//! - [`ops`]: the operations that change a document, and the reversible
//!   [`Edit`] records that they return.
//! - [`blend`]: blend modes and the blending space.
//! - [`adjust`]: adjustment layer parameters and their apply functions.
//! - [`composite`](mod@composite): the CPU reference compositor.
//! - [`gpu`]: the GPU compositor ([`GpuCompositor`]). It gives the same
//!   result as the CPU reference, within float tolerance. It keeps a cache
//!   of layer textures, and renders zoomed and panned views of a cached
//!   full composite for the editor canvas
//!   ([`GpuCompositor::render_view`], [`ViewSpec`]).
//! - [`selection`]: the document selection (rectangle, ellipse, and
//!   polygon shapes, combine modes, feather, invert, mask from selection).
//!   [`Document::selection_version`] changes at each selection change.
//! - [`brush`]: brush strokes (paint, erase, mask paint) on pixel layers
//!   and masks ([`Stroke`]). One stroke gives one undo edit.
//! - [`flood`]: flood regions for the magic wand and the paint bucket
//!   ([`flood_region`], [`Document::select_region`],
//!   [`Document::fill_region`]).
//! - [`gradient`]: gradient fills (linear, radial, angle, reflected,
//!   diamond) on pixel layers and masks ([`Document::fill_gradient`]).
//! - [`history`]: undo and redo. [`History::push_merge`] makes one undo
//!   step from a gesture (for example, a slider drag).
//! - [`mod@file`]: the native layered document file (`.exdoc`):
//!   [`write_document`] and [`read_document`].
//! - [`version`]: change stamps. [`Document::revision`] changes at each
//!   change of the document; [`Layer::pixel_version`] and
//!   [`Layer::mask_version`] change when the values of a layer change.
//!   Caches use them to find out of date data.
//!
//! # Pixel format
//!
//! All pixel data is linear Rec. 2020 RGBA `f32` with **premultiplied**
//! alpha (ADR 0002). Use [`composite::to_straight_alpha`] for output.
//!
//! # Use
//!
//! ```
//! use expose_editor::{Document, DocumentSpec, History, composite};
//!
//! let mut doc = Document::new(DocumentSpec::new(64, 64))?;
//! let mut history = History::default();
//! let bg = doc.layers()[0].id();
//! let undo = doc.set_opacity(bg, 0.5)?;
//! history.push("Opacity", vec![undo]);
//! let image = composite(&doc, doc.bounds())?;
//! assert!((image.pixel(0, 0).unwrap()[3] - 0.5).abs() < 1e-6);
//! history.undo(&mut doc)?;
//! # Ok::<(), expose_editor::EditorError>(())
//! ```
//!
//! # Dependency rules
//!
//! - This crate depends on `expose-core` and `expose-gpu` only (of the
//!   workspace crates).
//! - Do not use wgpu directly. All GPU work goes through the `expose-gpu`
//!   kernel API.
//! - No file system, threads, or clock (the crate must build for
//!   WebAssembly). The GPU functions are `async`; the caller supplies the
//!   executor.

// The color math uses the short names of the published formulas (for
// example `cb`, `cs`, `l`, `n`, `x` in the W3C blending specification), so
// that the code is easy to compare with the source.
#![allow(clippy::many_single_char_names, clippy::similar_names)]

pub mod adjust;
pub mod blend;
pub mod brush;
mod changes;
pub mod composite;
pub mod document;
pub mod error;
pub mod file;
pub mod flood;
pub mod gpu;
pub mod gradient;
pub mod history;
pub mod layer;
pub mod ops;
pub mod selection;
pub mod version;

pub use adjust::{
    Adjustment, BrightnessContrast, Curves, HueSaturation, Levels, LevelsChannel, MonotoneCubic,
    PreparedAdjustment,
};
pub use blend::{BlendMode, BlendSpace, RangeHandling};
pub use brush::{
    BrushSettings, MAX_BRUSH_SIZE, MAX_BRUSH_SPACING, MAX_STROKE_COORD, MIN_BRUSH_SIZE,
    MIN_BRUSH_SPACING, MIN_DAB_STEP, STROKE_TILE_SIZE, Stroke, StrokePoint, StrokeTarget,
};
pub use composite::{composite, composite_straight, to_straight_alpha};
pub use document::{
    Background, BitDepth, ColorMode, Document, DocumentSpec, MAX_DIMENSION, MAX_PIXELS,
};
pub use error::{EditorError, EditorResult, SharedGpuError};
pub use file::{FILE_EXTENSION, FileError, read_document, write_document};
pub use flood::{FillSettings, FloodSample, FloodSettings, MAX_FLOOD_TOLERANCE, flood_region};
pub use gpu::{
    CURVE_TABLE_SIZE, GpuCompositor, MAX_VIEW_ORIGIN, MAX_VIEW_SCALE, MIN_VIEW_SCALE, ViewSpec,
};
pub use gradient::{
    GRADIENT_TABLE_STEPS, GradientShape, GradientSpec, GradientStop, GradientTarget,
    MAX_GRADIENT_COORD, MAX_GRADIENT_STOPS, MIN_GRADIENT_LENGTH,
};
pub use history::{History, HistoryEntry};
pub use layer::{
    Group, GroupIsolation, Layer, LayerContent, LayerId, LayerKind, LayerProps, LockKind, Locks,
    Mask, MaskFill, MaskImage, PixelLayer, Placement,
};
pub use ops::Edit;
pub use selection::{
    MAX_FEATHER_RADIUS, MAX_POLYGON_POINTS, MAX_SELECTION_COORD, SelectionMode, SelectionShape,
};

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::CRATE_NAME;

    #[test]
    fn crate_name_is_correct() {
        assert_eq!(CRATE_NAME, "expose-editor");
    }
}
