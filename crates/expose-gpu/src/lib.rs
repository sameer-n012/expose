//! GPU processing for Expose.
//!
//! # Responsibility
//!
//! This crate contains the wgpu context, the shader pipelines, and the tiled
//! render graph.
//!
//! # Dependency rules
//!
//! - This crate can depend on `expose-core` only.
//! - This is the only crate that may use wgpu directly. Other crates use the
//!   API of this crate.
//! - Shaders are WGSL, so that they also run on WebGPU in a browser.

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::CRATE_NAME;

    #[test]
    fn crate_name_is_correct() {
        assert_eq!(CRATE_NAME, "expose-gpu");
    }
}
