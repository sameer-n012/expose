/**
 * Tone curve logic without the DOM: the point rules of the editor and a
 * monotone cubic interpolation for drawing.
 *
 * The types mirror `expose_core::develop::ToneCurve`. The interpolation here
 * is only for the drawing of the curve. The backend is the reference.
 */

/** The maximum number of points in one curve. Equal to `MAX_CURVE_POINTS` in `expose_core::develop`. */
export const MAX_CURVE_POINTS = 32;

/** The smallest distance in x between two neighbour points. */
export const MIN_GAP = 0.01;

export interface CurvePoint {
  readonly x: number;
  readonly y: number;
}

export const CURVE_CHANNELS = ['master', 'red', 'green', 'blue'] as const;
export type CurveChannel = (typeof CURVE_CHANNELS)[number];

/** The four curves of the develop settings (`tone_curve`). */
export type ToneCurveSettings = Readonly<Record<CurveChannel, readonly CurvePoint[]>>;

/** The identity curve: `(0, 0)` and `(1, 1)`. */
export function identityCurve(): CurvePoint[] {
  return [
    { x: 0, y: 0 },
    { x: 1, y: 1 },
  ];
}

/** True when the curve is the identity curve. */
export function isIdentity(points: readonly CurvePoint[]): boolean {
  return points.length === 2 && points[0]?.x === 0 && points[0].y === 0 && points[1]?.x === 1 && points[1].y === 1;
}

function clamp01(value: number): number {
  return Math.min(1, Math.max(0, value));
}

/**
 * Returns the value of the monotone cubic spline (Fritsch-Carlson) at `x`.
 * The result is in `0..=1`. A curve with fewer than 2 points is the identity.
 */
export function evaluateCurve(points: readonly CurvePoint[], x: number): number {
  return evaluateWith(points, tangents(points), x);
}

/** Returns `count` values of the curve at equal steps of x from 0 to 1. */
export function sampleCurve(points: readonly CurvePoint[], count: number): CurvePoint[] {
  const m = tangents(points);
  const out: CurvePoint[] = [];
  for (let i = 0; i < count; i++) {
    const x = count > 1 ? i / (count - 1) : 0;
    out.push({ x, y: evaluateWith(points, m, x) });
  }
  return out;
}

/** The tangent at each point (Fritsch-Carlson). */
function tangents(points: readonly CurvePoint[]): number[] {
  const n = points.length;
  if (n < 2) {
    return [];
  }
  const delta: number[] = [];
  for (let k = 0; k < n - 1; k++) {
    const a = points[k] as CurvePoint;
    const b = points[k + 1] as CurvePoint;
    delta.push((b.y - a.y) / (b.x - a.x));
  }
  const m: number[] = [delta[0] as number];
  for (let k = 1; k < n - 1; k++) {
    const before = delta[k - 1] as number;
    const after = delta[k] as number;
    m.push(before * after <= 0 ? 0 : (before + after) / 2);
  }
  m.push(delta[n - 2] as number);
  for (let k = 0; k < n - 1; k++) {
    const d = delta[k] as number;
    if (d === 0) {
      m[k] = 0;
      m[k + 1] = 0;
      continue;
    }
    const alpha = (m[k] as number) / d;
    const beta = (m[k + 1] as number) / d;
    const radius = alpha * alpha + beta * beta;
    if (radius > 9) {
      const tau = 3 / Math.sqrt(radius);
      m[k] = tau * alpha * d;
      m[k + 1] = tau * beta * d;
    }
  }
  return m;
}

function evaluateWith(points: readonly CurvePoint[], m: readonly number[], x: number): number {
  const n = points.length;
  const first = points[0];
  const last = points[n - 1];
  if (n < 2 || first === undefined || last === undefined) {
    return clamp01(x);
  }
  if (x <= first.x) {
    return clamp01(first.y);
  }
  if (x >= last.x) {
    return clamp01(last.y);
  }
  let k = 0;
  while (k < n - 2 && x > (points[k + 1] as CurvePoint).x) {
    k++;
  }
  const a = points[k] as CurvePoint;
  const b = points[k + 1] as CurvePoint;
  const h = b.x - a.x;
  const t = (x - a.x) / h;
  const t2 = t * t;
  const t3 = t2 * t;
  const y =
    (2 * t3 - 3 * t2 + 1) * a.y +
    (t3 - 2 * t2 + t) * h * (m[k] as number) +
    (-2 * t3 + 3 * t2) * b.y +
    (t3 - t2) * h * (m[k + 1] as number);
  return clamp01(y);
}

/**
 * Returns the curve with a new point at `(x, y)` and the index of the point,
 * or null when the curve has the maximum number of points or `x` is closer
 * than `MIN_GAP` to a point.
 */
export function addPoint(
  points: readonly CurvePoint[],
  x: number,
  y: number,
): { points: CurvePoint[]; index: number } | null {
  if (points.length >= MAX_CURVE_POINTS || !Number.isFinite(x) || !Number.isFinite(y)) {
    return null;
  }
  const nx = clamp01(x);
  if (points.some((p) => Math.abs(p.x - nx) < MIN_GAP)) {
    return null;
  }
  let index = points.findIndex((p) => p.x > nx);
  if (index < 0) {
    index = points.length;
  }
  const next = [...points];
  next.splice(index, 0, { x: nx, y: clamp01(y) });
  return { points: next, index };
}

/**
 * Returns the curve with point `index` at `(x, y)`. The y value is clamped to
 * `0..=1`. The x value is clamped between the neighbours (with `MIN_GAP`).
 * The first point keeps x = 0 and the last point keeps x = 1.
 */
export function movePoint(points: readonly CurvePoint[], index: number, x: number, y: number): CurvePoint[] {
  const current = points[index];
  if (current === undefined || !Number.isFinite(x) || !Number.isFinite(y)) {
    return [...points];
  }
  let nx = current.x;
  if (index !== 0 && index !== points.length - 1) {
    const low = (points[index - 1] as CurvePoint).x + MIN_GAP;
    const high = (points[index + 1] as CurvePoint).x - MIN_GAP;
    nx = Math.min(high, Math.max(low, x));
  }
  const next = [...points];
  next[index] = { x: nx, y: clamp01(y) };
  return next;
}

/** Returns the curve without point `index`. The two end points stay. */
export function removePoint(points: readonly CurvePoint[], index: number): CurvePoint[] {
  if (index <= 0 || index >= points.length - 1) {
    return [...points];
  }
  return points.filter((_, i) => i !== index);
}

/** True when `(x, y)` is so far outside the box that a drag there removes the point. */
export function isFarOutside(x: number, y: number, margin = 0.2): boolean {
  return x < -margin || x > 1 + margin || y < -margin || y > 1 + margin;
}
