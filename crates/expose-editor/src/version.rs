//! Change stamps for documents and layer content.
//!
//! A stamp is a number from one process-wide counter. Each stamp is used
//! once, so a stamp identifies one state:
//!
//! - [`Document::revision`](crate::Document::revision) gets a new stamp at
//!   each change of the document.
//! - Each layer has a pixel version and a mask version
//!   ([`Layer::pixel_version`](crate::Layer::pixel_version),
//!   [`Layer::mask_version`](crate::Layer::mask_version)). They get a new
//!   stamp when the pixel values or the mask values change.
//!
//! Because the counter is process-wide, two different documents never have
//! the same stamp for different content. A cache (for example, the layer
//! texture cache of [`GpuCompositor`](crate::GpuCompositor)) can thus keep
//! its entries when the caller changes to a different document or a clone.
//! A clone has the same stamps as the original, which is correct because
//! the content is the same.
//!
//! An atomic counter is not a thread: it compiles for WebAssembly.

use std::sync::atomic::{AtomicU64, Ordering};

/// The next stamp. Stamp 0 is never used.
static NEXT_STAMP: AtomicU64 = AtomicU64::new(1);

/// Returns a new stamp. Each call returns a larger value than all earlier
/// calls in this process.
pub(crate) fn next_stamp() -> u64 {
    // Only uniqueness is necessary, so relaxed order is sufficient. 2^64
    // stamps do not occur in practice (at one billion each second, the
    // counter wraps after more than 500 years).
    NEXT_STAMP.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::next_stamp;

    #[test]
    fn stamps_increase() {
        let a = next_stamp();
        let b = next_stamp();
        assert!(b > a);
        assert!(a > 0);
    }
}
