/**
 * Curves rules for the Editor adjustment layer.
 *
 * The Rust side (`Curves::validate` and `MonotoneCubic` in
 * `crates/expose-editor/src/adjust.rs`) uses the same rules and the same
 * `MonotoneSpline` as the develop tone curve: 2 to 32 points, coordinates in
 * `0..=1`, `x` strictly increasing, monotone cubic (Fritsch-Carlson), flat
 * outside the end points. So the point logic and the drawing interpolation of
 * `lib/develop/curve.ts` are reused. Its `MIN_GAP` (0.01) is stricter than
 * the Rust rule, which is safe.
 */
import type { AdjustmentDto } from './api';
import type { CurveChannel, CurvePoint } from '../develop/curve';

export {
  addPoint,
  CURVE_CHANNELS,
  evaluateCurve,
  identityCurve,
  isFarOutside,
  isIdentity,
  MAX_CURVE_POINTS,
  MIN_GAP,
  movePoint,
  removePoint,
  sampleCurve,
} from '../develop/curve';
export type { CurveChannel, CurvePoint } from '../develop/curve';

export type CurvesAdjustment = Extract<AdjustmentDto, { type: 'curves' }>;

/** Returns the adjustment with one curve replaced. The other curves stay. */
export function withCurve(adjustment: CurvesAdjustment, channel: CurveChannel, points: readonly CurvePoint[]): CurvesAdjustment {
  return { ...adjustment, [channel]: points.map((p) => ({ x: p.x, y: p.y })) };
}
