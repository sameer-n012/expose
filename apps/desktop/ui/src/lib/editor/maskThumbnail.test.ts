import { describe, expect, it } from 'vitest';
import { thumbnailBuffer } from './fakeStrokeApi';
import { decodeMaskThumbnail, thumbnailSizeFor } from './maskThumbnail';

describe('decodeMaskThumbnail', () => {
  it('expands gray bytes to opaque RGBA, rows top first', () => {
    const buffer = thumbnailBuffer(2, 1, 0);
    new Uint8Array(buffer, 8)[1] = 200;
    const t = decodeMaskThumbnail(buffer);
    expect([t.width, t.height]).toEqual([2, 1]);
    expect(Array.from(t.data)).toEqual([0, 0, 0, 255, 200, 200, 200, 255]);
  });

  it('throws on a short buffer, a zero size, or a wrong pixel count', () => {
    expect(() => decodeMaskThumbnail(new ArrayBuffer(4))).toThrow(/too short/);
    expect(() => decodeMaskThumbnail(thumbnailBuffer(0, 3))).toThrow(/bad size/);
    expect(() => decodeMaskThumbnail(thumbnailBuffer(257, 1))).toThrow(/bad size/);
    expect(() => decodeMaskThumbnail(thumbnailBuffer(2, 2).slice(0, 11))).toThrow(/expected 4/);
  });
});

describe('thumbnailSizeFor', () => {
  it('scales by the pixel ratio and stays in 1..256', () => {
    expect(thumbnailSizeFor(1)).toBe(24);
    expect(thumbnailSizeFor(2)).toBe(48);
    expect(thumbnailSizeFor(100)).toBe(256);
    expect(thumbnailSizeFor(0)).toBe(24);
  });
});
