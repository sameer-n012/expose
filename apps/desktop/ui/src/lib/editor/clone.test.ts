import { describe, expect, it } from 'vitest';
import { EMPTY_CLONE_STATE, afterStroke, cloneTarget, isCloneTool, sourceMarker, strokeOffset, withSource } from './clone';

describe('strokeOffset', () => {
  it('gives null without a source point', () => {
    expect(strokeOffset(EMPTY_CLONE_STATE, { x: 5, y: 5 }, true)).toBeNull();
  });

  it('rounds source minus start and stores it', () => {
    const result = strokeOffset(withSource({ x: 100.4, y: 50.6 }), { x: 10, y: 20 }, true);
    expect(result?.offset).toEqual([90, 31]);
    expect(result?.state.offset).toEqual([90, 31]);
  });

  it('aligned: reuses the stored offset at a new start', () => {
    const first = strokeOffset(withSource({ x: 100, y: 100 }), { x: 10, y: 10 }, true);
    const second = first === null ? null : strokeOffset(first.state, { x: 200, y: 200 }, true);
    expect(second?.offset).toEqual([90, 90]);
  });

  it('not aligned: starts again at the source point', () => {
    const first = strokeOffset(withSource({ x: 100, y: 100 }), { x: 10, y: 10 }, false);
    expect(first).not.toBeNull();
    const after = afterStroke(first?.state ?? EMPTY_CLONE_STATE, false);
    expect(after.offset).toBeNull();
    expect(strokeOffset(after, { x: 20, y: 30 }, false)?.offset).toEqual([80, 70]);
  });

  it('a new source point clears the stored offset', () => {
    const first = strokeOffset(withSource({ x: 100, y: 100 }), { x: 10, y: 10 }, true);
    const reset = withSource({ x: 0, y: 0 });
    expect(reset.offset).toBeNull();
    expect(strokeOffset(reset, { x: 10, y: 10 }, true)?.offset).toEqual([-10, -10]);
    expect(first?.offset).toEqual([90, 90]);
  });

  it('aligned: afterStroke keeps the offset', () => {
    const state = { source: { x: 1, y: 2 }, offset: [3, 4] as const };
    expect(afterStroke(state, true)).toBe(state);
  });
});

describe('sourceMarker', () => {
  it('is null without a source', () => {
    expect(sourceMarker(EMPTY_CLONE_STATE, { x: 1, y: 1 })).toBeNull();
  });

  it('is the source point without an offset or a pointer', () => {
    const state = withSource({ x: 7, y: 8 });
    expect(sourceMarker(state, { x: 1, y: 1 })).toEqual({ x: 7, y: 8 });
    expect(sourceMarker({ ...state, offset: [3, 4] }, null)).toEqual({ x: 7, y: 8 });
  });

  it('is pointer plus offset when both are known', () => {
    expect(sourceMarker({ source: { x: 7, y: 8 }, offset: [3, -4] }, { x: 10, y: 10 })).toEqual({ x: 13, y: 6 });
  });
});

describe('cloneTarget', () => {
  it('maps the tools to the target types', () => {
    expect(cloneTarget('cloneStamp', [1, 2], false)).toEqual({ type: 'clone', offset: [1, 2], sampleAllLayers: false });
    expect(cloneTarget('healingBrush', [1, 2], true)).toEqual({ type: 'heal', offset: [1, 2], sampleAllLayers: true });
    expect(isCloneTool('healingBrush')).toBe(true);
    expect(isCloneTool('brush')).toBe(false);
  });
});
