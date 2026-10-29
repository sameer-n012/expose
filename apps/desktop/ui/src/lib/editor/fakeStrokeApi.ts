/**
 * Stroke calls for the fake `EditorApi` objects of the tests. Only tests
 * import this file. A test that checks strokes replaces the calls.
 */
import { vi } from 'vitest';
import type { DocumentInfo, SampledColor } from './api';

export function strokeStubs() {
  return {
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
