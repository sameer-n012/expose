/**
 * Crop rectangle geometry for the crop tool.
 *
 * The backend geometry stage (ADR 0011, `crates/expose-develop/src/geometry.rs`)
 * flips the source, rotates it by `angle` (degrees, clockwise on the screen)
 * about the image center, and then crops to `rect` in a `W x H` frame with
 * the same center. `rect` is normalized to that frame (origin top left).
 * A sample point outside the source becomes transparent, so the crop tool
 * must keep the rect inside the rotated image.
 *
 * `maxInscribedRect` and `rectIsInside` are ports of the Rust functions with
 * the same formulas. The other functions move, resize, and shrink a rect and
 * keep it inside the rotated image.
 *
 * Inside the rotated image and inside the frame is a convex region. A rect
 * is inside when its four corners are inside. Thus, for a rect A that is
 * inside and a rect B, the rects between A and B (linear blend of the edges)
 * that are inside form one interval that starts at A. A binary search on
 * the blend amount finds its end (`clampAlong`).
 */

/** A rectangle normalized to the `W x H` frame (`0..1` on each axis, origin top left). */
export interface NormalizedRect {
  readonly left: number;
  readonly top: number;
  readonly width: number;
  readonly height: number;
}

/** The full frame. */
export const FULL_RECT: NormalizedRect = { left: 0, top: 0, width: 1, height: 1 };

/** The source size in pixels and the straighten angle in degrees. */
export interface CropFrame {
  readonly width: number;
  readonly height: number;
  readonly angle: number;
}

/** A handle of the crop frame: an edge (`n`, `s`, `e`, `w`) or a corner. */
export type CropHandle = 'n' | 's' | 'e' | 'w' | 'ne' | 'nw' | 'se' | 'sw';

export const CROP_HANDLES: readonly CropHandle[] = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'];

/** Tolerance in source pixels for `rectIsInside` (equal to the Rust constant). */
export const INSIDE_TOLERANCE_PIXELS = 0.01;

/** The smallest width and height of the crop rect, as a part of the frame. */
export const MIN_RECT_SIZE = 0.02;

/** The number of steps of each binary search. 2^-40 is far below one pixel. */
const SEARCH_STEPS = 40;

/** Incorrect input to `maxInscribedRect`. */
export class CropGeometryError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'CropGeometryError';
  }
}

function toRadians(degrees: number): number {
  return (degrees * Math.PI) / 180;
}

function isFiniteRect(rect: NormalizedRect): boolean {
  return [rect.left, rect.top, rect.width, rect.height].every(Number.isFinite);
}

/**
 * The largest rect with the pixel aspect `aspect` (width / height in
 * pixels) that is centered on the image center and lies fully inside the
 * rotated image and inside the frame. `aspect = null` means the source
 * aspect. Port of `max_inscribed_rect`.
 *
 * For half sizes `hw = r * hh`, `c = |cos|`, `s = |sin|`, the rect is inside
 * the rotated image when `hw*c + hh*s <= W/2` and `hw*s + hh*c <= H/2`.
 * Both limits are linear in `hh`, so the largest `hh` is the smallest of
 * the limits (the frame limits included).
 *
 * @throws CropGeometryError when the size is not greater than 0, the angle
 *   is not finite, or the aspect is not a finite number greater than 0.
 */
export function maxInscribedRect(
  width: number,
  height: number,
  angleDegrees: number,
  aspect: number | null = null,
): NormalizedRect {
  if (!(width > 0) || !(height > 0) || !Number.isFinite(width) || !Number.isFinite(height)) {
    throw new CropGeometryError(`The source size ${width} x ${height} is not valid.`);
  }
  if (!Number.isFinite(angleDegrees)) {
    throw new CropGeometryError('The crop angle is not finite.');
  }
  if (aspect !== null && !(Number.isFinite(aspect) && aspect > 0)) {
    throw new CropGeometryError('The aspect ratio must be a finite number greater than 0.');
  }
  const w = width;
  const h = height;
  const r = aspect ?? w / h;
  const rad = toRadians(angleDegrees);
  const s = Math.abs(Math.sin(rad));
  const c = Math.abs(Math.cos(rad));
  const hh = Math.min(w / (2 * (r * c + s)), h / (2 * (r * s + c)), h / 2, w / (2 * r));
  const hw = r * hh;
  const nw = Math.min((2 * hw) / w, 1);
  const nh = Math.min((2 * hh) / h, 1);
  return { left: (1 - nw) / 2, top: (1 - nh) / 2, width: nw, height: nh };
}

