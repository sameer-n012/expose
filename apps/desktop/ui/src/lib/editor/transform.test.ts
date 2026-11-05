import { describe, expect, it } from 'vitest';
import {
  applyMatrix,
  boundsCenter,
  boxCorners,
  containsPoint,
  cursorFor,
  handlePoint,
  hitTest,
  identityState,
  invertMatrix,
  isIdentityMatrix,
  isIdentityState,
  moveReference,
  referenceAtGrid,
  referenceImage,
  rotateByPointer,
  scaleByHandle,
  snapAngleRadians,
  stateFields,
  stateMatrix,
  translate,
  withFields,
  type TransformState,
} from './transform';

const BOUNDS = { x: 10, y: 20, width: 100, height: 50 };
const base = (): TransformState => identityState(BOUNDS);
const NO_KEYS = { keepAspect: false, aboutCenter: false };

function expectPoint(p: { x: number; y: number }, x: number, y: number): void {
  expect(p.x).toBeCloseTo(x, 6);
  expect(p.y).toBeCloseTo(y, 6);
}

describe('matrices', () => {
  it('the identity state gives the identity matrix', () => {
    expect(isIdentityState(base())).toBe(true);
    expect(isIdentityMatrix(stateMatrix(base()))).toBe(true);
  });

  it('composes translate(-ref), scale, rotate, translate(ref + t) in that order', () => {
    const s: TransformState = { ...base(), ref: { x: 0, y: 0 }, tx: 5, ty: 7, sx: 2, sy: 3, angle: Math.PI / 2 };
    // (1, 0): scale -> (2, 0); rotate 90 deg clockwise (y down) -> (0, 2); translate -> (5, 9).
    expectPoint(applyMatrix(stateMatrix(s), { x: 1, y: 0 }), 5, 9);
    // (0, 1): scale -> (0, 3); rotate -> (-3, 0); translate -> (2, 7).
    expectPoint(applyMatrix(stateMatrix(s), { x: 0, y: 1 }), 2, 7);
  });

  it('keeps the reference point at ref + t', () => {
    const s: TransformState = { ...base(), tx: 4, ty: -3, sx: 1.5, sy: 0.5, angle: 0.7 };
    expectPoint(applyMatrix(stateMatrix(s), s.ref), s.ref.x + 4, s.ref.y - 3);
  });

  it('the matrix has the CSS matrix order (a, b, c, d, e, f)', () => {
    const s: TransformState = { ...base(), ref: { x: 0, y: 0 }, tx: 5, ty: 6, sx: 2, sy: 3, angle: 0 };
    [2, 0, 0, 3, 5, 6].forEach((value, i) => expect(stateMatrix(s)[i]).toBeCloseTo(value, 9));
  });

  it('the inverse maps back', () => {
    const s: TransformState = { ...base(), tx: 4, ty: -3, sx: 1.5, sy: 0.5, angle: 0.7 };
    const m = stateMatrix(s);
    const inverse = invertMatrix(m);
    expect(inverse).not.toBeNull();
    const p = { x: 33, y: 44 };
    if (inverse !== null) {
      const back = applyMatrix(inverse, applyMatrix(m, p));
      expectPoint(back, 33, 44);
    }
  });

  it('a singular matrix has no inverse', () => {
    expect(invertMatrix([1, 0, 0, 0, 0, 0])).toBeNull();
  });
});

