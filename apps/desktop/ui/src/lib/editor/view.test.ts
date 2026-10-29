import { describe, expect, it } from 'vitest';
import {
  MAX_ZOOM,
  MIN_ZOOM,
  canvasToScreen,
  centeredView,
  clampView,
  clampZoom,
  fitView,
  fitZoom,
  frameBox,
  normalizeRotation,
  panBy,
  renderArea,
  rotatePoint,
  rotateVector,
  rotatedCover,
  screenToCanvas,
  unrotatePoint,
  steppedZoom,
  wheelZoomFactor,
  zoomAbout,
} from './view';

const viewport = { width: 800, height: 600 };
const doc = { width: 4000, height: 3000 };

describe('fit', () => {
  it('fits a large document with a margin', () => {
    const z = fitZoom(viewport, doc, 0);
    expect(z).toBeCloseTo(0.2);
    const view = fitView(viewport, doc, 0);
    expect(canvasToScreen(view, { x: 0, y: 0 })).toEqual({ x: 0, y: 0 });
    const end = canvasToScreen(view, { x: 4000, y: 3000 });
    expect(end.x).toBeCloseTo(800);
    expect(end.y).toBeCloseTo(600);
  });

  it('does not enlarge a small document', () => {
    expect(fitZoom(viewport, { width: 100, height: 100 })).toBe(1);
  });

  it('handles an empty document', () => {
    expect(fitZoom(viewport, { width: 0, height: 0 })).toBe(1);
  });
});

describe('coordinates', () => {
  it('round-trips', () => {
    const view = { zoom: 2.5, originX: 13, originY: -7 };
    const p = canvasToScreen(view, { x: 100, y: 50 });
    const q = screenToCanvas(view, p);
    expect(q.x).toBeCloseTo(100);
    expect(q.y).toBeCloseTo(50);
  });
});

describe('zoomAbout', () => {
  it('keeps the point under the pointer', () => {
    const view = { zoom: 1, originX: 100, originY: 200 };
    const anchor = { x: 300, y: 250 };
    const before = screenToCanvas(view, anchor);
    const next = zoomAbout(view, 4, anchor);
    expect(next.zoom).toBe(4);
    const after = screenToCanvas(next, anchor);
    expect(after.x).toBeCloseTo(before.x);
    expect(after.y).toBeCloseTo(before.y);
  });

  it('limits the zoom', () => {
    expect(zoomAbout({ zoom: 1, originX: 0, originY: 0 }, 1000, { x: 0, y: 0 }).zoom).toBe(MAX_ZOOM);
    expect(zoomAbout({ zoom: 1, originX: 0, originY: 0 }, 0, { x: 0, y: 0 }).zoom).toBe(MIN_ZOOM);
  });
});

describe('pan and clamp', () => {
  it('moves the content with the pointer', () => {
    const next = panBy({ zoom: 2, originX: 10, originY: 10 }, 20, -40);
    expect(next.originX).toBe(0);
    expect(next.originY).toBe(30);
  });

  it('centers a small document', () => {
    const view = clampView({ zoom: 1, originX: 999, originY: -999, rotation: 0 }, viewport, { width: 100, height: 100 });
    expect(canvasToScreen(view, { x: 50, y: 50 })).toEqual({ x: 400, y: 300 });
  });

  it('stops a large document at the slack', () => {
    const left = clampView({ zoom: 1, originX: -5000, originY: 0, rotation: 0 }, viewport, doc, 48);
    expect(left.originX).toBe(-48);
    const right = clampView({ zoom: 1, originX: 99999, originY: 0, rotation: 0 }, viewport, doc, 48);
    expect(right.originX).toBe(4000 - (800 - 48));
  });

  it('keeps the rotation and normalizes it', () => {
    expect(clampView({ zoom: 1, originX: 0, originY: 0, rotation: 370 }, viewport, doc).rotation).toBe(10);
  });

  it('lets each corner of a rotated document come into view', () => {
    // At 90°, the content-space box of the viewport is 600 x 800 around the same center.
    const rotation = 90;
    const left = clampView({ zoom: 1, originX: -5000, originY: -5000, rotation }, viewport, doc, 48);
    const right = clampView({ zoom: 1, originX: 99999, originY: 99999, rotation }, viewport, doc, 48);
    const cover = rotatedCover(viewport, rotation);
    // The document edge stops `slack` pixels inside the cover box on each side.
    expect(canvasToScreen(left, { x: 0, y: 0 })).toEqual({ x: cover.x + 48, y: cover.y + 48 });
    const end = canvasToScreen(right, { x: doc.width, y: doc.height });
    expect(end.x).toBeCloseTo(cover.x + cover.width - 48);
    expect(end.y).toBeCloseTo(cover.y + cover.height - 48);
  });

  it('centers at the clamp when the zoom fits', () => {
    const v = centeredView(viewport, doc, 0.1);
    expect(clampView(v, viewport, doc)).toEqual(v);
  });
});

describe('zoom steps', () => {
  it('steps in and out', () => {
    expect(steppedZoom(1, 1)).toBeCloseTo(1.25);
    expect(steppedZoom(1, -1)).toBeCloseTo(0.8);
    expect(steppedZoom(MAX_ZOOM, 1)).toBe(MAX_ZOOM);
  });

  it('clamps invalid zoom', () => {
    expect(clampZoom(Number.NaN)).toBe(1);
  });

  it('wheel factor: up zooms in, limited', () => {
    expect(wheelZoomFactor(-10)).toBeGreaterThan(1);
    expect(wheelZoomFactor(10)).toBeLessThan(1);
    expect(wheelZoomFactor(-100000)).toBeCloseTo(Math.exp(0.6));
  });
});

