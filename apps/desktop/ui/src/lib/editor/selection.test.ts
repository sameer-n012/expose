import { describe, expect, it } from 'vitest';
import {
  MAX_POLYGON_POINTS,
  appendLassoPoint,
  cycleLasso,
  isSelectionTool,
  boxBetween,
  boxPath,
  ellipsePath,
  polylinePath,
  clampFeather,
  closesPolygon,
  contourPath,
  contoursPath,
  cycleFill,
  cycleMarquee,
  dragShape,
  isClick,
  modeFromModifiers,
  outlineStep,
  polygonShape,
  repeatsLastPoint,
} from './selection';

const view = { zoom: 2, originX: 10, originY: 20 };

describe('modeFromModifiers', () => {
  it('maps the four combinations', () => {
    expect(modeFromModifiers({ shiftKey: false, altKey: false })).toBe('replace');
    expect(modeFromModifiers({ shiftKey: true, altKey: false })).toBe('add');
    expect(modeFromModifiers({ shiftKey: false, altKey: true })).toBe('subtract');
    expect(modeFromModifiers({ shiftKey: true, altKey: true })).toBe('intersect');
  });
});

describe('dragShape', () => {
  it('converts screen corners to canvas pixels', () => {
    // screen (20, 40) -> canvas (20, 40); screen (60, 100) -> canvas (40, 70)
    expect(dragShape('rectSelect', { x: 20, y: 40 }, { x: 60, y: 100 }, view)).toEqual({ type: 'rect', x: 20, y: 40, width: 20, height: 30 });
  });

  it('gives the same box for a negative drag', () => {
    expect(dragShape('ellipseSelect', { x: 60, y: 100 }, { x: 20, y: 40 }, view)).toEqual({ type: 'ellipse', x: 20, y: 40, width: 20, height: 30 });
  });

  it('rounds to whole pixels', () => {
    const shape = dragShape('rectSelect', { x: 1, y: 1 }, { x: 10, y: 10 }, { zoom: 1, originX: 0.4, originY: 0.4 });
    expect(shape).toEqual({ type: 'rect', x: 1, y: 1, width: 9, height: 9 });
  });

  it('returns null for a box without area', () => {
    expect(dragShape('rectSelect', { x: 5, y: 5 }, { x: 5.2, y: 50 }, { zoom: 1, originX: 0, originY: 0 })).toBeNull();
  });

  it('allows a box outside the canvas', () => {
    const shape = dragShape('rectSelect', { x: -20, y: -20 }, { x: 0, y: 0 }, { zoom: 1, originX: 0, originY: 0 });
    expect(shape).toEqual({ type: 'rect', x: -20, y: -20, width: 20, height: 20 });
  });
});

describe('isClick', () => {
  it('is true below 2 screen pixels', () => {
    expect(isClick({ x: 0, y: 0 }, { x: 1, y: 1 })).toBe(true);
    expect(isClick({ x: 0, y: 0 }, { x: 2, y: 0 })).toBe(false);
  });
});

describe('boxBetween', () => {
  it('normalizes the corners', () => {
    expect(boxBetween({ x: 9, y: 1 }, { x: 3, y: 5 })).toEqual({ x: 3, y: 1, width: 6, height: 4 });
  });
});

describe('polygon rules', () => {
  const pts = [
    { x: 0, y: 0 },
    { x: 100, y: 0 },
    { x: 100, y: 100 },
  ];

  it('closes only with 3 points and near the first point', () => {
    expect(closesPolygon(pts, { x: 4, y: 4 })).toBe(true);
    expect(closesPolygon(pts, { x: 7, y: 0 })).toBe(false);
    expect(closesPolygon(pts.slice(0, 2), { x: 1, y: 1 })).toBe(false);
    expect(closesPolygon([], { x: 0, y: 0 })).toBe(false);
  });

  it('finds a repeated point of a double-click', () => {
    expect(repeatsLastPoint(pts, { x: 100.5, y: 100 })).toBe(true);
    expect(repeatsLastPoint(pts, { x: 50, y: 50 })).toBe(false);
    expect(repeatsLastPoint([], { x: 0, y: 0 })).toBe(false);
  });

  it('makes a shape from 3 or more points and rounds them', () => {
    expect(polygonShape(pts.slice(0, 2))).toBeNull();
    expect(polygonShape([{ x: 0.4, y: 0.6 }, ...pts.slice(1)])).toEqual({
      type: 'polygon',
      points: [
        [0, 1],
        [100, 0],
        [100, 100],
      ],
    });
  });
});

