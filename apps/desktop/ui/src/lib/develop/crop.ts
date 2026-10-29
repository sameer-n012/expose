/**
 * The `crop` field of the develop settings and the edits of the crop tool.
 *
 * The types mirror `expose_core::develop::Crop` (serde, snake_case names):
 *
 * ```json
 * { "rect": { "left": 0, "top": 0, "width": 1, "height": 1 },
 *   "angle": 0, "flip_horizontal": false, "flip_vertical": false,
 *   "aspect": "original" }
 * ```
 *
 * `aspect` is `"free"`, `"original"`, or `{ "ratio": { "width": 3, "height": 2 } }`
 * (two integers greater than 0). A missing field reads as its default.
 *
 * Each edit takes the session size (`W x H`, the display-oriented original)
 * and keeps the rect inside the rotated image (see `cropGeometry.ts`).
 */
import {
  FULL_RECT,
  fitRect,
  mirrorRect,
  pixelAspect,
  rectForAngle,
  rectForAspect,
  swapRect,
  type CropFrame,
  type NormalizedRect,
} from './cropGeometry';
import type { DevelopDocument } from './settings';

/** The aspect constraint of the crop tool. */
export type CropAspect = 'free' | 'original' | { readonly ratio: { readonly width: number; readonly height: number } };

/** The crop settings (`crop` field of the document). */
export interface CropSettings {
  readonly rect: NormalizedRect;
  /** Degrees in `CROP_ANGLE_RANGE`, positive is clockwise. */
  readonly angle: number;
  readonly flip_horizontal: boolean;
  readonly flip_vertical: boolean;
  readonly aspect: CropAspect;
}

/** The range of the angle slider. Keep it equal to `ranges::CROP_ANGLE`. `lib/rustConstants.test.ts` checks min and max. */
export const CROP_ANGLE_RANGE = { min: -45, max: 45, step: 0.1 } as const;

/** The default crop. Keep it equal to `Default` of `Crop` in `expose_core::develop`. */
export const CROP_DEFAULTS: CropSettings = {
  rect: FULL_RECT,
  angle: 0,
  flip_horizontal: false,
  flip_vertical: false,
  aspect: 'original',
};

/** The session size in pixels. */
export interface FrameSize {
  readonly width: number;
  readonly height: number;
}

function isRatio(value: unknown): value is { ratio: { width: number; height: number } } {
  if (typeof value !== 'object' || value === null || !('ratio' in value)) {
    return false;
  }
  const ratio: unknown = value.ratio;
  return (
    typeof ratio === 'object' &&
    ratio !== null &&
    'width' in ratio &&
    'height' in ratio &&
    typeof ratio.width === 'number' &&
    typeof ratio.height === 'number' &&
    ratio.width > 0 &&
    ratio.height > 0
  );
}

function readAspect(value: unknown): CropAspect {
  if (value === 'free' || value === 'original') {
    return value;
  }
  return isRatio(value) ? { ratio: { width: value.ratio.width, height: value.ratio.height } } : CROP_DEFAULTS.aspect;
}

/** The crop of a document. Missing or wrong-typed values read as their defaults. */
export function cropOf(doc: DevelopDocument): CropSettings {
  const raw: unknown = doc['crop'];
  if (typeof raw !== 'object' || raw === null) {
    return CROP_DEFAULTS;
  }
  const value = raw as Record<string, unknown>;
  const rectRaw = (typeof value['rect'] === 'object' && value['rect'] !== null ? value['rect'] : {}) as Record<
    string,
    unknown
  >;
  const num = (v: unknown, fallback: number): number => (typeof v === 'number' && Number.isFinite(v) ? v : fallback);
  return {
    rect: {
      left: num(rectRaw['left'], FULL_RECT.left),
      top: num(rectRaw['top'], FULL_RECT.top),
      width: num(rectRaw['width'], FULL_RECT.width),
      height: num(rectRaw['height'], FULL_RECT.height),
    },
    angle: num(value['angle'], CROP_DEFAULTS.angle),
    flip_horizontal: value['flip_horizontal'] === true,
    flip_vertical: value['flip_vertical'] === true,
    aspect: readAspect(value['aspect']),
  };
}

