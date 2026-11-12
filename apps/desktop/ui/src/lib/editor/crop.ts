/**
 * Pure geometry of the editor crop tool (ADR 0032).
 *
 * A `CropBox` is a rectangle in canvas pixels. `angle` turns it about its
 * center, in degrees, clockwise on the screen (y points down). The `cropCanvas`
 * edit maps the new pixel (u, v) to the canvas point
 * `center + R(angle) * (u + 0.5 - width / 2, v + 0.5 - height / 2)`.
 *
 * The box can reach outside of the canvas. With "delete cropped pixels" off
 * the shell keeps the pixels outside of the new canvas.
 */
import type { EditDto } from './api';
import { HANDLES, type HandleId, type Interpolation } from './transform';
import type { Point, Size } from './view';

export interface CropBox {
  readonly centerX: number;
  readonly centerY: number;
  readonly width: number;
  readonly height: number;
  /** Degrees, -45..=45, clockwise on the screen. */
  readonly angle: number;
}

/** The largest angle of the crop. */
export const MAX_CROP_ANGLE = 45;
/** The smallest side of the box in canvas pixels. */
export const MIN_CROP_SIZE = 1;
/** The largest side of the box. A copy of `MAX_DIMENSION` in `expose-editor`. */
export const MAX_CROP_SIZE = 30_000;
/** The distance in screen pixels from a handle center that still hits the handle. */
export const CROP_HANDLE_RADIUS = 8;

export type CropAspect = 'free' | 'original' | '1:1' | '4:3' | '3:2' | '16:9';

export const CROP_ASPECTS: readonly { readonly id: CropAspect; readonly label: string }[] = [
  { id: 'free', label: 'Free' },
  { id: 'original', label: 'Original' },
  { id: '1:1', label: '1:1' },
  { id: '4:3', label: '4:3' },
  { id: '3:2', label: '3:2' },
  { id: '16:9', label: '16:9' },
];

export type CropEdit = Extract<EditDto, { type: 'cropCanvas' }>;

export type CropHit = { readonly kind: 'handle'; readonly handle: HandleId } | { readonly kind: 'move' } | { readonly kind: 'rotate' };

// ---- Basics ----

/** The box of the whole canvas. */
export function fullBox(size: Size): CropBox {
  return { centerX: size.width / 2, centerY: size.height / 2, width: size.width, height: size.height, angle: 0 };
}

/** True if the box is the whole canvas at angle 0. */
export function isFullBox(box: CropBox, size: Size): boolean {
  const full = fullBox(size);
  const near = (a: number, b: number) => Math.abs(a - b) < 1e-6;
  return (
    near(box.angle, 0) &&
    near(box.width, full.width) &&
    near(box.height, full.height) &&
    near(box.centerX, full.centerX) &&
    near(box.centerY, full.centerY)
  );
}

/** Turns a vector clockwise on the screen (y down). */
function turn(x: number, y: number, degrees: number): Point {
  const rad = (degrees * Math.PI) / 180;
  const c = Math.cos(rad);
  const s = Math.sin(rad);
  return { x: x * c - y * s, y: x * s + y * c };
}

/** A point of the box frame (origin at the center, axes along the box) as a canvas point. */
export function boxToCanvas(box: CropBox, local: Point): Point {
  const r = turn(local.x, local.y, box.angle);
  return { x: box.centerX + r.x, y: box.centerY + r.y };
}

/** A canvas point in the frame of the box. */
export function canvasToBox(box: CropBox, p: Point): Point {
  return turn(p.x - box.centerX, p.y - box.centerY, -box.angle);
}

/** The corners in the order top-left, top-right, bottom-right, bottom-left. */
export function cropCorners(box: CropBox): [Point, Point, Point, Point] {
  const hw = box.width / 2;
  const hh = box.height / 2;
  return [
    boxToCanvas(box, { x: -hw, y: -hh }),
    boxToCanvas(box, { x: hw, y: -hh }),
    boxToCanvas(box, { x: hw, y: hh }),
    boxToCanvas(box, { x: -hw, y: hh }),
  ];
}

