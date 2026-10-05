import { describe, expect, it } from 'vitest';
import {
  CropGeometryError,
  FULL_RECT,
  MIN_RECT_SIZE,
  fitRect,
  maxInscribedRect,
  mirrorRect,
  moveRect,
  pixelAspect,
  rectForAngle,
  rectForAspect,
  rectIsInside,
  resizeRect,
  swapRect,
  type CropFrame,
  type NormalizedRect,
} from './cropGeometry';

const center = (r: NormalizedRect): [number, number] => [r.left + r.width / 2, r.top + r.height / 2];

describe('maxInscribedRect (cases of the Rust tests)', () => {
  it('inscribed_rect_at_zero_angle', () => {
    expect(maxInscribedRect(30, 20, 0)).toEqual(FULL_RECT);
    // A square in a 30 x 20 frame: 20 x 20 pixels, centered.
    const sq = maxInscribedRect(30, 20, 0, 1);
    expect(Math.abs(sq.width - 20 / 30)).toBeLessThan(1e-6);
    expect(Math.abs(sq.height - 1)).toBeLessThan(1e-6);
    expect(Math.abs(sq.left - 5 / 30)).toBeLessThan(1e-6);
    expect(Math.abs(sq.top)).toBeLessThan(1e-6);
  });

  it('inscribed_rect_is_inside_and_maximal', () => {
    const [w, h] = [300, 200];
    for (const angle of [-45, -20, -3, 1, 12.5, 33, 45]) {
      for (const aspect of [null, 1, 16 / 9, 0.5, 4]) {
        const r = maxInscribedRect(w, h, angle, aspect);
        expect(rectIsInside(w, h, angle, r), `${angle} ${aspect}`).toBe(true);
        const ratio = aspect ?? 1.5;
        const pxRatio = (r.width * w) / (r.height * h);
        expect(Math.abs(pxRatio - ratio)).toBeLessThan(1e-3 * ratio);
        // Grow the rect by 0.5 % about the center: it must not fit.
        const scale = 1.005;
        const width = r.width * scale;
        const height = r.height * scale;
        const bigger = { left: 0.5 - width / 2, top: 0.5 - height / 2, width, height };
        expect(rectIsInside(w, h, angle, bigger), `${angle} ${aspect}`).toBe(false);
      }
    }
  });

  it('inscribed_rect_area_beats_search', () => {
    const [w, h, angle] = [300, 200, 15];
    const r = maxInscribedRect(w, h, angle);
    let best = 0;
    for (let i = 1; i <= 1000; i++) {
      const k = i / 1000;
      const cand = { left: 0.5 - k / 2, top: 0.5 - k / 2, width: k, height: k };
      if (rectIsInside(w, h, angle, cand)) {
        best = Math.max(best, k * k);
      }
    }
    const area = r.width * r.height;
    expect(area).toBeGreaterThanOrEqual(best - 1e-6);
    expect(area - best).toBeLessThan(0.005);
  });

  it('rejects bad input', () => {
    expect(() => maxInscribedRect(0, 10, 0)).toThrow(CropGeometryError);
    expect(() => maxInscribedRect(10, 10, Number.NaN)).toThrow(CropGeometryError);
    expect(() => maxInscribedRect(10, 10, 0, 0)).toThrow(CropGeometryError);
    expect(() => maxInscribedRect(10, 10, 0, Number.POSITIVE_INFINITY)).toThrow(CropGeometryError);
  });
});

describe('rectIsInside', () => {
  it('rect_is_inside_rejects_bad_input', () => {
    const full = FULL_RECT;
    expect(rectIsInside(10, 10, 0, full)).toBe(true);
    expect(rectIsInside(10, 10, 5, full)).toBe(false);
    expect(rectIsInside(0, 10, 0, full)).toBe(false);
    expect(rectIsInside(10, 10, 0, { ...full, width: 0 })).toBe(false);
    expect(rectIsInside(10, 10, 0, { ...full, left: 0.6 })).toBe(false);
    expect(rectIsInside(10, 10, Number.NaN, { left: 0.4, top: 0.4, width: 0.2, height: 0.2 })).toBe(false);
  });

  it('accepts a small centered rect at all angles and rejects corners when rotated', () => {
    const small = { left: 0.45, top: 0.45, width: 0.1, height: 0.1 };
    const corner = { left: 0, top: 0, width: 0.1, height: 0.1 };
    for (const angle of [-45, -10, 0, 7.5, 30, 45]) {
      expect(rectIsInside(400, 300, angle, small)).toBe(true);
      expect(rectIsInside(400, 300, angle, corner)).toBe(angle === 0);
    }
  });

  it('uses a tolerance of 0.01 source pixels', () => {
    expect(rectIsInside(100, 100, 0, { left: -0.00009, top: 0, width: 0.5, height: 0.5 })).toBe(true);
    expect(rectIsInside(100, 100, 0, { left: -0.0002, top: 0, width: 0.5, height: 0.5 })).toBe(false);
  });
});

