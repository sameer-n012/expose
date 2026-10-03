//! Photo library for Expose.
//!
//! # Responsibility
//!
//! This crate contains the library database, the preview cache, and the
//! sidecar files.
//!
//! # Dependency rules
//!
//! - This crate can depend on `expose-core` only.
//! - Put storage behind a trait. The desktop build uses SQLite. A web build
//!   can supply a browser storage backend.

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::CRATE_NAME;

    #[test]
    fn crate_name_is_correct() {
        assert_eq!(CRATE_NAME, "expose-catalog");
    }
}
