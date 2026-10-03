import { describe, expect, it } from 'vitest';
import { makeFrameBody } from '../../test/frames';
import { FRAME_HEADER_BYTES, FRAME_MAGIC, FrameFormatError, parseFrame } from './frame';

describe('parseFrame', () => {
  it('reads the header and gives a pixel view with no copy', () => {
    const body = makeFrameBody(3, 2, 7, { queueUs: 120, renderUs: 4500, fill: 9 });
    const frame = parseFrame(body);
    expect(frame).toMatchObject({ width: 3, height: 2, seq: 7, queueUs: 120, renderUs: 4500 });
    expect(frame.pixels.length).toBe(3 * 2 * 4);
    expect(frame.pixels.buffer).toBe(body);
    expect(frame.pixels.byteOffset).toBe(FRAME_HEADER_BYTES);
    expect(Array.from(frame.pixels.slice(0, 4))).toEqual([9, 9, 9, 9]);
  });

  it('matches the magic of the shell ("EXF1")', () => {
    const bytes = new Uint8Array(4);
    new DataView(bytes.buffer).setUint32(0, FRAME_MAGIC, true);
    expect(String.fromCharCode(...bytes)).toBe('EXF1');
  });

  it('rejects short bodies, a wrong magic, and a wrong length', () => {
    expect(() => parseFrame(new ArrayBuffer(10))).toThrow(FrameFormatError);
    expect(() => parseFrame(makeFrameBody(2, 2, 0, { magic: 0x12345678 }))).toThrow(/magic/);
    const truncated = makeFrameBody(4, 4).slice(0, FRAME_HEADER_BYTES + 10);
    expect(() => parseFrame(truncated)).toThrow(/expected/);
    expect(() => parseFrame(makeFrameBody(0, 0))).toThrow(FrameFormatError);
  });
});
