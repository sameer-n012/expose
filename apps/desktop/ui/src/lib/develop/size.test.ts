import { describe, expect, it } from 'vitest';
import {
  MAX_TARGET_LONG_EDGE,
  MIN_TARGET_LONG_EDGE,
  TARGET_LONG_EDGE_STEP,
  sameAspect,
  targetLongEdge,
} from './size';

const landscape = { width: 2560, height: 1707 };

describe('targetLongEdge', () => {
  it('fits the image into the box at the device-pixel ratio', () => {
    // Width limits: 800 CSS px * 2 = 1600 device px -> 1664 (next step of 128).
    expect(targetLongEdge({ width: 800, height: 800 }, 2, landscape)).toBe(1664);
    // Height limits: 300 * 1 = 300 px high -> long edge 450 -> 512.
    expect(targetLongEdge({ width: 2000, height: 300 }, 1, landscape)).toBe(512);
    // A portrait image in a wide box: the height limits.
    expect(targetLongEdge({ width: 2000, height: 600 }, 1, { width: 1707, height: 2560 })).toBe(640);
  });

  it('does not exceed the image long edge', () => {
    expect(targetLongEdge({ width: 3000, height: 2000 }, 2, landscape)).toBe(2560);
    expect(targetLongEdge({ width: 2500, height: 2000 }, 1, landscape)).toBe(2560);
  });

  it('keeps the target the same for small box changes', () => {
    const a = targetLongEdge({ width: 801, height: 900 }, 1, landscape);
    const b = targetLongEdge({ width: 830, height: 900 }, 1, landscape);
    expect(a).toBe(896);
    expect(b).toBe(a);
  });

  it('gives at least one step (and the minimum) for small boxes', () => {
    expect(targetLongEdge({ width: 3, height: 2 }, 1, landscape)).toBe(TARGET_LONG_EDGE_STEP);
    expect(TARGET_LONG_EDGE_STEP).toBeGreaterThanOrEqual(MIN_TARGET_LONG_EDGE);
    // A small image keeps its size even below the minimum.
    expect(targetLongEdge({ width: 3, height: 2 }, 1, { width: 40, height: 30 })).toBe(40);
  });

  it('clamps to the maximum', () => {
    const huge = { width: 20000, height: 10000 };
    expect(targetLongEdge({ width: 9000, height: 9000 }, 2, huge)).toBe(MAX_TARGET_LONG_EDGE);
  });

  it('uses the frame aspect ratio and the session limit for a crop', () => {
    // A small portrait crop frame (300 x 600) of a landscape session in a
    // tall box: the frame fills the box height, 1600 device px.
    const crop = { width: 300, height: 600 };
    expect(targetLongEdge({ width: 800, height: 800 }, 2, crop, 2560)).toBe(1664);
    // The session aspect would give a smaller target (soft frame).
    expect(targetLongEdge({ width: 400, height: 800 }, 2, landscape)).toBe(896);
    expect(targetLongEdge({ width: 400, height: 800 }, 2, crop, 2560)).toBe(1664);
    // The session long edge limits the target, not the frame size.
    expect(targetLongEdge({ width: 3000, height: 3000 }, 2, crop, 2560)).toBe(2560);
    // Without a limit: the long edge of the image.
    expect(targetLongEdge({ width: 3000, height: 3000 }, 2, crop)).toBe(600);
    expect(targetLongEdge({ width: 500, height: 500 }, 1, crop, 0)).toBeNull();
  });

  it('returns null for an empty or invalid size', () => {
    expect(targetLongEdge({ width: 0, height: 500 }, 2, landscape)).toBeNull();
    expect(targetLongEdge({ width: 500, height: 500 }, Number.NaN, landscape)).toBeNull();
    expect(targetLongEdge({ width: 500, height: 500 }, 1, { width: 0, height: 0 })).toBeNull();
  });
});

describe('sameAspect', () => {
  it('ignores pixel rounding and detects real changes', () => {
    expect(sameAspect({ width: 640, height: 427 }, { width: 1280, height: 853 })).toBe(true);
    expect(sameAspect({ width: 640, height: 427 }, { width: 640, height: 480 })).toBe(false);
    expect(sameAspect({ width: 0, height: 427 }, { width: 640, height: 427 })).toBe(false);
  });
});
