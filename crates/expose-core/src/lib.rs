//! Core types for Expose.
//!
//! # Responsibility
//!
//! This crate contains pixel buffers, color math, and edit parameter types.
//! It does no I/O and no GPU work.
//!
//! # Modules
//!
//! - [`buffer`]: the working image (linear Rec. 2020 RGBA `f32`), 8-bit and
//!   16-bit I/O images, rectangles, and conversion between them.
//! - [`color`]: matrices, primaries, white points, transfer functions, and
//!   color spaces.
//! - [`develop`]: non-destructive develop parameter types (no processing).
//! - [`error`]: the [`CoreError`] type.
//!
//! # Dependency rules
//!
//! - This crate must not depend on any other workspace crate.
//! - All other crates can depend on this crate.
//! - Do not call native-only APIs (file system, threads, clock) here. Use
//!   traits for platform services, so that a WebAssembly build can supply
//!   its own implementation.

pub mod buffer;
pub mod color;
pub mod develop;
pub mod error;

pub use error::{CoreError, CoreResult};
