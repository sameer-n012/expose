import { describe, expect, it } from 'vitest';
import { MAX_FLOOD_TOLERANCE, clampTolerance, defaultBucket, defaultWand, fillEdit, magicWandEdit } from './flood';
import { modeFromModifiers } from './selection';

describe('clampTolerance', () => {
  it('rounds and limits to 0..=255', () => {
    expect(clampTolerance(-5)).toBe(0);
    expect(clampTolerance(12.6)).toBe(13);
    expect(clampTolerance(999)).toBe(MAX_FLOOD_TOLERANCE);
    expect(clampTolerance(Number.NaN)).toBe(0);
  });
});

describe('defaults', () => {
  it('match the tool description', () => {
    expect(defaultWand()).toEqual({ tolerance: 32, contiguous: true, antiAlias: true, sampleAllLayers: false });
    expect(defaultBucket()).toEqual({ ...defaultWand(), opacity: 1, blendMode: 'normal' });
  });
});

describe('magicWandEdit', () => {
  it('builds the contract fields', () => {
    const edit = magicWandEdit(3, 4, modeFromModifiers({ shiftKey: true, altKey: false }), { ...defaultWand(), tolerance: 300 }, null);
    expect(edit).toEqual({
      type: 'magicWand',
      x: 3,
      y: 4,
      mode: 'add',
      tolerance: 255,
      contiguous: true,
      antiAlias: true,
      sampleAllLayers: false,
      layerId: null,
    });
  });
});

describe('fillEdit', () => {
  it('builds the contract fields and limits opacity', () => {
    const edit = fillEdit(7, 1, 2, [1, 0.5, 0], { ...defaultBucket(), opacity: 1.5, blendMode: 'multiply', contiguous: false });
    expect(edit).toEqual({
      type: 'fill',
      layerId: 7,
      x: 1,
      y: 2,
      color: [1, 0.5, 0],
      opacity: 1,
      blendMode: 'multiply',
      tolerance: 32,
      contiguous: false,
      antiAlias: true,
      sampleAllLayers: false,
    });
  });
});