describe('outlineStep', () => {
  it('is 1 at 100 % and above', () => {
    expect(outlineStep(1)).toBe(1);
    expect(outlineStep(8)).toBe(1);
  });
  it('grows when the view zooms out', () => {
    expect(outlineStep(0.5)).toBe(2);
    expect(outlineStep(0.25)).toBe(4);
    expect(outlineStep(0.3)).toBe(3);
  });
  it('is 1 for a bad scale', () => {
    expect(outlineStep(0)).toBe(1);
    expect(outlineStep(Number.NaN)).toBe(1);
  });
});

describe('contour paths', () => {
  it('writes a closed path', () => {
    expect(contourPath([0, 0, 10, 0, 10, 5])).toBe('M0 0 L10 0 L10 5 Z');
  });
  it('skips contours with fewer than 3 points', () => {
    expect(contourPath([0, 0, 1, 1])).toBe('');
    expect(contoursPath([[0, 0, 1, 1], [0, 0, 4, 0, 4, 4]])).toBe('M0 0 L4 0 L4 4 Z');
  });
  it('joins contours', () => {
    expect(contoursPath([[0, 0, 4, 0, 4, 4], [1, 1, 2, 1, 2, 2]])).toBe('M0 0 L4 0 L4 4 Z M1 1 L2 1 L2 2 Z');
  });
});

describe('preview paths', () => {
  it('writes a box, an ellipse, and a line', () => {
    expect(boxPath({ x: 1, y: 2, width: 3, height: 4 })).toBe('M1 2 h3 v4 h-3 Z');
    expect(ellipsePath({ x: 0, y: 0, width: 10, height: 4 })).toBe('M0 2 A5 2 0 1 0 10 2 A5 2 0 1 0 0 2 Z');
    expect(polylinePath([{ x: 1, y: 2 }, { x: 3, y: 4 }])).toBe('M1 2 L3 4');
    expect(polylinePath([])).toBe('');
  });
});

describe('tool helpers', () => {
  it('cycles the marquee tools', () => {
    expect(cycleMarquee('hand')).toBe('rectSelect');
    expect(cycleMarquee('rectSelect')).toBe('ellipseSelect');
    expect(cycleMarquee('ellipseSelect')).toBe('rectSelect');
  });
  it('cycles the fill tools', () => {
    expect(cycleFill('hand')).toBe('paintBucket');
    expect(cycleFill('hand', 'gradient')).toBe('gradient');
    expect(cycleFill('paintBucket')).toBe('gradient');
    expect(cycleFill('gradient')).toBe('paintBucket');
  });
  it('cycles the lasso tools', () => {
    expect(cycleLasso('hand')).toBe('lasso');
    expect(cycleLasso('hand', 'polygonLasso')).toBe('polygonLasso');
    expect(cycleLasso('lasso', 'lasso')).toBe('polygonLasso');
    expect(cycleLasso('polygonLasso')).toBe('lasso');
  });
  it('knows the selection tools', () => {
    expect(isSelectionTool('lasso')).toBe(true);
    expect(isSelectionTool('polygonLasso')).toBe(true);
    expect(isSelectionTool('brush')).toBe(false);
  });
  it('adds freehand points only when far enough', () => {
    const points: { x: number; y: number }[] = [];
    expect(appendLassoPoint(points, { x: 0, y: 0 }, 1)).toBe(true);
    expect(appendLassoPoint(points, { x: 0.5, y: 0 }, 1)).toBe(false);
    expect(appendLassoPoint(points, { x: 1, y: 0 }, 1)).toBe(true);
    expect(points).toHaveLength(2);
  });
  it('stops adding freehand points at the cap', () => {
    const points = Array.from({ length: MAX_POLYGON_POINTS }, (_, i) => ({ x: i, y: 0 }));
    expect(appendLassoPoint(points, { x: -50, y: 9 }, 1)).toBe(false);
    expect(points).toHaveLength(MAX_POLYGON_POINTS);
  });
  it('limits the feather', () => {
    expect(clampFeather(-3)).toBe(0);
    expect(clampFeather(900)).toBe(250);
    expect(clampFeather(12)).toBe(12);
    expect(clampFeather(Number.NaN)).toBe(0);
  });
});
