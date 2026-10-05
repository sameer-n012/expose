import { describe, expect, it } from 'vitest';
import {
  containRect,
  cropMatrix,
  cropTransform,
  frameToScreen,
  nextOverlay,
  overlayLines,
  rotatedFitScale,
  scaleContent,
  screenToFrame,
  toNormalizedDelta,
} from './cropView';

type Flips = { angle: number; flip_horizontal: boolean; flip_vertical: boolean };

/**
 * The backend mapping (port of `Geometry::new` in
 * `crates/expose-develop/src/geometry.rs` for the full rect): a frame
 * position to the source position.
 */
function backendSource(crop: Flips, w: number, h: number, ox: number, oy: number): [number, number] {
  const rad = (crop.angle * Math.PI) / 180;
  const sin = Math.sin(rad);
  const cos = Math.cos(rad);
  const cx = w / 2;
  const cy = h / 2;
  let sx = cos * (ox - cx) + sin * (oy - cy) + cx;
  let sy = -sin * (ox - cx) + cos * (oy - cy) + cy;
  if (crop.flip_horizontal) sx = w - sx;
  if (crop.flip_vertical) sy = h - sy;
  return [sx, sy];
}

/** The screen mapping of the CSS matrix (origin at the image center). */
function cssForward(crop: Flips, w: number, h: number, px: number, py: number): [number, number] {
  const [a, b, c, d] = cropMatrix(crop);
  const x = px - w / 2;
  const y = py - h / 2;
  return [a * x + c * y + w / 2, b * x + d * y + h / 2];
}

describe('cropMatrix', () => {
  it('is the inverse of the backend mapping for all flip and angle cases', () => {
    const [w, h] = [400, 300];
    for (const angle of [-45, -12.5, 0, 3, 30, 45]) {
      for (const flip_horizontal of [false, true]) {
        for (const flip_vertical of [false, true]) {
          const crop = { angle, flip_horizontal, flip_vertical };
          for (const [px, py] of [
            [10, 20],
            [390, 5],
            [200, 150],
            [77.5, 260.25],
          ] as const) {
            const [ox, oy] = cssForward(crop, w, h, px, py);
            const [sx, sy] = backendSource(crop, w, h, ox, oy);
            expect(sx).toBeCloseTo(px, 9);
            expect(sy).toBeCloseTo(py, 9);
          }
        }
      }
    }
  });

  it('maps the hand value of the Rust test back', () => {
    // W = 4, H = 2, angle 30: frame pixel (0, 0) samples source (0.4509619, 1.3169873).
    const crop = { angle: 30, flip_horizontal: false, flip_vertical: false };
    const [ox, oy] = cssForward(crop, 4, 2, 0.450_961_9, 1.316_987_3);
    expect(ox).toBeCloseTo(0.5, 6);
    expect(oy).toBeCloseTo(0.5, 6);
    // With a horizontal flip, the source x is W - x.
    const flipped = { ...crop, flip_horizontal: true };
    const [fx, fy] = cssForward(flipped, 4, 2, 4 - 0.450_961_9, 1.316_987_3);
    expect(fx).toBeCloseTo(0.5, 6);
    expect(fy).toBeCloseTo(0.5, 6);
  });

  it('flips first, then rotates', () => {
    // A point right of the center, flipped horizontally, goes left; then a
    // 90 degree clockwise turn (y down) moves it up.
    const crop = { angle: 90, flip_horizontal: true, flip_vertical: false };
    const [x, y] = cssForward(crop, 2, 2, 2, 1);
    expect(x).toBeCloseTo(1, 12);
    expect(y).toBeCloseTo(0, 12);
    // The other order (rotate, then flip) would give the point (1, 2).
  });
});

describe('cropTransform', () => {
  it('is none for no flip and no angle', () => {
    expect(cropTransform({ angle: 0, flip_horizontal: false, flip_vertical: false })).toBe('none');
  });

  it('writes the matrix in CSS order', () => {
    expect(cropTransform({ angle: 0, flip_horizontal: true, flip_vertical: false })).toBe('matrix(-1, 0, 0, 1, 0, 0)');
    expect(cropTransform({ angle: 0, flip_horizontal: false, flip_vertical: true })).toBe('matrix(1, 0, 0, -1, 0, 0)');
    // Clockwise 90 degrees on screen: CSS rotate(90deg) = matrix(0, 1, -1, 0, 0, 0).
    expect(cropTransform({ angle: 90, flip_horizontal: false, flip_vertical: false })).toBe('matrix(0, 1, -1, 0, 0, 0)');
    // Flip then rotate: rotate(90deg) scale(-1, 1).
    expect(cropTransform({ angle: 90, flip_horizontal: true, flip_vertical: false })).toBe('matrix(0, -1, -1, 0, 0, 0)');
  });
});

