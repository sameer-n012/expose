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
  panBy,
  renderArea,
  screenToCanvas,
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
    const view = clampView({ zoom: 1, originX: 999, originY: -999 }, viewport, { width: 100, height: 100 });
    expect(canvasToScreen(view, { x: 50, y: 50 })).toEqual({ x: 400, y: 300 });
  });

  it('stops a large document at the slack', () => {
    const left = clampView({ zoom: 1, originX: -5000, originY: 0 }, viewport, doc, 48);
    expect(left.originX).toBe(-48);
    const right = clampView({ zoom: 1, originX: 99999, originY: 0 }, viewport, doc, 48);
    expect(right.originX).toBe(4000 - (800 - 48));
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
    const view = { zoom: 0.5, originX: 10, originY: 20 };
    const area = renderArea(view, { width: 400.4, height: 300 }, 2);
    expect(area).toEqual({ originX: 10, originY: 20, scale: 1, width: 801, height: 600 });
  });

  it('falls back to a ratio of 1', () => {
    expect(renderArea({ zoom: 1, originX: 0, originY: 0 }, viewport, 0).scale).toBe(1);
  });

  it('places an old frame under a new view', () => {
    const area = renderArea({ zoom: 1, originX: 0, originY: 0 }, viewport, 2);
    expect(frameBox(area, { zoom: 1, originX: 0, originY: 0 })).toEqual({ left: 0, top: 0, width: 800, height: 600 });
    const zoomed = frameBox(area, { zoom: 2, originX: 10, originY: 0 });
    expect(zoomed).toEqual({ left: -20, top: 0, width: 1600, height: 1200 });
  });
});
