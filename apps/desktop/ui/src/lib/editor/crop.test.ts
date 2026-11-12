import { describe, expect, it } from 'vitest';
import {
  aspectRatio,
  cropCorners,
  cropCursor,
  cropEdit,
  cropHandlePoint,
  cropHitTest,
  fitAspect,
  fullBox,
  isFullBox,
  resizeByHandle,
  rotateByPointer,
  roundBox,
  swapBox,
  thirdsLines,
  translateBox,
  withFields,
  type CropBox,
} from './crop';

const CANVAS = { width: 100, height: 80 };
const box = (extra: Partial<CropBox> = {}): CropBox => ({ ...fullBox(CANVAS), ...extra });

describe('basics', () => {
  it('the full box covers the canvas', () => {
    expect(fullBox(CANVAS)).toEqual({ centerX: 50, centerY: 40, width: 100, height: 80, angle: 0 });
    expect(isFullBox(fullBox(CANVAS), CANVAS)).toBe(true);
    expect(isFullBox(box({ angle: 1 }), CANVAS)).toBe(false);
    expect(isFullBox(box({ width: 99 }), CANVAS)).toBe(false);
  });

  it('corners and handles at angle 0', () => {
    expect(cropCorners(box())[0]).toEqual({ x: 0, y: 0 });
    expect(cropCorners(box())[2]).toEqual({ x: 100, y: 80 });
    expect(cropHandlePoint(box(), 'r')).toEqual({ x: 100, y: 40 });
    expect(cropHandlePoint(box(), 'b')).toEqual({ x: 50, y: 80 });
  });

  it('a positive angle turns the box clockwise on the screen', () => {
    const p = cropHandlePoint(box({ width: 20, height: 20, centerX: 0, centerY: 0, angle: 90 }), 'r');
    expect(p.x).toBeCloseTo(0, 9);
    expect(p.y).toBeCloseTo(10, 9);
  });

  it('the thirds lines split the box', () => {
    const lines = thirdsLines(box({ width: 90, height: 60, centerX: 45, centerY: 30 }));
    expect(lines).toHaveLength(4);
    expect(lines[0]?.[0].x).toBeCloseTo(30, 9);
    expect(lines[1]?.[0].y).toBeCloseTo(20, 9);
  });
});

describe('hit test', () => {
  it('handle, inside, outside', () => {
    expect(cropHitTest(box(), { x: 99, y: 41 }, 1)).toEqual({ kind: 'handle', handle: 'r' });
    expect(cropHitTest(box(), { x: 3, y: 2 }, 1)).toEqual({ kind: 'handle', handle: 'tl' });
    expect(cropHitTest(box(), { x: 50, y: 20 }, 1)).toEqual({ kind: 'move' });
    expect(cropHitTest(box(), { x: 130, y: 20 }, 1)).toEqual({ kind: 'rotate' });
  });

  it('the handle radius shrinks with the zoom in canvas pixels', () => {
    expect(cropHitTest(box(), { x: 90, y: 40 }, 1)).toEqual({ kind: 'move' });
    expect(cropHitTest(box(), { x: 90, y: 40 }, 0.5)).toEqual({ kind: 'handle', handle: 'r' });
  });

  it('works in the rotated frame', () => {
    const b = box({ angle: 90, width: 40, height: 20, centerX: 50, centerY: 40 });
    // The right handle is below the center after 90 degrees.
    expect(cropHitTest(b, { x: 50, y: 60 }, 1)).toEqual({ kind: 'handle', handle: 'r' });
  });

  it('resize cursors follow the angle and the view rotation', () => {
    expect(cropCursor(box(), { kind: 'handle', handle: 'r' })).toBe('ew-resize');
    expect(cropCursor(box(), { kind: 'handle', handle: 'tl' })).toBe('nwse-resize');
    expect(cropCursor(box({ angle: 45 }), { kind: 'handle', handle: 'r' })).toBe('nwse-resize');
    expect(cropCursor(box(), { kind: 'handle', handle: 'r' }, 90)).toBe('ns-resize');
  });
});

describe('rounding', () => {
  it('rounds the edges to whole pixels at angle 0', () => {
    const r = roundBox({ centerX: 10.3, centerY: 20.4, width: 8.2, height: 6.7, angle: 0 });
    expect(r.width).toBe(8);
    expect(r.height).toBe(7);
    expect(Number.isInteger(r.centerX - r.width / 2)).toBe(true);
    expect(Number.isInteger(r.centerY - r.height / 2)).toBe(true);
  });

  it('keeps the box at other angles', () => {
    const b = { centerX: 10.3, centerY: 20.4, width: 8.2, height: 6.7, angle: 3 };
    expect(roundBox(b)).toEqual(b);
  });

  it('a translate rounds the offset', () => {
    const t = translateBox(box({ width: 50, centerX: 25 }), 10.4, 3.6);
    expect(t.centerX - t.width / 2).toBe(10);
    expect(t.centerY - t.height / 2).toBe(4);
  });
});

