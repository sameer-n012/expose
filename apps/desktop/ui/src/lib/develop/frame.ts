/**
 * Parser of the binary preview frame of the shell.
 *
 * Layout (all numbers little-endian u32; see
 * `src-tauri/src/preview/frame.rs`, keep the two in sync):
 *
 * | Offset | Content                                     |
 * | ------ | ------------------------------------------- |
 * | 0      | Magic "EXF1"                                |
 * | 4      | Width in pixels                             |
 * | 8      | Height in pixels                            |
 * | 12     | Frame sequence number in the session        |
 * | 16     | Queue time in microseconds                  |
 * | 20     | Render time in microseconds                 |
 * | 24     | Reserved (8 bytes)                          |
 * | 32     | RGBA8 pixels, row-major, straight alpha     |
 */

/** The size of the frame header in bytes. */
export const FRAME_HEADER_BYTES = 32;

/** The magic number: the bytes "EXF1" read as a little-endian u32. */
export const FRAME_MAGIC = 0x31465845;

/** A parsed frame. `pixels` is a view into the received buffer (no copy). */
export interface Frame {
  readonly width: number;
  readonly height: number;
  readonly seq: number;
  /** Time from the request to the render start in the shell, in microseconds. */
  readonly queueUs: number;
  /** Render time in the shell (develop, display encode, read-back), in microseconds. */
  readonly renderUs: number;
  /** RGBA8 pixels, `width * height * 4` bytes. */
  readonly pixels: Uint8ClampedArray<ArrayBuffer>;
}

/** The frame body is not valid. This is a bug in the shell or the transport. */
export class FrameFormatError extends Error {
  override readonly name = 'FrameFormatError';
}

/** Parses a frame body. Throws `FrameFormatError` if it is not valid. */
export function parseFrame(buffer: ArrayBuffer): Frame {
  if (buffer.byteLength < FRAME_HEADER_BYTES) {
    throw new FrameFormatError(`The frame has ${buffer.byteLength} bytes; the header needs ${FRAME_HEADER_BYTES}.`);
  }
  const view = new DataView(buffer, 0, FRAME_HEADER_BYTES);
  const magic = view.getUint32(0, true);
  if (magic !== FRAME_MAGIC) {
    throw new FrameFormatError(`The frame magic is 0x${magic.toString(16)}, not "EXF1".`);
  }
  const width = view.getUint32(4, true);
  const height = view.getUint32(8, true);
  const expected = FRAME_HEADER_BYTES + width * height * 4;
  if (width === 0 || height === 0 || buffer.byteLength !== expected) {
    throw new FrameFormatError(
      `The frame is ${width}x${height} with ${buffer.byteLength} bytes; expected ${expected} bytes.`,
    );
  }
  return {
    width,
    height,
    seq: view.getUint32(12, true),
    queueUs: view.getUint32(16, true),
    renderUs: view.getUint32(20, true),
    pixels: new Uint8ClampedArray(buffer, FRAME_HEADER_BYTES, width * height * 4),
  };
}