const frame = (angle: number): CropFrame => ({ width: 300, height: 200, angle });

describe('fitRect', () => {
  it('keeps a rect that fits', () => {
    const r = { left: 0.3, top: 0.3, width: 0.2, height: 0.2 };
    expect(fitRect(frame(10), r)).toBe(r);
  });

  it('shrinks about the center and keeps the aspect', () => {
    for (const angle of [-30, -5, 5, 20, 45]) {
      const r = { left: 0, top: 0.05, width: 0.95, height: 0.9 };
      expect(rectIsInside(300, 200, angle, r)).toBe(false);
      const fitted = fitRect(frame(angle), r);
      expect(rectIsInside(300, 200, angle, fitted)).toBe(true);
      expect(pixelAspect(frame(angle), fitted)).toBeCloseTo(pixelAspect(frame(angle), r), 9);
      const [cx, cy] = center(fitted);
      expect(cx).toBeCloseTo(0.475, 9);
      expect(cy).toBeCloseTo(0.5, 9);
      // Maximal: 0.1 % larger does not fit.
      const grown = {
        left: cx - (fitted.width * 1.001) / 2,
        top: cy - (fitted.height * 1.001) / 2,
        width: fitted.width * 1.001,
        height: fitted.height * 1.001,
      };
      expect(rectIsInside(300, 200, angle, grown)).toBe(false);
    }
  });

  it('moves a rect whose center is outside the rotated image', () => {
    const r = { left: 0, top: 0, width: 0.05, height: 0.05 };
    const fitted = fitRect(frame(30), r);
    expect(rectIsInside(300, 200, 30, fitted)).toBe(true);
    expect(pixelAspect(frame(30), fitted)).toBeCloseTo(1.5, 6);
  });
});

describe('moveRect', () => {
  const r = { left: 0.25, top: 0.25, width: 0.5, height: 0.5 };

  it('moves freely inside', () => {
    const moved = moveRect(frame(0), r, 0.1, -0.05);
    expect(moved.left).toBeCloseTo(0.35, 12);
    expect(moved.top).toBeCloseTo(0.2, 12);
    expect(moved.width).toBe(0.5);
  });

  it('clamps at the frame edges and slides along them', () => {
    const moved = moveRect(frame(0), r, 0.6, 0.1);
    // The edit tolerance (1e-6 px) allows a few 1e-9 of overshoot.
    expect(moved.left).toBeCloseTo(0.5, 7);
    expect(moved.top).toBeCloseTo(0.35, 7);
    expect(moved.width).toBe(0.5);
    expect(moved.height).toBe(0.5);
    const up = moveRect(frame(0), r, -1, -1);
    expect(up.left).toBeCloseTo(0, 7);
    expect(up.top).toBeCloseTo(0, 7);
  });

  it('clamps at the rotated image edges', () => {
    const f = frame(20);
    const start = scaleCenter(maxInscribedRect(300, 200, 20), 0.6);
    const moved = moveRect(f, start, 0.5, 0);
    expect(rectIsInside(300, 200, 20, moved)).toBe(true);
    expect(moved.left).toBeGreaterThan(start.left);
    expect(moved.left + moved.width).toBeLessThan(1);
  });
});

function scaleCenter(r: NormalizedRect, s: number): NormalizedRect {
  const width = r.width * s;
  const height = r.height * s;
  return { left: r.left + (r.width - width) / 2, top: r.top + (r.height - height) / 2, width, height };
}