describe('hit tests', () => {
  it('finds the handles, the reference, the inside, and the rotate area', () => {
    const s = base();
    expect(hitTest(s, { x: 10, y: 20 }, 1)).toEqual({ kind: 'handle', handle: 'tl' });
    expect(hitTest(s, { x: 110, y: 45 }, 1)).toEqual({ kind: 'handle', handle: 'r' });
    expect(hitTest(s, boundsCenter(BOUNDS), 1)).toEqual({ kind: 'ref' });
    expect(hitTest(s, { x: 30, y: 30 }, 1)).toEqual({ kind: 'move' });
    expect(hitTest(s, { x: 125, y: 85 }, 1)).toEqual({ kind: 'rotate' });
    expect(hitTest(s, { x: 300, y: 300 }, 1)).toBeNull();
  });

  it('the hit radius is in screen pixels', () => {
    const s = base();
    // 15 canvas px from the corner is 7.5 screen px at zoom 0.5: a hit.
    expect(hitTest(s, { x: -5, y: 20 }, 0.5)).toEqual({ kind: 'handle', handle: 'tl' });
    expect(hitTest(s, { x: -5, y: 20 }, 4)).toBeNull();
  });

  it('follows a rotated box', () => {
    const s: TransformState = { ...base(), angle: Math.PI / 2 };
    const corners = boxCorners(s);
    expect(hitTest(s, corners[0], 1)).toEqual({ kind: 'handle', handle: 'tl' });
    expect(containsPoint(s, boundsCenter(BOUNDS))).toBe(true);
    // The box is 50 wide and 100 tall after a quarter turn about the center (60, 45).
    expect(containsPoint(s, { x: 60, y: 90 })).toBe(true);
    expect(containsPoint(s, { x: 100, y: 45 })).toBe(false);
  });

  it('picks a cursor from the handle direction', () => {
    const s = base();
    expect(cursorFor(s, { kind: 'handle', handle: 'r' })).toBe('ew-resize');
    expect(cursorFor(s, { kind: 'handle', handle: 't' })).toBe('ns-resize');
    expect(cursorFor(s, { kind: 'handle', handle: 'br' })).not.toBe('ew-resize');
    expect(cursorFor(s, { kind: 'handle', handle: 'r' }, 90)).toBe('ns-resize');
    expect(cursorFor(s, { kind: 'move' })).toBe('move');
    expect(cursorFor(s, null)).toBeNull();
  });
});

describe('scale', () => {
  it('a corner handle scales both axes about the reference point', () => {
    const s = { ...base(), ref: { x: 10, y: 20 } };
    const next = scaleByHandle(s, 'br', { x: 110, y: 70 }, { x: 210, y: 120 }, NO_KEYS);
    expect(next.sx).toBeCloseTo(2, 6);
    expect(next.sy).toBeCloseTo(2, 6);
    // The reference point stays.
    expectPoint(referenceImage(next), 10, 20);
    expectPoint(handlePoint(next, 'br'), 210, 120);
  });

  it('an edge handle scales one axis', () => {
    const s = { ...base(), ref: { x: 10, y: 20 } };
    const next = scaleByHandle(s, 'r', { x: 110, y: 45 }, { x: 210, y: 300 }, NO_KEYS);
    expect(next.sx).toBeCloseTo(2, 6);
    expect(next.sy).toBe(1);
  });

  it('Shift keeps the aspect ratio', () => {
    const s = { ...base(), ref: { x: 10, y: 20 } };
    const next = scaleByHandle(s, 'br', { x: 110, y: 70 }, { x: 210, y: 80 }, { keepAspect: true, aboutCenter: false });
    expect(next.sx).toBeCloseTo(next.sy, 6);
    expect(next.sx).toBeCloseTo(2, 6);
  });

  it('Shift keeps a non-uniform start ratio', () => {
    const s = { ...base(), ref: { x: 10, y: 20 }, sx: 2, sy: 1 };
    const next = scaleByHandle(s, 'br', handlePoint(s, 'br'), { x: 410, y: 70 }, { keepAspect: true, aboutCenter: false });
    expect(next.sx / next.sy).toBeCloseTo(2, 6);
  });

  it('Alt scales about the box center', () => {
    const s = { ...base(), ref: { x: 10, y: 20 } };
    const center = applyMatrix(stateMatrix(s), boundsCenter(BOUNDS));
    const next = scaleByHandle(s, 'br', { x: 110, y: 70 }, { x: 160, y: 95 }, { keepAspect: false, aboutCenter: true });
    expect(next.sx).toBeCloseTo(2, 6);
    expect(next.sy).toBeCloseTo(2, 6);
    expectPoint(applyMatrix(stateMatrix(next), boundsCenter(BOUNDS)), center.x, center.y);
  });

  it('scales a rotated box along its own axes', () => {
    const s: TransformState = { ...base(), ref: { x: 60, y: 45 }, angle: Math.PI / 2 };
    const handle = handlePoint(s, 'r');
    const next = scaleByHandle(s, 'r', handle, { x: handle.x, y: handle.y + 50 }, NO_KEYS);
    // The right handle points down after a quarter turn: 50 px further doubles the half width of 50.
    expect(next.sx).toBeCloseTo(2, 6);
  });

  it('can flip with a negative scale', () => {
    const s = { ...base(), ref: { x: 10, y: 20 } };
    const next = scaleByHandle(s, 'r', { x: 110, y: 45 }, { x: -90, y: 45 }, NO_KEYS);
    expect(next.sx).toBeCloseTo(-1, 6);
  });
});