/**
 * True when `rect` lies fully inside the rotated image and inside the frame.
 * The tolerance is 0.01 source pixels (`tolerancePixels`). A rect with a zero or negative size,
 * a value that is not finite, a zero-size source, or an angle that is not
 * finite gives `false`. Port of `rect_is_inside`.
 */
export function rectIsInside(
  width: number,
  height: number,
  angleDegrees: number,
  rect: NormalizedRect,
  tolerancePixels: number = INSIDE_TOLERANCE_PIXELS,
): boolean {
  if (
    !(width > 0) ||
    !(height > 0) ||
    !Number.isFinite(angleDegrees) ||
    !isFiniteRect(rect) ||
    rect.width <= 0 ||
    rect.height <= 0
  ) {
    return false;
  }
  const w = width;
  const h = height;
  const tolerance = tolerancePixels;
  const left = rect.left * w;
  const top = rect.top * h;
  const right = left + rect.width * w;
  const bottom = top + rect.height * h;
  const inRange = (v: number, max: number): boolean => v >= -tolerance && v <= max + tolerance;
  if (!(inRange(left, w) && inRange(right, w) && inRange(top, h) && inRange(bottom, h))) {
    return false;
  }
  // The flips do not change the rotated region, so only the rotation counts.
  const rad = toRadians(angleDegrees);
  const sin = Math.sin(rad);
  const cos = Math.cos(rad);
  const cx = w / 2;
  const cy = h / 2;
  const corners: ReadonlyArray<readonly [number, number]> = [
    [left, top],
    [right, top],
    [left, bottom],
    [right, bottom],
  ];
  return corners.every(([px, py]) => {
    const dx = px - cx;
    const dy = py - cy;
    const sx = cos * dx + sin * dy + cx;
    const sy = -sin * dx + cos * dy + cy;
    return inRange(sx, w) && inRange(sy, h);
  });
}

/**
 * The tolerance in source pixels of the crop tool edits. It is much smaller
 * than `INSIDE_TOLERANCE_PIXELS`, so a clamped rect stops at the edge and
 * not 0.01 pixels outside it. It only absorbs float rounding.
 */
export const EDIT_TOLERANCE_PIXELS = 1e-6;

/** `rectIsInside` for a frame, with `EDIT_TOLERANCE_PIXELS`. */
export function insideFrame(frame: CropFrame, rect: NormalizedRect): boolean {
  return rectIsInside(frame.width, frame.height, frame.angle, rect, EDIT_TOLERANCE_PIXELS);
}

/** The pixel aspect (width / height in pixels) of `rect` in the frame. */
export function pixelAspect(frame: Pick<CropFrame, 'width' | 'height'>, rect: NormalizedRect): number {
  return (rect.width * frame.width) / (rect.height * frame.height);
}

/** The normalized aspect (`rect.width / rect.height`) of a pixel aspect in the frame. */
function normalizedAspect(frame: Pick<CropFrame, 'width' | 'height'>, pixelRatio: number): number {
  return (pixelRatio * frame.height) / frame.width;
}

/** Scales `rect` by `scale` about its center. */
export function scaleRect(rect: NormalizedRect, scale: number): NormalizedRect {
  const width = rect.width * scale;
  const height = rect.height * scale;
  return {
    left: rect.left + (rect.width - width) / 2,
    top: rect.top + (rect.height - height) / 2,
    width,
    height,
  };
}

/** The linear blend of the edges of two rects (`t = 0`: `a`, `t = 1`: `b`). */
export function lerpRect(a: NormalizedRect, b: NormalizedRect, t: number): NormalizedRect {
  const mix = (x: number, y: number): number => x + (y - x) * t;
  return {
    left: mix(a.left, b.left),
    top: mix(a.top, b.top),
    width: mix(a.width, b.width),
    height: mix(a.height, b.height),
  };
}

/**
 * Goes from `from` toward `to` as far as the rect stays inside. `from` must
 * be inside. When `to` is inside, the result is `to`.
 */
