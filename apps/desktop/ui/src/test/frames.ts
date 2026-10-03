/** Test helpers: preview frame bodies in the shell format. */
import { FRAME_HEADER_BYTES, FRAME_MAGIC } from '../lib/develop/frame';

/** Makes a frame body with the given header fields; pixel bytes are `fill`. */
export function makeFrameBody(
  width: number,
  height: number,
  seq = 0,
  options: { queueUs?: number; renderUs?: number; fill?: number; magic?: number } = {},
): ArrayBuffer {
  const buffer = new ArrayBuffer(FRAME_HEADER_BYTES + width * height * 4);
  const view = new DataView(buffer);
  view.setUint32(0, options.magic ?? FRAME_MAGIC, true);
  view.setUint32(4, width, true);
  view.setUint32(8, height, true);
  view.setUint32(12, seq, true);
  view.setUint32(16, options.queueUs ?? 0, true);
  view.setUint32(20, options.renderUs ?? 0, true);
  new Uint8Array(buffer, FRAME_HEADER_BYTES).fill(options.fill ?? 0);
  return buffer;
}
