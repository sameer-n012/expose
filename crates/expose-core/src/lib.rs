//! Core types for Expose.
//!
//! # Responsibility
//!
//! This crate contains pixel buffers, color math, and edit parameter types.
//! It does no I/O and no GPU work.
//!
//! # Modules
//!
//! - [`color`]: matrices, primaries, white points, transfer functions, and
//!   color spaces.
//! - [`error`]: the [`CoreError`] type.
//!
//! # Dependency rules
//!
//! - This crate must not depend on any other workspace crate.
//! - All other crates can depend on this crate.
//! - Do not call native-only APIs (file system, threads, clock) here. Use
//!   traits for platform services, so that a WebAssembly build can supply
//!   its own implementation.

pub mod color;
pub mod error;

pub use error::{CoreError, CoreResult};