/** Returns a copy of `doc` with the crop changed. All crop fields are written. */
export function withCrop(doc: DevelopDocument, crop: CropSettings): DevelopDocument {
  return { ...doc, crop: { ...crop, rect: { ...crop.rect } } };
}

/**
 * The settings for the preview in crop mode. The geometry stage is off
 * then, so the crop has no effect on the frame. Without the field, crop
 * edits do not request new frames.
 */
export function withoutCrop(doc: DevelopDocument): DevelopDocument {
  if (!('crop' in doc)) {
    return doc;
  }
  const { crop: _crop, ...rest } = doc;
  return rest as DevelopDocument;
}

/** True when the crop changes nothing (the aspect is only a tool setting). */
export function isIdentityCrop(crop: CropSettings): boolean {
  return (
    crop.angle === 0 &&
    !crop.flip_horizontal &&
    !crop.flip_vertical &&
    crop.rect.left === 0 &&
    crop.rect.top === 0 &&
    crop.rect.width === 1 &&
    crop.rect.height === 1
  );
}

/** The pixel aspect (width / height) of an aspect constraint, or null for free. */
export function aspectRatio(aspect: CropAspect, size: FrameSize): number | null {
  if (aspect === 'free') {
    return null;
  }
  if (aspect === 'original') {
    return size.width / size.height;
  }
  return aspect.ratio.width / aspect.ratio.height;
}

/** One entry of the aspect list. */
export interface AspectPreset {
  /** `free`, `original`, or `W:H`. */
  readonly id: string;
  readonly label: string;
  readonly aspect: CropAspect;
}

const ratio = (width: number, height: number): CropAspect => ({ ratio: { width, height } });

/** The aspect presets, portrait and landscape forms next to each other. */
export const ASPECT_PRESETS: readonly AspectPreset[] = [
  { id: 'free', label: 'Free', aspect: 'free' },
  { id: 'original', label: 'Original', aspect: 'original' },
  ...(
    [
      [1, 1],
      [4, 5],
      [5, 4],
      [5, 7],
      [7, 5],
      [2, 3],
      [3, 2],
      [3, 4],
      [4, 3],
      [9, 16],
      [16, 9],
    ] as const
  ).map(([w, h]) => ({ id: `${w}:${h}`, label: `${w} : ${h}`, aspect: ratio(w, h) })),
];

/** The id of an aspect (`free`, `original`, or `W:H`). */
export function aspectId(aspect: CropAspect): string {
  return typeof aspect === 'string' ? aspect : `${aspect.ratio.width}:${aspect.ratio.height}`;
}

/** The aspect of an id from `aspectId`, or null for an id that is not valid. */
export function aspectFromId(id: string): CropAspect | null {
  if (id === 'free' || id === 'original') {
    return id;
  }
  const match = /^(\d+):(\d+)$/.exec(id);
  if (match === null) {
    return null;
  }
  const w = Number(match[1]);
  const h = Number(match[2]);
  return w > 0 && h > 0 ? ratio(w, h) : null;
}

function gcd(a: number, b: number): number {
  let x = Math.abs(a);
  let y = Math.abs(b);
  while (y !== 0) {
    [x, y] = [y, x % y];
  }
  return x;
}

/** A reduced integer ratio for pixel sizes (each at least 1). */
export function integerRatio(width: number, height: number): CropAspect {
  const w = Math.max(1, Math.round(width));
  const h = Math.max(1, Math.round(height));
  const d = gcd(w, h);
  return ratio(w / d, h / d);
}