describe('resizeRect', () => {
  const r = { left: 0.25, top: 0.25, width: 0.5, height: 0.5 };

  it('free edge resize changes one side only', () => {
    const out = resizeRect(frame(0), r, 'e', 0.1, 0.3, null);
    expect(out).toEqual({ left: 0.25, top: 0.25, width: 0.6, height: 0.5 });
    const west = resizeRect(frame(0), r, 'w', 0.1, 0, null);
    expect(west.left).toBeCloseTo(0.35, 12);
    expect(west.width).toBeCloseTo(0.4, 12);
    const north = resizeRect(frame(0), r, 'n', 0, -0.1, null);
    expect(north.top).toBeCloseTo(0.15, 12);
    expect(north.height).toBeCloseTo(0.6, 12);
  });

  it('free corner resize stops at the frame', () => {
    const out = resizeRect(frame(0), r, 'se', 1, 1, null);
    expect(out.left).toBe(0.25);
    expect(out.left + out.width).toBeCloseTo(1, 7);
    expect(out.top + out.height).toBeCloseTo(1, 7);
  });

  it('keeps the minimum size', () => {
    const out = resizeRect(frame(0), r, 'nw', 2, 2, null);
    expect(out.width).toBeCloseTo(MIN_RECT_SIZE, 12);
    expect(out.height).toBeCloseTo(MIN_RECT_SIZE, 12);
    expect(out.left + out.width).toBeCloseTo(0.75, 12);
    const ratio = resizeRect(frame(0), r, 'se', -2, -2, 1);
    expect(Math.min(ratio.width, ratio.height)).toBeGreaterThanOrEqual(MIN_RECT_SIZE - 1e-12);
  });

  it('corner resize with a ratio keeps the ratio', () => {
    const f = frame(0);
    const square = rectForAspect(f, r, 1);
    for (const [handle, dx, dy] of [
      ['se', 0.05, 0.3],
      ['nw', -0.2, 0.01],
      ['ne', -0.1, -0.1],
      ['sw', 0.3, 0.3],
    ] as const) {
      const start = scaleCenter(square, 0.5);
      const out = resizeRect(f, start, handle, dx, dy, 1);
      expect(pixelAspect(f, out)).toBeCloseTo(1, 9);
      expect(rectIsInside(300, 200, 0, out)).toBe(true);
    }
    // The anchor (opposite corner) stays.
    const start = scaleCenter(square, 0.5);
    const out = resizeRect(f, start, 'se', 0.05, 0, 1);
    expect(out.left).toBeCloseTo(start.left, 12);
    expect(out.top).toBeCloseTo(start.top, 12);
  });

  it('edge resize with a ratio keeps the center of the other axis', () => {
    const f = frame(10);
    const start = scaleCenter(maxInscribedRect(300, 200, 10, 1.5), 0.5);
    const out = resizeRect(f, start, 'e', 0.05, 0, 1.5);
    expect(pixelAspect(f, out)).toBeCloseTo(1.5, 9);
    expect(center(out)[1]).toBeCloseTo(center(start)[1], 12);
    expect(out.left).toBeCloseTo(start.left, 12);
  });

  it('a ratio resize stops inside the rotated image', () => {
    const f = frame(25);
    const start = scaleCenter(maxInscribedRect(300, 200, 25, 1), 0.5);
    const out = resizeRect(f, start, 'se', 1, 1, 1);
    expect(rectIsInside(300, 200, 25, out)).toBe(true);
    expect(pixelAspect(f, out)).toBeCloseTo(1, 6);
    expect(out.width).toBeGreaterThan(start.width);
  });
});

describe('rect changes', () => {
  it('a largest rect follows the angle', () => {
    const r0 = rectForAngle(frame(0), FULL_RECT, 10, null);
    expect(r0).toEqual(maxInscribedRect(300, 200, 10, 1.5));
    const back = rectForAngle(frame(10), r0, 0, null);
    expect(back.width).toBeCloseTo(1, 9);
    expect(back.height).toBeCloseTo(1, 9);
  });

  it('a smaller rect stays when it fits and shrinks about its center when not', () => {
    const small = { left: 0.4, top: 0.4, width: 0.2, height: 0.2 };
    expect(rectForAngle(frame(0), small, 30, null)).toBe(small);
    const wide = { left: 0.05, top: 0.3, width: 0.9, height: 0.4 };
    const out = rectForAngle(frame(0), wide, 15, null);
    expect(rectIsInside(300, 200, 15, out)).toBe(true);
    expect(pixelAspect(frame(15), out)).toBeCloseTo(pixelAspect(frame(0), wide), 9);
    expect(center(out)[0]).toBeCloseTo(0.5, 9);
    expect(center(out)[1]).toBeCloseTo(0.5, 9);
  });

  it('an aspect change gives the largest rect of the ratio', () => {
    expect(rectForAspect(frame(5), FULL_RECT, 1)).toEqual(maxInscribedRect(300, 200, 5, 1));
    const r = { left: 0.1, top: 0.1, width: 0.3, height: 0.3 };
    expect(rectForAspect(frame(0), r, null)).toBe(r);
  });

  it('swaps the orientation about the center', () => {
    const f = frame(0);
    const r = { left: 0.4, top: 0.3, width: 0.2, height: 0.3 }; // 60 x 60 px
    const wide = { left: 0.3, top: 0.4, width: 0.4, height: 0.2 }; // 120 x 40 px
    const swapped = swapRect(f, wide);
    expect(swapped.width * 300).toBeCloseTo(40, 9);
    expect(swapped.height * 200).toBeCloseTo(120, 9);
    expect(center(swapped)[0]).toBeCloseTo(0.5, 12);
    expect(center(swapped)[1]).toBeCloseTo(0.5, 12);
    expect(swapRect(f, r).width).toBeCloseTo(0.2, 12);
    // The full frame (300 x 200) becomes 200 x 300 px, scaled to fit: 133.3 x 200.
    const full = swapRect(f, FULL_RECT);
    expect(full.height).toBeCloseTo(1, 9);
    expect(full.width * 300).toBeCloseTo(200 * (200 / 300), 6);
  });

  it('mirrors a rect', () => {
    const r = { left: 0.1, top: 0.2, width: 0.3, height: 0.4 };
    expect(mirrorRect(r, true).left).toBeCloseTo(0.6, 12);
    expect(mirrorRect(r, false).top).toBeCloseTo(0.4, 12);
  });
});