/** The direction of a handle in the box frame: -1, 0, or 1 for each axis. */
function handleSigns(handle: HandleId): { sx: number; sy: number } {
  const sx = handle.includes('l') ? -1 : handle.includes('r') ? 1 : 0;
  const sy = handle.includes('t') ? -1 : handle.includes('b') ? 1 : 0;
  return { sx, sy };
}

/** The canvas position of a handle. */
export function cropHandlePoint(box: CropBox, handle: HandleId): Point {
  const { sx, sy } = handleSigns(handle);
  return boxToCanvas(box, { x: (sx * box.width) / 2, y: (sy * box.height) / 2 });
}

/** The four lines of the rule of thirds: `[from, to]` pairs in canvas pixels. */
export function thirdsLines(box: CropBox): [Point, Point][] {
  const lines: [Point, Point][] = [];
  const hw = box.width / 2;
  const hh = box.height / 2;
  for (const t of [-1 / 6, 1 / 6]) {
    lines.push([boxToCanvas(box, { x: box.width * t, y: -hh }), boxToCanvas(box, { x: box.width * t, y: hh })]);
    lines.push([boxToCanvas(box, { x: -hw, y: box.height * t }), boxToCanvas(box, { x: hw, y: box.height * t })]);
  }
  return lines;
}

// ---- Hit test and cursor ----

/** What the pointer at the canvas point `p` grabs: a handle, the inside (move), or the outside (rotate). */
export function cropHitTest(box: CropBox, p: Point, zoom: number): CropHit {
  const radius = CROP_HANDLE_RADIUS / (zoom > 0 ? zoom : 1);
  let best: { handle: HandleId; d: number } | null = null;
  for (const handle of HANDLES) {
    const at = cropHandlePoint(box, handle);
    const d = Math.hypot(p.x - at.x, p.y - at.y);
    if (d <= radius && (best === null || d < best.d)) {
      best = { handle, d };
    }
  }
  if (best !== null) {
    return { kind: 'handle', handle: best.handle };
  }
  const local = canvasToBox(box, p);
  if (Math.abs(local.x) <= box.width / 2 && Math.abs(local.y) <= box.height / 2) {
    return { kind: 'move' };
  }
  return { kind: 'rotate' };
}

const RESIZE_CURSORS = ['ew-resize', 'nwse-resize', 'ns-resize', 'nesw-resize'] as const;

/** The CSS cursor of a hit. A resize cursor follows the handle direction on the screen. */
export function cropCursor(box: CropBox, hit: CropHit, viewRotation = 0): string {
  switch (hit.kind) {
    case 'move':
      return 'move';
    case 'rotate':
      return 'alias';
    case 'handle': {
      const { sx, sy } = handleSigns(hit.handle);
      const degrees = (Math.atan2(sy, sx) * 180) / Math.PI + box.angle + viewRotation;
      // A direction and its opposite give the same cursor: 4 buckets of 45 degrees.
      const bucket = ((Math.round(degrees / 45) % 4) + 4) % 4;
      return RESIZE_CURSORS[bucket] ?? 'move';
    }
  }
}

// ---- Rounding ----

/**
 * At angle 0 rounds the four edges to whole canvas pixels, so that the crop is
 * a lossless offset. At other angles the box stays as it is (the edit rounds
 * the size). Each side is at least 1 px.
 */
export function roundBox(box: CropBox): CropBox {
  if (box.angle !== 0) {
    return box;
  }
  const left = Math.round(box.centerX - box.width / 2);
  const top = Math.round(box.centerY - box.height / 2);
  const width = Math.max(MIN_CROP_SIZE, Math.round(box.width));
  const height = Math.max(MIN_CROP_SIZE, Math.round(box.height));
  return { ...box, centerX: left + width / 2, centerY: top + height / 2, width, height };
}

// ---- Aspect ----

/**
 * The width/height ratio that the aspect locks, or null for free. `portrait`
 * turns the ratio around. `original` is the ratio of the canvas.
 */