function clampAlong(frame: CropFrame, from: NormalizedRect, to: NormalizedRect): NormalizedRect {
  if (insideFrame(frame, to)) {
    return to;
  }
  let lo = 0;
  let hi = 1;
  for (let i = 0; i < SEARCH_STEPS; i++) {
    const mid = (lo + hi) / 2;
    if (insideFrame(frame, lerpRect(from, to, mid))) {
      lo = mid;
    } else {
      hi = mid;
    }
  }
  return lerpRect(from, to, lo);
}

/**
 * Returns `rect` when it is inside. Else it shrinks `rect` about its center
 * to the largest size that is inside, and keeps the aspect. When the center
 * itself is not inside (or the result is smaller than `MIN_RECT_SIZE`),
 * the rect also moves toward the largest centered rect of the same aspect,
 * only as far as necessary.
 */
export function fitRect(frame: CropFrame, rect: NormalizedRect): NormalizedRect {
  if (insideFrame(frame, rect)) {
    return rect;
  }
  if (!isFiniteRect(rect) || rect.width <= 0 || rect.height <= 0) {
    return maxInscribedRect(frame.width, frame.height, frame.angle);
  }
  const target = maxInscribedRect(frame.width, frame.height, frame.angle, pixelAspect(frame, rect));
  const smallest = 1e-6;
  if (insideFrame(frame, scaleRect(rect, smallest))) {
    let lo = smallest;
    let hi = 1;
    for (let i = 0; i < SEARCH_STEPS; i++) {
      const mid = (lo + hi) / 2;
      if (insideFrame(frame, scaleRect(rect, mid))) {
        lo = mid;
      } else {
        hi = mid;
      }
    }
    const scaled = scaleRect(rect, lo);
    if (scaled.width >= MIN_RECT_SIZE && scaled.height >= MIN_RECT_SIZE) {
      return scaled;
    }
  }
  // The smallest blend amount that is inside: the target is inside, and the
  // inside amounts form one interval that ends at 1.
  let lo = 0;
  let hi = 1;
  for (let i = 0; i < SEARCH_STEPS; i++) {
    const mid = (lo + hi) / 2;
    if (insideFrame(frame, lerpRect(rect, target, mid))) {
      hi = mid;
    } else {
      lo = mid;
    }
  }
  return lerpRect(rect, target, hi);
}

/**
 * Moves `rect` by `(dx, dy)` (normalized). The move stops where the rect
 * would leave the rotated image. Each axis is limited on its own, so the
 * rect slides along an edge. When `rect` is not inside at the start, the
 * moved rect is fitted with `fitRect`.
 */
export function moveRect(frame: CropFrame, rect: NormalizedRect, dx: number, dy: number): NormalizedRect {
  if (!insideFrame(frame, rect)) {
    return fitRect(frame, { ...rect, left: rect.left + dx, top: rect.top + dy });
  }
  const afterX = clampAlong(frame, rect, { ...rect, left: rect.left + dx });
  return clampAlong(frame, afterX, { ...afterX, top: afterX.top + dy });
}

interface Edges {
  l: number;
  t: number;
  r: number;
  b: number;
}

function edgesOf(rect: NormalizedRect): Edges {
  return { l: rect.left, t: rect.top, r: rect.left + rect.width, b: rect.top + rect.height };
}

function rectOf(e: Edges): NormalizedRect {
  return { left: e.l, top: e.t, width: e.r - e.l, height: e.b - e.t };
}

/**
 * Resizes `rect` with a handle dragged by `(dx, dy)` (normalized). The
 * opposite edge or corner stays fixed. `ratio` is the pixel aspect to keep
 * (width / height in pixels), or null for a free resize. The width and the
 * height stay at least `MIN_RECT_SIZE`. The change stops where the rect
 * would leave the rotated image.
 *
 * With a ratio, a corner follows the axis that changes more; an edge
 * changes the other size about the center of that axis.
 */