describe('screen mapping', () => {
  it('fits and centers the image in the box', () => {
    expect(containRect(800, 400, 300, 200)).toEqual({ x: 100, y: 0, width: 600, height: 400 });
    expect(containRect(300, 600, 300, 200)).toEqual({ x: 0, y: 200, width: 300, height: 200 });
    expect(containRect(0, 600, 300, 200)).toEqual({ x: 0, y: 0, width: 0, height: 0 });
  });

  it('converts a pointer move to normalized units', () => {
    expect(toNormalizedDelta(60, -20, { x: 100, y: 0, width: 600, height: 400 })).toEqual({ dx: 0.1, dy: -0.05 });
    expect(toNormalizedDelta(60, -20, { x: 0, y: 0, width: 0, height: 0 })).toEqual({ dx: 0, dy: 0 });
  });
});

describe('overlays', () => {
  it('cycles thirds, grid, golden', () => {
    expect(nextOverlay('thirds')).toBe('grid');
    expect(nextOverlay('grid')).toBe('golden');
    expect(nextOverlay('golden')).toBe('thirds');
  });

  it('gives the guide lines', () => {
    const thirds = overlayLines('thirds');
    expect(thirds).toHaveLength(4);
    expect(thirds[0]?.x1).toBeCloseTo(1 / 3, 12);
    expect(overlayLines('grid')).toHaveLength(10);
    const golden = overlayLines('golden').map((l) => l.x1).filter((x) => x > 0);
    expect(golden[0]).toBeCloseTo(0.381966, 6);
    expect(golden[1]).toBeCloseTo(0.618034, 6);
    expect(overlayLines('fine').length).toBeGreaterThan(overlayLines('grid').length);
  });
});

describe('rotatedFitScale', () => {
  const image = { width: 300, height: 200 };

  it('is 1 at angle 0 and never zooms in', () => {
    expect(rotatedFitScale(0, image, image)).toBe(1);
    expect(rotatedFitScale(0, image, { width: 900, height: 900 })).toBe(1);
    expect(rotatedFitScale(45, image, { width: 9000, height: 9000 })).toBe(1);
  });

  it('fits the bounding box of a 3:2 image at 45 degrees', () => {
    // Bounding box: (300 + 200) * cos 45 on both sides = 353.55.
    const expected = 200 / (500 * Math.SQRT1_2);
    expect(rotatedFitScale(45, image, image)).toBeCloseTo(expected, 9);
    expect(expected).toBeCloseTo(0.565685, 6);
    expect(rotatedFitScale(-45, image, image)).toBeCloseTo(expected, 9);
  });

  it('uses the tighter side of the box', () => {
    expect(rotatedFitScale(90, image, { width: 200, height: 300 })).toBeCloseTo(1, 9);
    expect(rotatedFitScale(90, image, { width: 100, height: 300 })).toBeCloseTo(0.5, 9);
  });

  it('does not depend on flips (the angle is the only input) and is 1 for an empty box', () => {
    // The flip changes the sign of the matrix terms, not the bounding box:
    // the zoomed matrix of a flipped view has the same length per axis.
    const [a, b] = cropMatrix({ angle: 30, flip_horizontal: true, flip_vertical: false }, 0.5);
    const [a2, b2] = cropMatrix({ angle: 30, flip_horizontal: false, flip_vertical: false }, 0.5);
    expect(Math.hypot(a, b)).toBeCloseTo(Math.hypot(a2, b2), 12);
    expect(rotatedFitScale(30, image, { width: 0, height: 0 })).toBe(1);
  });
});

describe('zoomed view', () => {
  it('scales the matrix and keeps its shape', () => {
    const flat = cropMatrix({ angle: 30, flip_horizontal: true, flip_vertical: false });
    const scaled = cropMatrix({ angle: 30, flip_horizontal: true, flip_vertical: false }, 0.5);
    scaled.forEach((v, i) => expect(v).toBeCloseTo((flat[i] ?? 0) * 0.5, 12));
    expect(cropTransform({ angle: 0, flip_horizontal: false, flip_vertical: false }, 0.5)).toBe('matrix(0.5, 0, 0, 0.5, 0, 0)');
    expect(cropTransform({ angle: 0, flip_horizontal: false, flip_vertical: false }, 1)).toBe('none');
  });

  it('scales the content about its center', () => {
    const content = { x: 50, y: 20, width: 400, height: 200 };
    expect(scaleContent(content, 0.5)).toEqual({ x: 150, y: 70, width: 200, height: 100 });
    expect(scaleContent(content, 1)).toEqual(content);
  });

  it('maps a frame point to the screen and back with the zoom-out', () => {
    const content = scaleContent({ x: 50, y: 20, width: 400, height: 200 }, 0.6);
    for (const [fx, fy] of [
      [0, 0],
      [1, 1],
      [0.25, 0.8],
    ] as const) {
      const screen = frameToScreen(content, fx, fy);
      const back = screenToFrame(content, screen.x, screen.y);
      expect(back.x).toBeCloseTo(fx, 12);
      expect(back.y).toBeCloseTo(fy, 12);
    }
    // The frame center stays at the center of the unscaled content.
    expect(frameToScreen(content, 0.5, 0.5)).toEqual({ x: 250, y: 120 });
    // A pointer move maps with the scaled size.
    expect(toNormalizedDelta(120, 0, content).dx).toBeCloseTo(0.5, 12);
    expect(screenToFrame({ x: 0, y: 0, width: 0, height: 0 }, 5, 5)).toEqual({ x: 0, y: 0 });
  });
});
