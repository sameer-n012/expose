//! The develop preview: GPU render of preview frames and their transport
//! to the UI.
//!
//! # Data path of one frame
//!
//! 1. The UI sends the develop settings (`preview_render` command).
//! 2. The [`engine`] render thread runs the develop kernel on the GPU
//!    preview (long edge [`engine::DEFAULT_LONG_EDGE`]).
//! 3. The display encode kernel converts the result to 8-bit RGBA in the
//!    display color space (sRGB or Display P3), on the GPU.
//! 4. The thread reads the RGBA8 pixels back (4 bytes per pixel) into a
//!    buffer after a 32-byte [`frame`] header.
//! 5. The command returns the buffer as a raw binary IPC response
//!    (`tauri::ipc::Response`). The UI gets an `ArrayBuffer` and puts the
//!    pixels into a canvas with `putImageData`.
//!
//! The `preview` URI scheme ([`crate::preview_protocol`]) serves the same
//! frames, and also JPEG and PNG frames, for the transport benchmark.

pub mod engine;
pub mod frame;

pub use engine::{
    DEFAULT_LONG_EDGE, Frame, FrameFormat, OpenRequest, PreviewEngine, PreviewError, RenderRequest,
    SessionId, SessionInfo, test_pattern,
};
pub use frame::{FRAME_HEADER_BYTES, FRAME_MAGIC, FrameHeader, FrameTimings};