describe('rotate', () => {
  it('turns about the reference point', () => {
    const s = { ...base(), ref: { x: 0, y: 0 } };
    const next = rotateByPointer(s, { x: 10, y: 0 }, { x: 0, y: 10 }, false);
    expect(next.angle).toBeCloseTo(Math.PI / 2, 6);
    expectPoint(referenceImage(next), 0, 0);
  });

  it('Shift snaps the angle to 15 degrees', () => {
    const s = { ...base(), ref: { x: 0, y: 0 } };
    const angle = (20 * Math.PI) / 180;
    const next = rotateByPointer(s, { x: 10, y: 0 }, { x: 10 * Math.cos(angle), y: 10 * Math.sin(angle) }, true);
    expect(next.angle).toBeCloseTo((15 * Math.PI) / 180, 9);
    expect(snapAngleRadians((23 * Math.PI) / 180)).toBeCloseTo((30 * Math.PI) / 180, 9);
  });
});

describe('reference point', () => {
  it('moving it keeps the matrix', () => {
    const s: TransformState = { ...base(), tx: 4, ty: 9, sx: 1.5, sy: 0.8, angle: 0.6 };
    const moved = moveReference(s, { x: 77, y: 31 });
    const a = stateMatrix(s);
    const b = stateMatrix(moved);
    a.forEach((value, i) => expect(b[i]).toBeCloseTo(value, 9));
    expect(moved.ref).toEqual({ x: 77, y: 31 });
  });

  it('the grid sets the reference to corners, edges, and the center', () => {
    expect(referenceAtGrid(base(), 0, 0).ref).toEqual({ x: 10, y: 20 });
    expect(referenceAtGrid(base(), 2, 1).ref).toEqual({ x: 110, y: 45 });
    expect(referenceAtGrid(base(), 1, 1).ref).toEqual({ x: 60, y: 45 });
    expect(isIdentityState(referenceAtGrid(base(), 2, 2))).toBe(true);
  });
});

describe('fields', () => {
  it('shows the state and takes it back (round trip)', () => {
    const s: TransformState = { ...base(), tx: 4, ty: 9, sx: 1.5, sy: 0.8, angle: 0.6 };
    const fields = stateFields(s);
    expect(fields.width).toBeCloseTo(150, 9);
    expect(fields.height).toBeCloseTo(80, 9);
    expect(fields.x).toBeCloseTo(s.ref.x + 4, 9);
    const back = withFields(base(), fields);
    expect(back.tx).toBeCloseTo(4, 9);
    expect(back.ty).toBeCloseTo(9, 9);
    expect(back.sx).toBeCloseTo(1.5, 9);
    expect(back.sy).toBeCloseTo(0.8, 9);
    expect(back.angle).toBeCloseTo(0.6, 9);
  });

  it('X and Y place the reference point', () => {
    const next = withFields(base(), { x: 200, y: 100 });
    expectPoint(referenceImage(next), 200, 100);
  });

  it('ignores a bad value and a zero scale', () => {
    const s = base();
    expect(withFields(s, { width: Number.NaN, height: 0 })).toEqual(s);
  });

  it('wraps the angle into (-180, 180]', () => {
    expect(stateFields(withFields(base(), { angle: 190 })).angle).toBeCloseTo(-170, 9);
    expect(stateFields(withFields(base(), { angle: -180 })).angle).toBeCloseTo(180, 9);
  });

  it('translate adds to the translation', () => {
    const next = translate(base(), 3, -2);
    expectPoint(referenceImage(next), 63, 43);
  });
});
