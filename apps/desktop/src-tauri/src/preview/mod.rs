//! The develop preview: GPU render of preview frames and their transport
//! to the UI.
//!
//! # Data path of one frame
//!
//! 1. The UI sends the develop settings and the target frame long edge
//!    (`preview_render` command). It keeps up to two requests in flight,
//!    so that the render of the next frame overlaps the transfer of the
//!    current frame.
//! 2. The [`engine`] render thread runs the develop kernel on the GPU
//!    preview. The frame long edge is [`engine::DEFAULT_LONG_EDGE`] or the
//!    smaller target long edge; with a crop, the develop input is larger
//!    by the crop factor (see [`engine::input_long_edge`]).
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
    CROP_BASE_LONG_EDGE, DEFAULT_LONG_EDGE, Frame, FrameFormat, INPUT_STEP, MAX_LONG_EDGE,
    MIN_LONG_EDGE, OpenRequest, PreviewEngine, PreviewError, RenderRequest, SessionId, SessionInfo,
    effective_long_edge, input_long_edge, test_pattern,
};
pub use frame::{FRAME_HEADER_BYTES, FRAME_MAGIC, FrameHeader, FrameTimings};