export function aspectRatio(aspect: CropAspect, portrait: boolean, canvas: Size): number | null {
  let ratio: number;
  if (aspect === 'free') {
    return null;
  }
  if (aspect === 'original') {
    ratio = canvas.width / canvas.height;
  } else {
    const [a, b] = aspect.split(':').map(Number);
    ratio = (a ?? 1) / (b ?? 1);
  }
  if (!Number.isFinite(ratio) || ratio <= 0) {
    return null;
  }
  return portrait ? 1 / ratio : ratio;
}

/** Limits the sides to 1..=MAX. With a ratio, both sides scale together. */
function clampSides(width: number, height: number, ratio: number | null): { width: number; height: number } {
  if (ratio === null) {
    return { width: Math.min(MAX_CROP_SIZE, Math.max(MIN_CROP_SIZE, width)), height: Math.min(MAX_CROP_SIZE, Math.max(MIN_CROP_SIZE, height)) };
  }
  const up = Math.max(MIN_CROP_SIZE / width, MIN_CROP_SIZE / height, 1);
  const down = Math.min(MAX_CROP_SIZE / (width * up), MAX_CROP_SIZE / (height * up), 1);
  return { width: width * up * down, height: height * up * down };
}

/** Makes the box match a ratio about its center. The new box fits inside the old one. */
export function fitAspect(box: CropBox, ratio: number | null): CropBox {
  if (ratio === null) {
    return box;
  }
  let width = box.width;
  let height = width / ratio;
  if (height > box.height) {
    height = box.height;
    width = height * ratio;
  }
  return roundBox({ ...box, ...clampSides(width, height, ratio) });
}

/** Swaps width and height about the center. */
export function swapBox(box: CropBox): CropBox {
  return roundBox({ ...box, width: box.height, height: box.width });
}

// ---- Gestures ----

/** Moves the box by a canvas vector. */
export function translateBox(box: CropBox, dx: number, dy: number): CropBox {
  return roundBox({ ...box, centerX: box.centerX + dx, centerY: box.centerY + dy });
}

/**
 * Drags a handle to `pointer` (canvas point). The opposite handle (or edge)
 * stays fixed, measured in the rotated frame of the box. `ratio` keeps the
 * width/height ratio. `aboutCenter` keeps the center instead.
 */
export function resizeByHandle(start: CropBox, handle: HandleId, pointer: Point, ratio: number | null, aboutCenter = false): CropBox {
  const { sx, sy } = handleSigns(handle);
  const p = canvasToBox(start, pointer);
  // The wanted sides from the pointer position in the box frame.
  let w = sx === 0 ? start.width : aboutCenter ? 2 * sx * p.x : sx * p.x + start.width / 2;
  let h = sy === 0 ? start.height : aboutCenter ? 2 * sy * p.y : sy * p.y + start.height / 2;
  w = Math.max(w, 1e-9);
  h = Math.max(h, 1e-9);
  if (ratio !== null) {
    if (sx !== 0 && sy !== 0) {
      // A corner: the larger side wins.
      if (w / h > ratio) {
        h = w / ratio;
      } else {
        w = h * ratio;
      }
    } else if (sx !== 0) {
      h = w / ratio;
    } else {
      w = h * ratio;
    }
  }
  const sides = clampSides(w, h, ratio);
  // The center moves along the box axes by half of the change of a side that the handle controls.
  const dx = aboutCenter || sx === 0 ? 0 : (sx * (sides.width - start.width)) / 2;
  const dy = aboutCenter || sy === 0 ? 0 : (sy * (sides.height - start.height)) / 2;
  const center = boxToCanvas(start, { x: dx, y: dy });
  const next: CropBox = { ...start, centerX: center.x, centerY: center.y, ...sides };
  return start.angle === 0 ? roundEdges(next, start, sx, sy, aboutCenter) : next;
}

/**
 * Rounds the edges of a resized box at angle 0. The fixed edges stay as they
 * were. The moving edges fall on whole pixels, at least 1 px from the fixed ones.
 */
