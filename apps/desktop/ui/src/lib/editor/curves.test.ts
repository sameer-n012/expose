import { describe, expect, it } from 'vitest';
import { addPoint, evaluateCurve, identityCurve, removePoint, withCurve, type CurvesAdjustment } from './curves';

describe('curve interpolation', () => {
  // Fritsch-Carlson by hand: deltas 1.6 and 0.4, tangents 1.6, 1.0, 0.4.
  // At x = 0.25: 0.125*0.5*1.6 + 0.5*0.8 - 0.125*0.5*1.0 = 0.4375.
  const pts = [
    { x: 0, y: 0 },
    { x: 0.5, y: 0.8 },
    { x: 1, y: 1 },
  ];

  it('matches the Rust spline at hand-computed values', () => {
    expect(evaluateCurve(pts, 0.25)).toBeCloseTo(0.4375, 6);
    expect(evaluateCurve(pts, 0.5)).toBeCloseTo(0.8, 6);
  });

  it('is flat outside the end points', () => {
    const p = [
      { x: 0.2, y: 0.1 },
      { x: 0.8, y: 0.7 },
    ];
    expect(evaluateCurve(p, 0)).toBeCloseTo(0.1);
    expect(evaluateCurve(p, 1)).toBeCloseTo(0.7);
  });
});

describe('curve edits', () => {
  it('keeps the curve valid for the Rust rules', () => {
    const added = addPoint(identityCurve(), 0.5, 0.7);
    expect(added?.points).toHaveLength(3);
    expect(addPoint(identityCurve(), 0.001, 0.5)).toBeNull();
    expect(removePoint(added?.points ?? [], 0)).toHaveLength(3);
    expect(removePoint(added?.points ?? [], 1)).toHaveLength(2);
  });

  it('replaces one curve and copies the points', () => {
    const base: CurvesAdjustment = {
      type: 'curves',
      master: identityCurve(),
      red: identityCurve(),
      green: identityCurve(),
      blue: identityCurve(),
    };
    const next = withCurve(base, 'green', [
      { x: 0, y: 0.1 },
      { x: 1, y: 1 },
    ]);
    expect(next.green[0]).toEqual({ x: 0, y: 0.1 });
    expect(next.master).toEqual(identityCurve());
  });
});
