/**
 * Stroke calls for the fake `EditorApi` objects of the tests. Only tests
 * import this file. A test that checks strokes replaces the calls.
 */
import { vi } from 'vitest';
import type { DocumentInfo, LayerBounds, SampledColor } from './api';

/** A mask thumbnail answer: width, height (u32 little-endian), then `width * height` bytes of `value`. */
export function thumbnailBuffer(width: number, height: number, value = 255): ArrayBuffer {
  const buffer = new ArrayBuffer(8 + width * height);
  const view = new DataView(buffer);
  view.setUint32(0, width, true);
  view.setUint32(4, height, true);
  new Uint8Array(buffer, 8).fill(value);
  return buffer;
}

export function strokeStubs() {
  return {
    maskThumbnail: vi.fn(async (..._args: unknown[]): Promise<ArrayBuffer> => thumbnailBuffer(2, 2)),
    layerBounds: vi.fn(async (..._args: unknown[]): Promise<LayerBounds | null> => ({ x: 0, y: 0, width: 100, height: 80 })),
    sampleColor: vi.fn(async (..._args: unknown[]): Promise<SampledColor> => ({ color: '#336699', alpha: 1 })),
    strokeBegin: vi.fn(async (..._args: unknown[]) => 1),
    strokePoints: vi.fn(async (..._args: unknown[]) => null),
    strokeEnd: vi.fn(async (..._args: unknown[]): Promise<DocumentInfo> => {
      throw new Error('strokeEnd is not faked.');
    }),
    strokeCancel: vi.fn(async (..._args: unknown[]): Promise<DocumentInfo> => {
      throw new Error('strokeCancel is not faked.');
    }),
  };
}
