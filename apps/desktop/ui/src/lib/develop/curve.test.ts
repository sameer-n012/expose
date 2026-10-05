import { describe, expect, it } from 'vitest';
import {
  addPoint,
  evaluateCurve,
  identityCurve,
  isFarOutside,
  isIdentity,
  MAX_CURVE_POINTS,
  MIN_GAP,
  movePoint,
  removePoint,
  sampleCurve,
  type CurvePoint,
} from './curve';

const S: CurvePoint[] = [
  { x: 0, y: 0 },
  { x: 0.25, y: 0.15 },
  { x: 0.5, y: 0.5 },
  { x: 0.75, y: 0.9 },
  { x: 1, y: 1 },
];

describe('evaluateCurve', () => {
  it('passes through the points', () => {
    for (const p of S) {
      expect(evaluateCurve(S, p.x)).toBeCloseTo(p.y, 9);
    }
  });

  it('is the identity for the identity curve', () => {
    for (const x of [0, 0.1, 0.5, 0.99, 1]) {
      expect(evaluateCurve(identityCurve(), x)).toBeCloseTo(x, 9);
    }
  });

  it('is monotone for monotone points, also with sharp steps', () => {
    const steep: CurvePoint[] = [
      { x: 0, y: 0 },
      { x: 0.4, y: 0.05 },
      { x: 0.45, y: 0.9 },
      { x: 0.5, y: 0.92 },
      { x: 1, y: 1 },
    ];
    for (const points of [S, steep]) {
      const samples = sampleCurve(points, 257);
      for (let i = 1; i < samples.length; i++) {
        expect((samples[i] as CurvePoint).y).toBeGreaterThanOrEqual((samples[i - 1] as CurvePoint).y - 1e-12);
      }
    }
  });

  it('stays in 0..1', () => {
    for (const p of sampleCurve(S, 100)) {
      expect(p.y).toBeGreaterThanOrEqual(0);
      expect(p.y).toBeLessThanOrEqual(1);
    }
  });
});

describe('point rules', () => {
  it('adds a point in x order and returns its index', () => {
    const r = addPoint(identityCurve(), 0.5, 0.7);
    expect(r?.index).toBe(1);
    expect(r?.points.map((p) => p.x)).toEqual([0, 0.5, 1]);
  });

  it('refuses a point at the maximum count or too near a point', () => {
    let points: CurvePoint[] = identityCurve();
    for (let i = 1; points.length < MAX_CURVE_POINTS; i++) {
      points = addPoint(points, i / 40, 0.5)?.points ?? points;
    }
    expect(points.length).toBe(MAX_CURVE_POINTS);
    expect(addPoint(points, 0.999, 0.5)).toBeNull();
    expect(addPoint(identityCurve(), MIN_GAP / 2, 0.5)).toBeNull();
  });

  it('clamps a moved point between its neighbours', () => {
    const moved = movePoint(S, 2, 0.9, 1.5);
    expect(moved[2]?.x).toBeCloseTo(0.75 - MIN_GAP, 9);
    expect(moved[2]?.y).toBe(1);
    expect(movePoint(S, 2, 0, -1)[2]?.x).toBeCloseTo(0.25 + MIN_GAP, 9);
  });

  it('keeps the end points at x = 0 and x = 1', () => {
    const moved = movePoint(movePoint(S, 0, 0.3, 0.2), 4, 0.6, 0.8);
    expect(moved[0]).toEqual({ x: 0, y: 0.2 });
    expect(moved[4]).toEqual({ x: 1, y: 0.8 });
  });

  it('removes inner points only', () => {
    expect(removePoint(S, 2).length).toBe(4);
    expect(removePoint(S, 0)).toEqual(S);
    expect(removePoint(S, 4)).toEqual(S);
    expect(removePoint(identityCurve(), 1).length).toBe(2);
  });

  it('knows the identity and far outside', () => {
    expect(isIdentity(identityCurve())).toBe(true);
    expect(isIdentity(S)).toBe(false);
    expect(isFarOutside(0.5, 0.5)).toBe(false);
    expect(isFarOutside(0.5, 1.5)).toBe(true);
    expect(isFarOutside(-0.3, 0.5)).toBe(true);
  });
});
