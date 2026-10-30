/**
 * Pure logic of free transform (ADR 0029).
 *
 * - The state is: the bounds of the layer content (canvas pixels), a
 *   reference point (canvas pixels), a translation, a scale (sx, sy), and an
 *   angle (radians, clockwise on the screen because y points down).
 * - The matrix is translate(-ref), scale(sx, sy), rotate(angle),
 *   translate(ref + t). It maps the old layer position to the new one.
 *   Format: `[a, b, c, d, e, f]` with `x' = a*x + c*y + e` and
 *   `y' = b*x + d*y + f` (the order of a CSS `matrix()`).
 * - All points are canvas pixels. Hit tests take a `zoom` (screen pixels per
 *   canvas pixel) to turn screen distances into canvas distances.
 */
import type { TransformMatrix } from './api';
import type { Point } from './view';

/** A 2D affine matrix `[a, b, c, d, e, f]`. */
export type Matrix = TransformMatrix;

/** The layer bounds in canvas pixels. x and y can be negative. */
export interface Bounds {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

export type Interpolation = 'bicubic' | 'bilinear' | 'nearest';

export interface TransformState {
  readonly bounds: Bounds;
  /** The reference point, in canvas pixels. */
  readonly ref: Point;
  readonly tx: number;
  readonly ty: number;
  readonly sx: number;
  readonly sy: number;
  /** Radians. */
  readonly angle: number;
}

/** The angle step of the Shift snap, in degrees. */
export const ANGLE_SNAP_DEGREES = 15;
/** The size of a handle hit area, in screen pixels (distance from the handle center). */
export const HANDLE_HIT_RADIUS = 8;
/** The width of the rotate area outside the corners, in screen pixels. */
export const ROTATE_ZONE = 24;
/** The smallest scale that the numeric fields accept (absolute value). */
export const MIN_SCALE = 0.0001;

const EPSILON = 1e-9;

const IDENTITY: Matrix = [1, 0, 0, 1, 0, 0];

// ---- Matrices ----

/** Maps a point with a matrix. */
export function applyMatrix(m: Matrix, p: Point): Point {
  return { x: m[0] * p.x + m[2] * p.y + m[4], y: m[1] * p.x + m[3] * p.y + m[5] };
}

/** The inverse of a matrix, or null if it is singular. */
export function invertMatrix(m: Matrix): Matrix | null {
  const [a, b, c, d, e, f] = m;
  const det = a * d - b * c;
  if (!Number.isFinite(det) || Math.abs(det) < EPSILON) {
    return null;
  }
  return [d / det, -b / det, -c / det, a / det, (c * f - d * e) / det, (b * e - a * f) / det];
}

/** True if the matrix is the identity (within a small tolerance). */
export function isIdentityMatrix(m: Matrix, tolerance = 1e-6): boolean {
  return m.every((value, i) => Math.abs(value - (IDENTITY[i] ?? 0)) <= tolerance);
}

/** The matrix of a state. */
export function stateMatrix(s: TransformState): Matrix {
  const cos = Math.cos(s.angle);
  const sin = Math.sin(s.angle);
  const a = cos * s.sx;
  const b = sin * s.sx;
  const c = -sin * s.sy;
  const d = cos * s.sy;
  return [a, b, c, d, s.ref.x + s.tx - (a * s.ref.x + c * s.ref.y), s.ref.y + s.ty - (b * s.ref.x + d * s.ref.y)];
}

/** The state that does nothing; the reference point is the bounds center. */
export function identityState(bounds: Bounds): TransformState {
  return { bounds, ref: boundsCenter(bounds), tx: 0, ty: 0, sx: 1, sy: 1, angle: 0 };
}

/** True if the state gives the identity matrix. */
export function isIdentityState(s: TransformState): boolean {
  return isIdentityMatrix(stateMatrix(s));
}

export function boundsCenter(b: Bounds): Point {
  return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
}

// ---- Handles ----

export type HandleId = 'tl' | 't' | 'tr' | 'r' | 'br' | 'b' | 'bl' | 'l';

/** The handles in drawing order. */
export const HANDLES: readonly HandleId[] = ['tl', 't', 'tr', 'r', 'br', 'b', 'bl', 'l'];

/** The side of a handle on each axis: -1 left/top, 0 middle, 1 right/bottom. */
const HANDLE_SIDE: Record<HandleId, readonly [number, number]> = {
  tl: [-1, -1],
  t: [0, -1],
  tr: [1, -1],
  r: [1, 0],
  br: [1, 1],
  b: [0, 1],
  bl: [-1, 1],
  l: [-1, 0],
};

/** The position of a handle in the bounds, before the transform. */
export function handleLocal(bounds: Bounds, handle: HandleId): Point {
  const [sx, sy] = HANDLE_SIDE[handle];
  return { x: bounds.x + ((sx + 1) * bounds.width) / 2, y: bounds.y + ((sy + 1) * bounds.height) / 2 };
}

/** The position of a handle after the transform. */
export function handlePoint(s: TransformState, handle: HandleId): Point {
  return applyMatrix(stateMatrix(s), handleLocal(s.bounds, handle));
}

/** The four corners after the transform: top-left, top-right, bottom-right, bottom-left. */
export function boxCorners(s: TransformState): [Point, Point, Point, Point] {
  return [handlePoint(s, 'tl'), handlePoint(s, 'tr'), handlePoint(s, 'br'), handlePoint(s, 'bl')];
}

/** The SVG path of the transformed box outline in the given point space. */
export function boxOutline(corners: readonly Point[]): string {
  return corners.map((p, i) => `${i === 0 ? 'M' : 'L'} ${p.x} ${p.y}`).join(' ') + ' Z';
}

// ---- Hit tests ----

export type Hit = { readonly kind: 'ref' } | { readonly kind: 'handle'; readonly handle: HandleId } | { readonly kind: 'move' } | { readonly kind: 'rotate' };

function distance(a: Point, b: Point): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

/** True if the canvas point is inside the transformed box. */
export function containsPoint(s: TransformState, p: Point): boolean {
  const inverse = invertMatrix(stateMatrix(s));
  if (inverse === null) {
    return false;
  }
  const q = applyMatrix(inverse, p);
  const b = s.bounds;
  return q.x >= b.x && q.x <= b.x + b.width && q.y >= b.y && q.y <= b.y + b.height;
}

/** The reference point after the transform (it is where the reference marker shows). */
export function referenceImage(s: TransformState): Point {
  return { x: s.ref.x + s.tx, y: s.ref.y + s.ty };
}

/**
 * Tests which part of the transform a canvas point touches. The order is:
 * reference marker, handles, inside the box (move), near a corner outside
 * the box (rotate). Returns null for any other point.
 */
export function hitTest(s: TransformState, p: Point, zoom: number): Hit | null {
  const scale = zoom > 0 ? zoom : 1;
  const hitRadius = HANDLE_HIT_RADIUS / scale;
  if (distance(p, referenceImage(s)) <= hitRadius) {
    return { kind: 'ref' };
  }
  let best: { handle: HandleId; d: number } | null = null;
  for (const handle of HANDLES) {
    const d = distance(p, handlePoint(s, handle));
    if (d <= hitRadius && (best === null || d < best.d)) {
      best = { handle, d };
    }
  }
  if (best !== null) {
    return { kind: 'handle', handle: best.handle };
  }
  if (containsPoint(s, p)) {
    return { kind: 'move' };
  }
  const zone = ROTATE_ZONE / scale;
  if (boxCorners(s).some((corner) => distance(p, corner) <= zone)) {
    return { kind: 'rotate' };
  }
  return null;
}

const RESIZE_CURSORS = ['ew-resize', 'nwse-resize', 'ns-resize', 'nesw-resize'] as const;

/**
 * The CSS cursor for a hit. `viewRotation` is the rotation of the view in
 * degrees. A resize cursor follows the direction of the handle on the screen.
 */
export function cursorFor(s: TransformState, hit: Hit | null, viewRotation = 0): string | null {
  if (hit === null) {
    return null;
  }
  switch (hit.kind) {
    case 'ref':
      return 'crosshair';
    case 'move':
      return 'move';
    case 'rotate':
      return 'alias';
    case 'handle': {
      const center = applyMatrix(stateMatrix(s), boundsCenter(s.bounds));
      const at = handlePoint(s, hit.handle);
      const degrees = (Math.atan2(at.y - center.y, at.x - center.x) * 180) / Math.PI + viewRotation;
      // A direction and its opposite give the same cursor: 4 buckets of 45 degrees.
      const bucket = ((Math.round(degrees / 45) % 4) + 4) % 4;
      return RESIZE_CURSORS[bucket] ?? 'move';
    }
  }
}

// ---- Gestures ----

/** Keeps the matrix result and moves the reference point to `ref`. */
export function moveReference(s: TransformState, ref: Point): TransformState {
  const m = stateMatrix(s);
  // The translation part of the rotate-scale: A * (ref' - ref).
  const dx = ref.x - s.ref.x;
  const dy = ref.y - s.ref.y;
  const rsx = m[0] * dx + m[2] * dy;
  const rsy = m[1] * dx + m[3] * dy;
  return { ...s, ref, tx: s.tx + s.ref.x - ref.x + rsx, ty: s.ty + s.ref.y - ref.y + rsy };
}

/** The reference point at a grid position: column and row 0..2 over the bounds. */
export function referenceAtGrid(s: TransformState, column: number, row: number): TransformState {
  const b = s.bounds;
  return moveReference(s, { x: b.x + (b.width * column) / 2, y: b.y + (b.height * row) / 2 });
}

/** Moves the state by a canvas distance. */
export function translate(s: TransformState, dx: number, dy: number): TransformState {
  return { ...s, tx: s.tx + dx, ty: s.ty + dy };
}

export interface ScaleOptions {
  /** Keep the aspect ratio of the start state. */
  readonly keepAspect: boolean;
  /** Scale about the box center instead of the reference point. */
  readonly aboutCenter: boolean;
}

/**
 * Scales with a handle. `start` is the state at the pointer-down, `pointer`
 * is the canvas point now, and `grab` is the canvas point at the pointer-down.
 * The grab offset from the handle is kept, so the handle does not jump.
 */
export function scaleByHandle(start: TransformState, handle: HandleId, grab: Point, pointer: Point, options: ScaleOptions): TransformState {
  const m = stateMatrix(start);
  const handleAt = applyMatrix(m, handleLocal(start.bounds, handle));
  const wanted = { x: pointer.x + (handleAt.x - grab.x), y: pointer.y + (handleAt.y - grab.y) };
  const anchorLocal = options.aboutCenter ? boundsCenter(start.bounds) : start.ref;
  // The image of the anchor stays where it is.
  const fixed = applyMatrix(m, anchorLocal);
  const cos = Math.cos(start.angle);
  const sin = Math.sin(start.angle);
  // The wanted vector from the fixed point, turned back by the angle.
  const vx = cos * (wanted.x - fixed.x) + sin * (wanted.y - fixed.y);
  const vy = -sin * (wanted.x - fixed.x) + cos * (wanted.y - fixed.y);
  const local = handleLocal(start.bounds, handle);
  const ux = local.x - anchorLocal.x;
  const uy = local.y - anchorLocal.y;
  const [sideX, sideY] = HANDLE_SIDE[handle];
  let sx = sideX !== 0 && Math.abs(ux) > EPSILON ? vx / ux : start.sx;
  let sy = sideY !== 0 && Math.abs(uy) > EPSILON ? vy / uy : start.sy;
  if (options.keepAspect) {
    const fx = sx / (start.sx || 1);
    const fy = sy / (start.sy || 1);
    // The axis that the handle moves most decides; an edge handle has one axis.
    const factor = sideX === 0 ? fy : sideY === 0 ? fx : Math.abs(fx) >= Math.abs(fy) ? fx : fy;
    sx = start.sx * factor;
    sy = start.sy * factor;
  }
  return keepImage({ ...start, sx, sy }, anchorLocal, fixed);
}

/** Changes the translation so that the image of `anchor` is `target`. */
function keepImage(s: TransformState, anchor: Point, target: Point): TransformState {
  const m = stateMatrix({ ...s, tx: 0, ty: 0 });
  const at = applyMatrix(m, anchor);
  return { ...s, tx: target.x - at.x, ty: target.y - at.y };
}

/** The angle (radians) from `center` to `p`. */
function angleOf(center: Point, p: Point): number {
  return Math.atan2(p.y - center.y, p.x - center.x);
}

/** Rounds an angle in radians to the nearest snap step. */
export function snapAngleRadians(angle: number, stepDegrees = ANGLE_SNAP_DEGREES): number {
  const step = (stepDegrees * Math.PI) / 180;
  return Math.round(angle / step) * step;
}

/**
 * Rotates about the reference point. The angle follows the pointer around the
 * reference point. With `snap` the total angle goes to a 15 degree step.
 */
export function rotateByPointer(start: TransformState, grab: Point, pointer: Point, snap: boolean): TransformState {
  const center = referenceImage(start);
  const raw = start.angle + angleOf(center, pointer) - angleOf(center, grab);
  return { ...start, angle: snap ? snapAngleRadians(raw) : raw };
}

// ---- Numeric fields ----

/** The values of the options bar. */
export interface TransformFields {
  /** The reference point position after the transform. */
  readonly x: number;
  readonly y: number;
  /** Percent. */
  readonly width: number;
  readonly height: number;
  /** Degrees, in (-180, 180]. */
  readonly angle: number;
}

/** Turns degrees into (-180, 180]. */
export function normalizeDegrees(degrees: number): number {
  const wrapped = ((((degrees + 180) % 360) + 360) % 360) - 180;
  return wrapped === -180 ? 180 : wrapped;
}

export function stateFields(s: TransformState): TransformFields {
  const at = referenceImage(s);
  return { x: at.x, y: at.y, width: s.sx * 100, height: s.sy * 100, angle: normalizeDegrees((s.angle * 180) / Math.PI) };
}

/**
 * Applies the fields that changed. A value that is not finite, or a scale
 * with a too small absolute value, leaves the state as it is.
 */
export function withFields(s: TransformState, fields: Partial<TransformFields>): TransformState {
  let next = s;
  const valid = (value: number | undefined): value is number => value !== undefined && Number.isFinite(value);
  if (valid(fields.x)) {
    next = { ...next, tx: fields.x - next.ref.x };
  }
  if (valid(fields.y)) {
    next = { ...next, ty: fields.y - next.ref.y };
  }
  if (valid(fields.width) && Math.abs(fields.width) / 100 >= MIN_SCALE) {
    next = { ...next, sx: fields.width / 100 };
  }
  if (valid(fields.height) && Math.abs(fields.height) / 100 >= MIN_SCALE) {
    next = { ...next, sy: fields.height / 100 };
  }
  if (valid(fields.angle)) {
    next = { ...next, angle: (normalizeDegrees(fields.angle) * Math.PI) / 180 };
  }
  return next;
}

/** The arrow key distance in canvas pixels. */
export function nudgeStep(shift: boolean): number {
  return shift ? 10 : 1;
}
