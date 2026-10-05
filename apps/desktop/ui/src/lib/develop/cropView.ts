/**
 * Screen-side helpers of the crop tool: the CSS transform of the preview,
 * the mapping between screen pixels and the normalized frame, and the
 * guide lines of the crop overlay.
 *
 * In crop mode the preview frame is the full image without geometry
 * (`applyGeometry: false`). CSS then shows the flip and the rotation, so
 * the user sees the image move under a fixed, axis-aligned crop frame.
 */
import type { CropSettings } from './crop';

/**
 * The 2 x 2 matrix `[a, b, c, d]` (CSS `matrix(a, b, c, d, 0, 0)` order:
 * `x' = a*x + c*y`, `y' = b*x + d*y`) that maps a source offset from the
 * image center to the frame offset from the center. Screen coordinates
 * have y down.
 *
 * The backend flips first, then rotates clockwise by `angle`. Thus the
 * matrix is `R(angle) * S` with `S = diag(sx, sy)` (`-1` for a flip).
 */
export function cropMatrix(crop: Pick<CropSettings, 'angle' | 'flip_horizontal' | 'flip_vertical'>): [number, number, number, number] {
  const rad = (crop.angle * Math.PI) / 180;
  const cos = Math.cos(rad);
  const sin = Math.sin(rad);
  const sx = crop.flip_horizontal ? -1 : 1;
  const sy = crop.flip_vertical ? -1 : 1;
  // R = [[cos, -sin], [sin, cos]] (clockwise on a y-down screen).
  return [cos * sx, sin * sx, -sin * sy, cos * sy];
}

/**
 * The CSS `transform` of the preview canvas in crop mode, or `none`. The
 * transform origin must be the center of the canvas (the CSS default).
 */
export function cropTransform(crop: Pick<CropSettings, 'angle' | 'flip_horizontal' | 'flip_vertical'>): string {
  if (crop.angle === 0 && !crop.flip_horizontal && !crop.flip_vertical) {
    return 'none';
  }
  const fmt = (v: number): string => {
    // Round away float noise (for example 6e-17 for cos 90) for short CSS.
    const r = Math.round(v * 1e9) / 1e9;
    return Object.is(r, -0) ? '0' : String(r);
  };
  const [a, b, c, d] = cropMatrix(crop);
  return `matrix(${fmt(a)}, ${fmt(b)}, ${fmt(c)}, ${fmt(d)}, 0, 0)`;
}

/** A rectangle in CSS pixels. */
export interface ScreenRect {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

/**
 * The rect of an image of `imageWidth x imageHeight` in a box, scaled to
 * fit and centered (CSS `object-fit: contain`).
 */
export function containRect(boxWidth: number, boxHeight: number, imageWidth: number, imageHeight: number): ScreenRect {
  if (!(boxWidth > 0 && boxHeight > 0 && imageWidth > 0 && imageHeight > 0)) {
    return { x: 0, y: 0, width: 0, height: 0 };
  }
  const scale = Math.min(boxWidth / imageWidth, boxHeight / imageHeight);
  const width = imageWidth * scale;
  const height = imageHeight * scale;
  return { x: (boxWidth - width) / 2, y: (boxHeight - height) / 2, width, height };
}

/** A pointer move in CSS pixels as a normalized move in the frame shown in `content`. */
export function toNormalizedDelta(dx: number, dy: number, content: ScreenRect): { dx: number; dy: number } {
  if (!(content.width > 0 && content.height > 0)) {
    return { dx: 0, dy: 0 };
  }
  return { dx: dx / content.width, dy: dy / content.height };
}

/** The guide overlays inside the crop rect. */
export type CropOverlayKind = 'thirds' | 'grid' | 'golden';

export const CROP_OVERLAYS: readonly CropOverlayKind[] = ['thirds', 'grid', 'golden'];

export const CROP_OVERLAY_LABELS: Readonly<Record<CropOverlayKind, string>> = {
  thirds: 'Rule of thirds',
  grid: 'Grid',
  golden: 'Golden ratio',
};

/** The overlay after `kind` (the `O` key). */
export function nextOverlay(kind: CropOverlayKind): CropOverlayKind {
  const index = CROP_OVERLAYS.indexOf(kind);
  return CROP_OVERLAYS[(index + 1) % CROP_OVERLAYS.length] ?? 'thirds';
}

/** One guide line, in `0..1` units of the crop rect. */
export interface GuideLine {
  readonly x1: number;
  readonly y1: number;
  readonly x2: number;
  readonly y2: number;
}

/** The number of cells per side of the `grid` overlay. */
export const GRID_DIVISIONS = 6;
/** The number of cells per side of the fine grid while the angle changes. */
export const FINE_GRID_DIVISIONS = 18;

/** The inverse of the golden ratio (about 0.618). */
const GOLDEN = (Math.sqrt(5) - 1) / 2;

function crossLines(positions: readonly number[]): GuideLine[] {
  return positions.flatMap((p) => [
    { x1: p, y1: 0, x2: p, y2: 1 },
    { x1: 0, y1: p, x2: 1, y2: p },
  ]);
}

function divisions(count: number): number[] {
  return Array.from({ length: count - 1 }, (_, i) => (i + 1) / count);
}

/**
 * The guide lines of an overlay. `fine` is the dense grid that shows while
 * the user changes the angle.
 */
export function overlayLines(kind: CropOverlayKind | 'fine'): GuideLine[] {
  switch (kind) {
    case 'thirds':
      return crossLines(divisions(3));
    case 'grid':
      return crossLines(divisions(GRID_DIVISIONS));
    case 'golden':
      return crossLines([1 - GOLDEN, GOLDEN]);
    case 'fine':
      return crossLines(divisions(FINE_GRID_DIVISIONS));
  }
}
