//! Core types for Expose.
//!
//! # Responsibility
//!
//! This crate contains pixel buffers, color math, and edit parameter types.
//! It does no I/O and no GPU work.
//!
//! # Dependency rules
//!
//! - This crate must not depend on any other workspace crate.
//! - All other crates can depend on this crate.
//! - Do not call native-only APIs (file system, threads, clock) here. Use
//!   traits for platform services, so that a WebAssembly build can supply
//!   its own implementation.

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::CRATE_NAME;

    #[test]
    fn crate_name_is_correct() {
        assert_eq!(CRATE_NAME, "expose-core");
    }
}
