//! The binary frame format that goes from the preview engine to the UI.
//!
//! An RGBA8 frame body is a fixed header and then the pixels:
//!
//! | Offset | Size | Content                                              |
//! | ------ | ---- | ---------------------------------------------------- |
//! | 0      | 4    | Magic [`FRAME_MAGIC`] (`"EXF1"` in byte order)       |
//! | 4      | 4    | Width in pixels                                      |
//! | 8      | 4    | Height in pixels                                     |
//! | 12     | 4    | Frame sequence number in the session                 |
//! | 16     | 4    | Queue time in microseconds (request to render start) |
//! | 20     | 4    | Render time in microseconds (develop to pixels)      |
//! | 24     | 8    | Reserved, zero                                       |
//! | 32     | n    | RGBA8 pixels, row-major, no row padding              |
//!
//! All numbers are little-endian `u32`. `n = width * height * 4`. The
//! pixels have straight alpha and are in the display color space of the
//! session, so the UI can give them to `ImageData` with no conversion.
//!
//! The UI parser is `ui/src/lib/develop/frame.ts`. Keep the two in sync.

use std::time::Duration;

/// The first four bytes of a frame: `E`, `X`, `F`, `1`.
pub const FRAME_MAGIC: u32 = u32::from_le_bytes(*b"EXF1");

/// The size of the frame header in bytes.
pub const FRAME_HEADER_BYTES: usize = 32;

/// The timing of one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameTimings {
    /// The time from the request to the start of the render.
    pub queue: Duration,
    /// The time from the start of the render to the finished body
    /// (develop, display encode, read-back, and a file encode if any).
    pub render: Duration,
}

/// The header fields of an RGBA8 frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Sequence number of the frame in its session.
    pub seq: u32,
    /// Stage timings.
    pub timings: FrameTimings,
}

impl FrameHeader {
    /// Writes the header into the first [`FRAME_HEADER_BYTES`] of `body`.
    ///
    /// Returns `false` and writes nothing if `body` is shorter than the
    /// header.
    pub fn write(&self, body: &mut [u8]) -> bool {
        let Some(header) = body.get_mut(..FRAME_HEADER_BYTES) else {
            return false;
        };
        let fields = [
            FRAME_MAGIC,
            self.width,
            self.height,
            self.seq,
            micros(self.timings.queue),
            micros(self.timings.render),
            0,
            0,
        ];
        for (chunk, value) in header.chunks_exact_mut(4).zip(fields) {
            chunk.copy_from_slice(&value.to_le_bytes());
        }
        true
    }

    /// Reads a header from the start of `body`. Returns `None` if the body
    /// is too short or the magic is wrong. For tests and tools; the UI has
    /// its own parser.
    pub fn read(body: &[u8]) -> Option<Self> {
        let header = body.get(..FRAME_HEADER_BYTES)?;
        let mut fields = [0_u32; 8];
        for (field, chunk) in fields.iter_mut().zip(header.chunks_exact(4)) {
            let bytes: [u8; 4] = chunk.try_into().ok()?;
            *field = u32::from_le_bytes(bytes);
        }
        if fields[0] != FRAME_MAGIC {
            return None;
        }
        Some(Self {
            width: fields[1],
            height: fields[2],
            seq: fields[3],
            timings: FrameTimings {
                queue: Duration::from_micros(u64::from(fields[4])),
                render: Duration::from_micros(u64::from(fields[5])),
            },
        })
    }

    /// The text form for the `X-Expose-Frame` HTTP header of the `preview`
    /// URI scheme: `width,height,seq,queue_us,render_us`.
    pub fn to_header_value(&self) -> String {
        format!(
            "{},{},{},{},{}",
            self.width,
            self.height,
            self.seq,
            micros(self.timings.queue),
            micros(self.timings.render)
        )
    }
}

/// The body length of an RGBA8 frame of `width` x `height` pixels.
pub fn rgba8_body_len(width: u32, height: u32) -> usize {
    FRAME_HEADER_BYTES + width as usize * height as usize * 4
}

/// Microseconds, saturated to `u32::MAX` (about 71 minutes).
fn micros(duration: Duration) -> u32 {
    u32::try_from(duration.as_micros()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_is_exf1_in_byte_order() {
        assert_eq!(FRAME_MAGIC.to_le_bytes(), *b"EXF1");
    }

    #[test]
    fn header_round_trips() {
        let header = FrameHeader {
            width: 2560,
            height: 1707,
            seq: 42,
            timings: FrameTimings {
                queue: Duration::from_micros(150),
                render: Duration::from_micros(9_500),
            },
        };
        let mut body = vec![0xAA_u8; rgba8_body_len(2, 1)];
        assert!(header.write(&mut body));
        assert_eq!(&body[..4], b"EXF1");
        assert_eq!(&body[24..32], &[0; 8]);
        // The pixels stay as they are.
        assert_eq!(&body[FRAME_HEADER_BYTES..], &[0xAA; 8]);
        assert_eq!(FrameHeader::read(&body), Some(header));
        assert_eq!(header.to_header_value(), "2560,1707,42,150,9500");
    }

    #[test]
    fn short_or_foreign_bodies_are_rejected() {
        let header = FrameHeader {
            width: 1,
            height: 1,
            seq: 0,
            timings: FrameTimings::default(),
        };
        let mut short = [0_u8; FRAME_HEADER_BYTES - 1];
        assert!(!header.write(&mut short));
        assert_eq!(FrameHeader::read(&short), None);
        let foreign = [0_u8; FRAME_HEADER_BYTES];
        assert_eq!(FrameHeader::read(&foreign), None);
    }

    #[test]
    fn long_durations_saturate() {
        assert_eq!(micros(Duration::from_secs(10_000)), u32::MAX);
        assert_eq!(rgba8_body_len(2560, 1707), 32 + 2560 * 1707 * 4);
    }
}