/** The aspect with portrait and landscape swapped. Free stays free. */
export function swapAspect(aspect: CropAspect, size: FrameSize): CropAspect {
  if (aspect === 'free') {
    return aspect;
  }
  if (aspect === 'original') {
    return integerRatio(size.height, size.width);
  }
  return ratio(aspect.ratio.height, aspect.ratio.width);
}

function frameOf(crop: CropSettings, size: FrameSize): CropFrame {
  return { width: size.width, height: size.height, angle: crop.angle };
}

/** The pixel aspect that edits of the rect keep, or null for free. */
export function lockedRatio(crop: CropSettings, size: FrameSize): number | null {
  return aspectRatio(crop.aspect, size);
}

/**
 * Sets the angle (clamped to `CROP_ANGLE_RANGE`). The rect follows: a
 * largest rect stays the largest, another rect shrinks when it does not fit.
 */
export function setCropAngle(crop: CropSettings, size: FrameSize, angle: number): CropSettings {
  if (!Number.isFinite(angle)) {
    return crop;
  }
  const clamped = Math.min(CROP_ANGLE_RANGE.max, Math.max(CROP_ANGLE_RANGE.min, angle));
  const rect = rectForAngle(frameOf(crop, size), crop.rect, clamped, lockedRatio(crop, size));
  return { ...crop, angle: clamped, rect };
}

/** Sets the aspect. A ratio gives the largest centered rect of that ratio; free keeps the rect. */
export function setCropAspect(crop: CropSettings, size: FrameSize, aspect: CropAspect): CropSettings {
  const rect = rectForAspect(frameOf(crop, size), crop.rect, aspectRatio(aspect, size));
  return { ...crop, aspect, rect };
}

/**
 * Locks the aspect to the current rect shape (an integer ratio of its pixel
 * size), or unlocks it (free).
 */
export function setCropLock(crop: CropSettings, size: FrameSize, locked: boolean): CropSettings {
  if (!locked) {
    return { ...crop, aspect: 'free' };
  }
  if (crop.aspect !== 'free') {
    return crop;
  }
  const w = crop.rect.width * size.width;
  const h = crop.rect.height * size.height;
  return { ...crop, aspect: integerRatio(w, h) };
}

/** Swaps portrait and landscape: of the aspect and of the rect (about its center). */
export function swapCropOrientation(crop: CropSettings, size: FrameSize): CropSettings {
  const aspect = swapAspect(crop.aspect, size);
  const frame = frameOf(crop, size);
  const swapped = swapRect(frame, crop.rect);
  const target = aspectRatio(aspect, size);
  // `swapRect` keeps the pixel aspect exactly unless it had to move the rect;
  // a ratio rect that cannot keep its center becomes the largest of the ratio.
  const rect =
    target !== null && Math.abs(pixelAspect(frame, swapped) - target) > 1e-6 * target
      ? rectForAspect(frame, swapped, target)
      : swapped;
  return { ...crop, aspect, rect };
}

/**
 * Mirrors the image. The angle changes sign and the rect is mirrored, so
 * the result is the mirror image of the old result (the same content stays
 * in the crop).
 */
export function flipCrop(crop: CropSettings, axis: 'horizontal' | 'vertical'): CropSettings {
  const horizontal = axis === 'horizontal';
  return {
    ...crop,
    flip_horizontal: horizontal ? !crop.flip_horizontal : crop.flip_horizontal,
    flip_vertical: horizontal ? crop.flip_vertical : !crop.flip_vertical,
    // Keep 0 and not -0 in the document.
    angle: crop.angle === 0 ? 0 : -crop.angle,
    rect: mirrorRect(crop.rect, horizontal),
  };
}

/** Sets the rect. It is fitted into the rotated image. */
export function setCropRect(crop: CropSettings, size: FrameSize, rect: NormalizedRect): CropSettings {
  return { ...crop, rect: fitRect(frameOf(crop, size), rect) };
}

/** The default crop (full rect, angle 0, no flips, aspect original). */
export function resetCrop(): CropSettings {
  return CROP_DEFAULTS;
}