describe('render area', () => {
  it('uses device pixels', () => {
    const view = { zoom: 0.5, originX: 10, originY: 20, rotation: 0 };
    const area = renderArea(view, { width: 400.4, height: 300 }, 2);
    expect(area).toEqual({ originX: 10, originY: 20, scale: 1, width: 801, height: 600 });
  });

  it('falls back to a ratio of 1', () => {
    expect(renderArea({ zoom: 1, originX: 0, originY: 0, rotation: 0 }, viewport, 0).scale).toBe(1);
  });

  it('covers the rotated viewport', () => {
    const area = renderArea({ zoom: 2, originX: 10, originY: 20, rotation: 90 }, viewport, 1);
    // 600 x 800 CSS pixels: 100 px narrower and 100 px taller on each side (50 canvas px at zoom 2).
    expect(area).toEqual({ originX: 10 + 100 / 2, originY: 20 - 100 / 2, scale: 2, width: 600, height: 800 });
    const box = frameBox(area, { zoom: 2, originX: 10, originY: 20 });
    expect(box).toEqual({ left: 100, top: -100, width: 600, height: 800 });
  });

  it('places an old frame under a new view', () => {
    const area = renderArea({ zoom: 1, originX: 0, originY: 0, rotation: 0 }, viewport, 2);
    expect(frameBox(area, { zoom: 1, originX: 0, originY: 0 })).toEqual({ left: 0, top: 0, width: 800, height: 600 });
    const zoomed = frameBox(area, { zoom: 2, originX: 10, originY: 0 });
    expect(zoomed).toEqual({ left: -20, top: 0, width: 1600, height: 1200 });
  });
});

describe('rotation', () => {
  it('normalizes to (-180, 180]', () => {
    expect(normalizeRotation(0)).toBe(0);
    expect(Object.is(normalizeRotation(-0), 0)).toBe(true);
    expect(normalizeRotation(180)).toBe(180);
    expect(normalizeRotation(-180)).toBe(180);
    expect(normalizeRotation(190)).toBe(-170);
    expect(normalizeRotation(-190)).toBe(170);
    expect(normalizeRotation(720 + 45)).toBe(45);
    expect(normalizeRotation(Number.NaN)).toBe(0);
  });

  it('cover box at 0° is the viewport', () => {
    expect(rotatedCover(viewport, 0)).toEqual({ x: 0, y: 0, width: 800, height: 600 });
  });

  it('cover box at 90° swaps the sides', () => {
    expect(rotatedCover(viewport, 90)).toEqual({ x: 100, y: -100, width: 600, height: 800 });
    expect(rotatedCover(viewport, -90)).toEqual({ x: 100, y: -100, width: 600, height: 800 });
  });

  it('cover box at 45° holds the viewport corners', () => {
    const box = rotatedCover(viewport, 45);
    const side = 1400 * Math.SQRT1_2;
    expect(box.width).toBeCloseTo(side);
    expect(box.height).toBeCloseTo(side);
    expect(box.x + box.width / 2).toBeCloseTo(400);
    expect(box.y + box.height / 2).toBeCloseTo(300);
    // Each viewport corner in content space is inside the box.
    for (const corner of [
      { x: 0, y: 0 },
      { x: 800, y: 0 },
      { x: 0, y: 600 },
      { x: 800, y: 600 },
    ]) {
      const p = unrotatePoint(corner, viewport, 45);
      expect(p.x).toBeGreaterThanOrEqual(box.x - 1e-9);
      expect(p.x).toBeLessThanOrEqual(box.x + box.width + 1e-9);
      expect(p.y).toBeGreaterThanOrEqual(box.y - 1e-9);
      expect(p.y).toBeLessThanOrEqual(box.y + box.height + 1e-9);
    }
  });

  it('unrotates and rotates back', () => {
    const p = { x: 123, y: 456 };
    for (const angle of [0, 30, 90, -135, 180]) {
      const q = rotatePoint(unrotatePoint(p, viewport, angle), viewport, angle);
      expect(q.x).toBeCloseTo(p.x);
      expect(q.y).toBeCloseTo(p.y);
    }
  });

  it('keeps the center and the point at 0°', () => {
    expect(unrotatePoint({ x: 400, y: 300 }, viewport, 77)).toEqual({ x: 400, y: 300 });
    const p = { x: 5, y: 7 };
    expect(unrotatePoint(p, viewport, 0)).toBe(p);
  });

  it('turns clockwise on the screen', () => {
    // A content point right of the center shows below the center at 90°.
    expect(rotatePoint({ x: 500, y: 300 }, viewport, 90)).toEqual({ x: 400, y: 400 });
    expect(rotateVector({ x: 1, y: 0 }, 90)).toEqual({ x: 0, y: 1 });
  });

  it('fit keeps the rotation', () => {
    expect(fitView(viewport, doc, 0, 30).rotation).toBe(30);
    expect(fitView(viewport, doc, 0).rotation).toBe(0);
  });

  it('zoom and pan keep the rotation', () => {
    const view = { zoom: 1, originX: 0, originY: 0, rotation: 45 };
    expect(zoomAbout(view, 2, { x: 0, y: 0 }).rotation).toBe(45);
    expect(panBy(view, 10, 10).rotation).toBe(45);
  });
});
