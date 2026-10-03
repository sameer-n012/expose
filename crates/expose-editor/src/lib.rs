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
//! - [`composite`]: the CPU reference compositor.
//! - [`history`]: undo and redo.
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
//! - This crate can depend on `expose-core` only.
//! - Do not use wgpu directly. GPU work goes through `expose-gpu` when a
//!   later decision allows that dependency.
//! - No file system, threads, or clock (the crate must build for
//!   WebAssembly).

// The color math uses the short names of the published formulas (for
// example `cb`, `cs`, `l`, `n`, `x` in the W3C blending specification), so
// that the code is easy to compare with the source.
#![allow(clippy::many_single_char_names, clippy::similar_names)]

pub mod adjust;
pub mod blend;
pub mod composite;
pub mod document;
pub mod error;
pub mod history;
pub mod layer;
pub mod ops;

pub use adjust::{
    Adjustment, BrightnessContrast, Curves, HueSaturation, Levels, LevelsChannel, MonotoneCubic,
    PreparedAdjustment,
};
pub use blend::{BlendMode, BlendSpace, RangeHandling};
pub use composite::{composite, composite_straight, to_straight_alpha};
pub use document::{
    Background, BitDepth, ColorMode, Document, DocumentSpec, MAX_DIMENSION, MAX_PIXELS,
};
pub use error::{EditorError, EditorResult};
pub use history::{History, HistoryEntry};
pub use layer::{
    Group, GroupIsolation, Layer, LayerContent, LayerId, LayerKind, LayerProps, LockKind, Locks,
    Mask, MaskFill, MaskImage, PixelLayer, Placement,
};
pub use ops::Edit;

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