describe('resize', () => {
  it('a free right handle moves only the right edge', () => {
    const r = resizeByHandle(box(), 'r', { x: 70, y: 10 }, null);
    expect(r).toEqual({ centerX: 35, centerY: 40, width: 70, height: 80, angle: 0 });
  });

  it('a free top-left corner keeps the bottom-right corner', () => {
    const r = resizeByHandle(box(), 'tl', { x: 20, y: 10 }, null);
    expect(r.centerX + r.width / 2).toBe(100);
    expect(r.centerY + r.height / 2).toBe(80);
    expect(r.width).toBe(80);
    expect(r.height).toBe(70);
  });

  it('the size is at least 1 px', () => {
    const r = resizeByHandle(box(), 'r', { x: -50, y: 0 }, null);
    expect(r.width).toBe(1);
    expect(r.centerX - r.width / 2).toBe(0);
  });

  it('a corner with a ratio keeps the ratio and the opposite corner', () => {
    const r = resizeByHandle(box({ width: 90, height: 60, centerX: 45, centerY: 30 }), 'br', { x: 60, y: 10 }, 3 / 2);
    expect(r.width / r.height).toBeCloseTo(1.5, 1);
    expect(r.centerX - r.width / 2).toBe(0);
    expect(r.centerY - r.height / 2).toBe(0);
  });

  it('an edge with a ratio changes the other side about the middle', () => {
    const r = resizeByHandle(box({ width: 80, height: 80, centerX: 40, centerY: 40 }), 'r', { x: 40, y: 0 }, 1);
    expect(r.width).toBe(40);
    expect(r.height).toBe(40);
    expect(r.centerY).toBe(40);
    expect(r.centerX).toBe(20);
  });

  it('resizes in the rotated frame about the opposite handle', () => {
    const start = box({ centerX: 50, centerY: 50, width: 40, height: 20, angle: 90 });
    const fixed = cropHandlePoint(start, 'l');
    // The right handle is at (50, 70). Drag it to (50, 90): the width grows by 20.
    const r = resizeByHandle(start, 'r', { x: 50, y: 90 }, null);
    expect(r.width).toBeCloseTo(60, 9);
    expect(r.height).toBe(20);
    const after = cropHandlePoint(r, 'l');
    expect(after.x).toBeCloseTo(fixed.x, 9);
    expect(after.y).toBeCloseTo(fixed.y, 9);
  });

  it('about the center keeps the center', () => {
    const r = resizeByHandle(box(), 'r', { x: 80, y: 40 }, null, true);
    expect(r.centerX).toBe(50);
    expect(r.width).toBe(60);
  });
});

describe('aspect', () => {
  it('ratios and orientation', () => {
    expect(aspectRatio('free', false, CANVAS)).toBeNull();
    expect(aspectRatio('1:1', false, CANVAS)).toBe(1);
    expect(aspectRatio('4:3', false, CANVAS)).toBeCloseTo(4 / 3, 9);
    expect(aspectRatio('4:3', true, CANVAS)).toBeCloseTo(3 / 4, 9);
    expect(aspectRatio('original', false, CANVAS)).toBeCloseTo(1.25, 9);
    expect(aspectRatio('16:9', false, CANVAS)).toBeCloseTo(16 / 9, 9);
  });

  it('fitAspect keeps the box inside the old one', () => {
    const r = fitAspect(box(), 1);
    expect(r.width).toBe(80);
    expect(r.height).toBe(80);
    expect(r.centerX).toBe(50);
  });

  it('swap exchanges the sides about the center', () => {
    const s = swapBox(box({ width: 40, height: 20, centerX: 50, centerY: 40 }));
    expect(s.width).toBe(20);
    expect(s.height).toBe(40);
    expect(s.centerX).toBe(50);
  });
});

describe('rotate', () => {
  const start = box({ centerX: 0, centerY: 0, width: 20, height: 20 });

  it('turns by the pointer angle', () => {
    const r = rotateByPointer(start, { x: 10, y: 0 }, { x: 0, y: 10 }, false);
    // 90 degrees is above the limit.
    expect(r.angle).toBe(45);
    const small = rotateByPointer(start, { x: 10, y: 0 }, { x: 10, y: 5 }, false);
    expect(small.angle).toBeCloseTo((Math.atan(0.5) * 180) / Math.PI, 9);
  });

  it('Shift snaps to whole degrees', () => {
    const r = rotateByPointer(start, { x: 10, y: 0 }, { x: 10, y: 5 }, true);
    expect(r.angle).toBe(27);
  });

  it('clamps to 45 degrees in both directions', () => {
    expect(rotateByPointer(start, { x: 10, y: 0 }, { x: -10, y: 1 }, false).angle).toBe(45);
    expect(rotateByPointer(start, { x: 10, y: 0 }, { x: -10, y: -1 }, false).angle).toBe(-45);
  });

  it('keeps the center', () => {
    const r = rotateByPointer(box(), { x: 100, y: 40 }, { x: 100, y: 60 }, false);
    expect(r.centerX).toBe(50);
    expect(r.centerY).toBe(40);
  });
});

describe('fields and edit', () => {
  it('sets the width with a ratio', () => {
    const r = withFields(box(), { width: 50 }, 1);
    expect(r.width).toBe(50);
    expect(r.height).toBe(50);
  });

  it('clamps the angle and drops bad values', () => {
    expect(withFields(box(), { angle: 90 }, null).angle).toBe(45);
    expect(withFields(box({ angle: 5 }), { angle: Number.NaN }, null).angle).toBe(5);
    expect(withFields(box(), { width: Number.NaN }, null).width).toBe(100);
  });

  it('the edit has whole sizes and the options', () => {
    const e = cropEdit({ centerX: 30.5, centerY: 20, width: 40.4, height: 10.6, angle: 3 }, false, 'bilinear');
    expect(e).toEqual({
      type: 'cropCanvas',
      centerX: 30.5,
      centerY: 20,
      width: 40,
      height: 11,
      angle: 3,
      deleteCropped: false,
      interpolation: 'bilinear',
    });
  });
});
