//! Layer-based editor for Expose.
//!
//! # Responsibility
//!
//! This crate contains the layered document model, the editing tools, and
//! the edit history.
//!
//! # Dependency rules
//!
//! - This crate can depend on `expose-core` only.
//! - Do not use wgpu directly. GPU work goes through `expose-gpu` when a
//!   later decision allows that dependency.

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