export function resizeRect(
  frame: CropFrame,
  rect: NormalizedRect,
  handle: CropHandle,
  dx: number,
  dy: number,
  ratio: number | null,
): NormalizedRect {
  const e = edgesOf(rect);
  const west = handle.includes('w');
  const east = handle.includes('e');
  const north = handle.includes('n');
  const south = handle.includes('s');

  if (ratio === null) {
    const x: Edges = { ...e };
    if (east) x.r = Math.max(e.r + dx, e.l + MIN_RECT_SIZE);
    if (west) x.l = Math.min(e.l + dx, e.r - MIN_RECT_SIZE);
    const y: Edges = { ...x };
    if (south) y.b = Math.max(e.b + dy, e.t + MIN_RECT_SIZE);
    if (north) y.t = Math.min(e.t + dy, e.b - MIN_RECT_SIZE);
    if (!insideFrame(frame, rect)) {
      return fitRect(frame, rectOf(y));
    }
    const afterX = clampAlong(frame, rect, rectOf(x));
    const ax = edgesOf(afterX);
    const yOnly: Edges = { ...ax, t: north ? y.t : ax.t, b: south ? y.b : ax.b };
    return clampAlong(frame, afterX, rectOf(yOnly));
  }

  const k = normalizedAspect(frame, ratio);
  const minWidth = Math.max(MIN_RECT_SIZE, MIN_RECT_SIZE * k);
  const draggedWidth = east ? e.r + dx - e.l : west ? e.r - (e.l + dx) : rect.width;
  const draggedHeight = south ? e.b + dy - e.t : north ? e.b - (e.t + dy) : rect.height;
  let width: number;
  if ((east || west) && (north || south)) {
    const changeX = Math.abs(draggedWidth / rect.width - 1);
    const changeY = Math.abs(draggedHeight / rect.height - 1);
    width = changeX >= changeY ? draggedWidth : draggedHeight * k;
  } else if (east || west) {
    width = draggedWidth;
  } else {
    width = draggedHeight * k;
  }
  width = Math.max(width, minWidth);
  const height = width / k;

  let left: number;
  if (east) left = e.l;
  else if (west) left = e.r - width;
  else left = (e.l + e.r) / 2 - width / 2;
  let top: number;
  if (south) top = e.t;
  else if (north) top = e.b - height;
  else top = (e.t + e.b) / 2 - height / 2;

  const proposed: NormalizedRect = { left, top, width, height };
  if (!insideFrame(frame, rect)) {
    return fitRect(frame, proposed);
  }
  return clampAlong(frame, rect, proposed);
}

/**
 * The rect for a new angle. A rect that was the largest centered rect of
 * its aspect stays the largest one (it grows and shrinks with the angle).
 * Another rect stays as it is when it fits, else it shrinks with `fitRect`.
 */
export function rectForAngle(
  frame: CropFrame,
  rect: NormalizedRect,
  angle: number,
  ratio: number | null,
): NormalizedRect {
  const aspect = ratio ?? pixelAspect(frame, rect);
  const next: CropFrame = { ...frame, angle };
  const oldMax = maxInscribedRect(frame.width, frame.height, frame.angle, aspect);
  if (rectsClose(rect, oldMax)) {
    return maxInscribedRect(frame.width, frame.height, angle, aspect);
  }
  return fitRect(next, rect);
}

/**
 * The rect for a new aspect: the largest centered rect of that pixel
 * aspect that is inside. `null` (free) keeps the rect.
 */
export function rectForAspect(frame: CropFrame, rect: NormalizedRect, ratio: number | null): NormalizedRect {
  if (ratio === null) {
    return fitRect(frame, rect);
  }
  return maxInscribedRect(frame.width, frame.height, frame.angle, ratio);
}

/**
 * Swaps the pixel width and height of `rect` about its center (portrait to
 * landscape). The result is scaled down to the frame and fitted.
 */
export function swapRect(frame: CropFrame, rect: NormalizedRect): NormalizedRect {
  let width = (rect.height * frame.height) / frame.width;
  let height = (rect.width * frame.width) / frame.height;
  const over = Math.max(width, height, 1);
  width /= over;
  height /= over;
  const cx = rect.left + rect.width / 2;
  const cy = rect.top + rect.height / 2;
  return fitRect(frame, { left: cx - width / 2, top: cy - height / 2, width, height });
}

/** Mirrors `rect` in the frame (for a flip of the image). */
export function mirrorRect(rect: NormalizedRect, horizontal: boolean): NormalizedRect {
  return horizontal
    ? { ...rect, left: 1 - rect.left - rect.width }
    : { ...rect, top: 1 - rect.top - rect.height };
}

/** True when all four values differ by at most `tolerance`. */
export function rectsClose(a: NormalizedRect, b: NormalizedRect, tolerance = 1e-4): boolean {
  return (
    Math.abs(a.left - b.left) <= tolerance &&
    Math.abs(a.top - b.top) <= tolerance &&
    Math.abs(a.width - b.width) <= tolerance &&
    Math.abs(a.height - b.height) <= tolerance
  );
}
