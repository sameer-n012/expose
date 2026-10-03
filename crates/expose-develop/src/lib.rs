//! Develop pipeline for Expose.
//!
//! # Responsibility
//!
//! This crate contains the non-destructive develop pipeline. It changes edit
//! parameters into a render graph. The original image never changes.
//!
//! # Dependency rules
//!
//! - This crate can depend on `expose-core` and `expose-gpu`.
//! - Do not use wgpu directly. Use the API of `expose-gpu`.

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::CRATE_NAME;

    #[test]
    fn crate_name_is_correct() {
        assert_eq!(CRATE_NAME, "expose-develop");
    }
}
