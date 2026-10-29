/** The mask thumbnail of a layer: the decoder and the size rule. */

/** A decoded thumbnail: RGBA pixels, rows top first. */
export interface MaskThumbnail {
  width: number;
  height: number;
  /** RGBA, `width * height * 4` bytes. The mask value is in R, G, and B. Alpha is 255. */
  data: Uint8ClampedArray<ArrayBuffer>;
}

/** The largest `maxSize` that the shell accepts. */
export const MAX_THUMBNAIL_SIZE = 256;

/** The chip size in CSS pixels that the thumbnail must fill. */
const CHIP_CSS_PX = 24;

/** The `maxSize` for the mask chip: the chip size times the device pixel ratio, rounded, 1..256. */
export function thumbnailSizeFor(devicePixelRatio: number): number {
  const ratio = Number.isFinite(devicePixelRatio) && devicePixelRatio > 0 ? devicePixelRatio : 1;
  return Math.min(MAX_THUMBNAIL_SIZE, Math.max(1, Math.round(CHIP_CSS_PX * ratio)));
}

/**
 * Decodes the binary answer of `editor_mask_thumbnail`: width u32 and height u32
 * (little-endian), then `width * height` gray bytes. Throws if the size does not fit.
 */
export function decodeMaskThumbnail(buffer: ArrayBuffer): MaskThumbnail {
  if (buffer.byteLength < 8) {
    throw new Error(`Mask thumbnail is too short: ${buffer.byteLength} bytes.`);
  }
  const view = new DataView(buffer);
  const width = view.getUint32(0, true);
  const height = view.getUint32(4, true);
  if (width === 0 || height === 0 || width > MAX_THUMBNAIL_SIZE || height > MAX_THUMBNAIL_SIZE) {
    throw new Error(`Mask thumbnail has a bad size: ${width}x${height}.`);
  }
  const count = width * height;
  if (buffer.byteLength !== 8 + count) {
    throw new Error(`Mask thumbnail has ${buffer.byteLength - 8} pixel bytes, expected ${count}.`);
  }
  const gray = new Uint8Array(buffer, 8, count);
  const data = new Uint8ClampedArray(count * 4);
  for (let i = 0; i < count; i += 1) {
    const value = gray[i] ?? 0;
    data[i * 4] = value;
    data[i * 4 + 1] = value;
    data[i * 4 + 2] = value;
    data[i * 4 + 3] = 255;
  }
  return { width, height, data };
}