function roundEdges(box: CropBox, start: CropBox, sx: number, sy: number, aboutCenter: boolean): CropBox {
  if (aboutCenter) {
    return roundBox(box);
  }
  let left = Math.round(box.centerX - box.width / 2);
  let right = Math.round(box.centerX + box.width / 2);
  let top = Math.round(box.centerY - box.height / 2);
  let bottom = Math.round(box.centerY + box.height / 2);
  if (sx < 0) {
    right = Math.round(start.centerX + start.width / 2);
    left = Math.min(left, right - MIN_CROP_SIZE);
  } else if (sx > 0) {
    left = Math.round(start.centerX - start.width / 2);
    right = Math.max(right, left + MIN_CROP_SIZE);
  }
  if (sy < 0) {
    bottom = Math.round(start.centerY + start.height / 2);
    top = Math.min(top, bottom - MIN_CROP_SIZE);
  } else if (sy > 0) {
    top = Math.round(start.centerY - start.height / 2);
    bottom = Math.max(bottom, top + MIN_CROP_SIZE);
  }
  return { ...box, centerX: (left + right) / 2, centerY: (top + bottom) / 2, width: right - left, height: bottom - top };
}

/** Limits an angle to the allowed range. A value that is not a number gives 0. */
export function clampAngle(degrees: number): number {
  if (!Number.isFinite(degrees)) {
    return 0;
  }
  return Math.min(MAX_CROP_ANGLE, Math.max(-MAX_CROP_ANGLE, degrees));
}

/**
 * Turns the box about its center by the angle between `grab` and `pointer`
 * (both canvas points). `snap` rounds the new angle to whole degrees.
 */
export function rotateByPointer(start: CropBox, grab: Point, pointer: Point, snap: boolean): CropBox {
  const a0 = Math.atan2(grab.y - start.centerY, grab.x - start.centerX);
  const a1 = Math.atan2(pointer.y - start.centerY, pointer.x - start.centerX);
  let delta = ((a1 - a0) * 180) / Math.PI;
  delta = ((((delta + 180) % 360) + 360) % 360) - 180;
  let angle = start.angle + delta;
  if (snap) {
    angle = Math.round(angle);
  }
  return roundBox({ ...start, angle: clampAngle(angle) });
}

// ---- Fields ----

export interface CropFields {
  width: number;
  height: number;
  angle: number;
}

export function boxFields(box: CropBox): CropFields {
  return { width: box.width, height: box.height, angle: box.angle };
}

/**
 * Sets the field W, H, or the angle. The box keeps its center. With a ratio,
 * the other side follows the changed side. A value that is not a finite
 * number is dropped.
 */
export function withFields(box: CropBox, fields: Partial<CropFields>, ratio: number | null): CropBox {
  let { width, height } = box;
  const wantW = fields.width;
  const wantH = fields.height;
  if (wantW !== undefined && Number.isFinite(wantW)) {
    width = wantW;
    if (ratio !== null) {
      height = width / ratio;
    }
  } else if (wantH !== undefined && Number.isFinite(wantH)) {
    height = wantH;
    if (ratio !== null) {
      width = height * ratio;
    }
  }
  const angle = fields.angle !== undefined && Number.isFinite(fields.angle) ? clampAngle(fields.angle) : box.angle;
  return roundBox({ ...box, ...clampSides(width, height, ratio), angle });
}

// ---- Edit ----

/** The `cropCanvas` edit of a box. The size is whole pixels. */
export function cropEdit(box: CropBox, deleteCropped: boolean, interpolation: Interpolation): CropEdit {
  const side = (value: number) => Math.min(MAX_CROP_SIZE, Math.max(MIN_CROP_SIZE, Math.round(value)));
  return {
    type: 'cropCanvas',
    centerX: box.centerX,
    centerY: box.centerY,
    width: side(box.width),
    height: side(box.height),
    angle: box.angle,
    deleteCropped,
    interpolation,
  };
}
