//! Image decode and encode for Expose.
//!
//! # Responsibility
//!
//! This crate decodes RAW files and standard image formats. It also encodes
//! standard image formats.
//!
//! # Dependency rules
//!
//! - This crate can depend on `expose-core` only.
//! - Use pure Rust decoders, so that this crate compiles to WebAssembly.
//!   Do not add C libraries or GPL/LGPL code without operator approval.

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::CRATE_NAME;

    #[test]
    fn crate_name_is_correct() {
        assert_eq!(CRATE_NAME, "expose-raw");
    }
}
