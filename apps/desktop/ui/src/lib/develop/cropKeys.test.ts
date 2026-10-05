import { describe, expect, it } from 'vitest';
import { applyCropKey, KEY_STEP, KEY_STEP_LARGE } from './cropKeys';
import { insideFrame, pixelAspect } from './cropGeometry';

const frame = { width: 300, height: 200, angle: 0 };
const rect = { left: 0.25, top: 0.25, width: 0.5, height: 0.5 };
const plain = { shiftKey: false, altKey: false };

describe('applyCropKey', () => {
  it('moves by one step, or a big step with Shift', () => {
    const right = applyCropKey(frame, rect, { key: 'ArrowRight', ...plain }, null);
    expect(right?.left).toBeCloseTo(0.25 + KEY_STEP, 9);
    const up = applyCropKey(frame, rect, { key: 'ArrowUp', shiftKey: true, altKey: false }, null);
    expect(up?.top).toBeCloseTo(0.25 - KEY_STEP_LARGE, 9);
    expect(up?.height).toBeCloseTo(0.5, 9);
  });

  it('stops at the edge of the rotated image', () => {
    const full = { left: 0.5, top: 0, width: 0.5, height: 1 };
    const moved = applyCropKey(frame, full, { key: 'ArrowRight', ...plain }, null);
    expect(moved?.left).toBeCloseTo(0.5, 6);
    const tilted = { ...frame, angle: 10 };
    const inside = { left: 0.4, top: 0.4, width: 0.2, height: 0.2 };
    let current = inside;
    for (let i = 0; i < 30; i++) {
      current = applyCropKey(tilted, current, { key: 'ArrowLeft', shiftKey: true, altKey: false }, null) ?? current;
    }
    expect(insideFrame(tilted, current)).toBe(true);
  });

  it('resizes from the bottom right with Alt', () => {
    const larger = applyCropKey(frame, rect, { key: 'ArrowRight', shiftKey: false, altKey: true }, null);
    expect(larger).toMatchObject({ left: 0.25, top: 0.25 });
    expect(larger?.width).toBeCloseTo(0.5 + KEY_STEP, 9);
    const smaller = applyCropKey(frame, rect, { key: 'ArrowUp', shiftKey: false, altKey: true }, null);
    expect(smaller?.height).toBeCloseTo(0.5 - KEY_STEP, 9);
  });

  it('keeps a locked aspect when it resizes', () => {
    const square = { left: 0.3, top: 0.2, width: 0.2, height: 0.3 };
    const next = applyCropKey(frame, square, { key: 'ArrowRight', shiftKey: true, altKey: true }, 1);
    expect(next).not.toBeNull();
    expect(pixelAspect(frame, next ?? square)).toBeCloseTo(1, 9);
  });

  it('ignores other keys', () => {
    for (const key of ['Enter', 'Escape', 'o', 'x', 'a']) {
      expect(applyCropKey(frame, rect, { key, ...plain }, null)).toBeNull();
    }
  });
});
