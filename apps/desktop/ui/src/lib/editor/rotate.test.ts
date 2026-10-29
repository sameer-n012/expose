import { describe, expect, it } from 'vitest';
import { beginRotateDrag, cancelRotateDrag, dragRotation, pointerAngle, rotationLabel, snapRotation } from './rotate';

const viewport = { width: 800, height: 600 };
// Points around the center (400, 300).
const right = { x: 500, y: 300 };
const below = { x: 400, y: 400 };
const left = { x: 300, y: 300 };
const above = { x: 400, y: 200 };

describe('rotate tool', () => {
  it('measures angles clockwise on the screen', () => {
    expect(pointerAngle(right, viewport)).toBe(0);
    expect(pointerAngle(below, viewport)).toBeCloseTo(90);
    expect(pointerAngle(above, viewport)).toBeCloseTo(-90);
  });

  it('turns by the change of the pointer angle', () => {
    const drag = beginRotateDrag(10, right, viewport);
    expect(dragRotation(drag, below, viewport, false)).toBeCloseTo(100);
    expect(dragRotation(drag, above, viewport, false)).toBeCloseTo(-80);
    expect(dragRotation(drag, right, viewport, false)).toBeCloseTo(10);
  });

  it('wraps past 180°', () => {
    const drag = beginRotateDrag(170, right, viewport);
    // Half a turn more: 350° is -10°.
    expect(dragRotation(drag, left, viewport, false)).toBeCloseTo(-10);
  });

  it('snaps to 15° steps with Shift', () => {
    const drag = beginRotateDrag(0, right, viewport);
    // 20° clockwise snaps to 15°; 23° snaps to 30°.
    const at = (deg: number) => ({ x: 400 + 100 * Math.cos((deg * Math.PI) / 180), y: 300 + 100 * Math.sin((deg * Math.PI) / 180) });
    expect(dragRotation(drag, at(20), viewport, true)).toBe(15);
    expect(dragRotation(drag, at(23), viewport, true)).toBe(30);
    expect(dragRotation(drag, at(-7), viewport, true)).toBe(0);
    expect(snapRotation(-178)).toBe(180);
  });

  it('cancel gives back the start angle', () => {
    const drag = beginRotateDrag(-45, right, viewport);
    expect(cancelRotateDrag(drag)).toBe(-45);
  });

  it('labels the rotation', () => {
    expect(rotationLabel(30)).toBe('Rotation 30°');
    expect(rotationLabel(-12.345)).toBe('Rotation -12.3°');
    expect(rotationLabel(-0.01)).toBe('Rotation 0°');
  });
});
