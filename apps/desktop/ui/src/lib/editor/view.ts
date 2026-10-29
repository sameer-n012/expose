/**
 * Pure view math of the Editor canvas.
 *
 * A `View` maps canvas coordinates (document pixels) to screen coordinates
 * (CSS pixels of the viewport):
 *
 *   screen = (canvas - origin) * zoom
 *
 * `origin` is the canvas coordinate at the top-left corner of the viewport.
 * `zoom` is CSS pixels per canvas pixel.
 *
 * Rotation (ADR 0026): `rotation` turns the content about the center of the
 * viewport (degrees, clockwise on the screen). All the mapping above is in
 * the "content space": the space of the viewport before the rotation. The
 * view shows the content space in a container with a CSS rotation about the
 * viewport center. A pointer position must go through `unrotatePoint`
 * before `screenToCanvas`. At rotation 0 the content space is the screen.
 */
import type { RenderArea } from './api';

/** The part of a view that maps canvas pixels to content-space pixels. */
export interface ViewMapping {
  readonly zoom: number;
  readonly originX: number;
  readonly originY: number;
}

export interface View extends ViewMapping {
  /** The rotation of the view in degrees, in (-180, 180]. Clockwise on the screen. */
  readonly rotation: number;
}

export interface Size {
  readonly width: number;
  readonly height: number;
}

export interface Point {
  readonly x: number;
  readonly y: number;
}

export const MIN_ZOOM = 0.02;
export const MAX_ZOOM = 32;
/** The factor of one zoom step (`Mod+=` and `Mod+-`). */
export const ZOOM_STEP = 1.25;
/** Space around the document in the fit view, in CSS pixels. */
export const FIT_MARGIN = 24;
/** The part of the viewport that stays free around a large document at the pan limit, in CSS pixels. */
export const PAN_SLACK = 48;

/** Limits a zoom value to the allowed range. */
export function clampZoom(zoom: number): number {
  if (!Number.isFinite(zoom)) {
    return 1;
  }
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom));
}

/** The zoom at which the whole document fits in the viewport (with a margin). Not above 100 %. */
export function fitZoom(viewport: Size, doc: Size, margin: number = FIT_MARGIN): number {
  if (doc.width <= 0 || doc.height <= 0) {
    return 1;
  }
  const w = Math.max(1, viewport.width - 2 * margin);
  const h = Math.max(1, viewport.height - 2 * margin);
  return clampZoom(Math.min(1, w / doc.width, h / doc.height));
}

/**
 * A view with the given zoom and the document centered in the viewport.
 * The rotation does not change the center, so it does not change the origin.
 */
export function centeredView(viewport: Size, doc: Size, zoom: number, rotation: number = 0): View {
  const z = clampZoom(zoom);
  return {
    zoom: z,
    originX: doc.width / 2 - viewport.width / 2 / z,
    originY: doc.height / 2 - viewport.height / 2 / z,
    rotation: normalizeRotation(rotation),
  };
}

/**
 * The view that shows the whole document. The zoom uses the size of the
 * document without the rotation (a rotated document can go past the
 * viewport corners); the view keeps `rotation`.
 */
export function fitView(viewport: Size, doc: Size, margin: number = FIT_MARGIN, rotation: number = 0): View {
  return centeredView(viewport, doc, fitZoom(viewport, doc, margin), rotation);
}

/** Canvas pixels to content-space pixels. */
export function canvasToScreen(view: ViewMapping, p: Point): Point {
  return { x: (p.x - view.originX) * view.zoom, y: (p.y - view.originY) * view.zoom };
}

/** Content-space pixels to canvas pixels. */
export function screenToCanvas(view: ViewMapping, p: Point): Point {
  return { x: p.x / view.zoom + view.originX, y: p.y / view.zoom + view.originY };
}

/** Changes the zoom so that the canvas point under `anchor` (content space) stays under it. */
export function zoomAbout<V extends ViewMapping>(view: V, newZoom: number, anchor: Point): V {
  const z = clampZoom(newZoom);
  const under = screenToCanvas(view, anchor);
  return { ...view, zoom: z, originX: under.x - anchor.x / z, originY: under.y - anchor.y / z };
}

/** Moves the view by a content-space distance (content follows the pointer). */
export function panBy<V extends ViewMapping>(view: V, dx: number, dy: number): V {
  return { ...view, originX: view.originX - dx / view.zoom, originY: view.originY - dy / view.zoom };
}

// ---- Rotation ----

/** The angle step of the rotate tool with Shift held, in degrees. */
export const ROTATION_SNAP = 15;

/**
 * Brings an angle in degrees to the range (-180, 180]. A value that is not
 * finite gives 0 (like `clampZoom`, a bad value must not reach the view).
 */
export function normalizeRotation(degrees: number): number {
  if (!Number.isFinite(degrees)) {
    return 0;
  }
  const r = ((degrees % 360) + 360) % 360;
  // `+ 0` changes -0 to 0.
  return (r > 180 ? r - 360 : r) + 0;
}

/**
 * Cosine and sine of an angle in degrees. Multiples of 90° give exact
 * values, so a quarter turn does not add a fraction of a pixel.
 */
function cosSin(degrees: number): { cos: number; sin: number } {
  const r = normalizeRotation(degrees);
  if (r === 0) return { cos: 1, sin: 0 };
  if (r === 90) return { cos: 0, sin: 1 };
  if (r === 180) return { cos: -1, sin: 0 };
  if (r === -90) return { cos: 0, sin: -1 };
  const rad = (r * Math.PI) / 180;
  return { cos: Math.cos(rad), sin: Math.sin(rad) };
}

/** Turns a vector by `degrees` (clockwise on the screen, where y points down). */
export function rotateVector(v: Point, degrees: number): Point {
  if (degrees === 0) {
    return v;
  }
  const { cos, sin } = cosSin(degrees);
  return { x: v.x * cos - v.y * sin, y: v.x * sin + v.y * cos };
}

/**
 * A screen point (viewport pixels) to content space: turns it by
 * `-rotation` about the viewport center. At rotation 0 it gives `p`.
 */
export function unrotatePoint(p: Point, viewport: Size, rotation: number): Point {
  if (rotation === 0) {
    return p;
  }
  const cx = viewport.width / 2;
  const cy = viewport.height / 2;
  const v = rotateVector({ x: p.x - cx, y: p.y - cy }, -rotation);
  return { x: cx + v.x, y: cy + v.y };
}

/** The inverse of `unrotatePoint`: a content-space point to the screen. */
export function rotatePoint(p: Point, viewport: Size, rotation: number): Point {
  if (rotation === 0) {
    return p;
  }
  const cx = viewport.width / 2;
  const cy = viewport.height / 2;
  const v = rotateVector({ x: p.x - cx, y: p.y - cy }, rotation);
  return { x: cx + v.x, y: cy + v.y };
}

/** A box in content-space pixels. */
export interface Box {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

/**
 * The content-space box that covers the whole viewport at a rotation: the
 * axis-aligned box of the viewport turned by `-rotation` about its center.
 * It has the same center as the viewport. At rotation 0 it is the viewport.
 */
export function rotatedCover(viewport: Size, rotation: number): Box {
  const { cos, sin } = cosSin(rotation);
  const c = Math.abs(cos);
  const s = Math.abs(sin);
  const width = viewport.width * c + viewport.height * s;
  const height = viewport.width * s + viewport.height * c;
  return { x: (viewport.width - width) / 2, y: (viewport.height - height) / 2, width, height };
}

function clampAxis(origin: number, docSize: number, viewSize: number, zoom: number, slack: number): number {
  const docScreen = docSize * zoom;
  if (docScreen + 2 * slack <= viewSize) {
    // The document is small: keep it centered.
    return docSize / 2 - viewSize / 2 / zoom;
  }
  const min = -slack / zoom;
  const max = docSize - (viewSize - slack) / zoom;
  return Math.min(max, Math.max(min, origin));
}

/**
 * Keeps the document in view: a small document stays centered; a large
 * document cannot move farther than `slack` pixels past its edge.
 *
 * With a rotation, the limits use the content-space box that covers the
 * viewport (`rotatedCover`), so the user can still move each corner of the
 * document into view. At rotation 0 that box is the viewport.
 */
export function clampView(view: View, viewport: Size, doc: Size, slack: number = PAN_SLACK): View {
  const zoom = clampZoom(view.zoom);
  const rotation = normalizeRotation(view.rotation);
  const cover = rotatedCover(viewport, rotation);
  // The canvas point at the top-left corner of the cover box.
  const dx = cover.x / zoom;
  const dy = cover.y / zoom;
  return {
    zoom,
    originX: clampAxis(view.originX + dx, doc.width, cover.width, zoom, slack) - dx,
    originY: clampAxis(view.originY + dy, doc.height, cover.height, zoom, slack) - dy,
    rotation,
  };
}

/** One zoom step in (`direction` 1) or out (`direction` -1). */
export function steppedZoom(zoom: number, direction: 1 | -1): number {
  return clampZoom(direction === 1 ? zoom * ZOOM_STEP : zoom / ZOOM_STEP);
}

/**
 * The zoom factor of a wheel event with a zoom intent (pinch, or the
 * primary modifier held). `deltaY` < 0 zooms in.
 */
export function wheelZoomFactor(deltaY: number, deltaMode: number = 0): number {
  // Line mode (1) gives small numbers; page mode (2) gives even smaller ones.
  const pixels = deltaMode === 1 ? deltaY * 16 : deltaMode === 2 ? deltaY * 100 : deltaY;
  // Limit one event so that a mouse wheel notch does not jump.
  const limited = Math.max(-60, Math.min(60, pixels));
  return Math.exp(-limited * 0.01);
}

/** True when the dimensions are usable for a frame request. */
export function hasArea(viewport: Size): boolean {
  return viewport.width >= 1 && viewport.height >= 1;
}

/**
 * The frame request for a view at the given device pixel ratio. With a
 * rotation, the frame covers the content-space box of the whole viewport
 * (`rotatedCover`); at rotation 0 it is the viewport exactly.
 */
export function renderArea(view: View, viewport: Size, devicePixelRatio: number): RenderArea {
  const dpr = Number.isFinite(devicePixelRatio) && devicePixelRatio > 0 ? devicePixelRatio : 1;
  const cover = rotatedCover(viewport, view.rotation);
  // At rotation 0 the origin stays the same value (no `+ 0` that changes -0).
  const rotated = view.rotation !== 0;
  return {
    originX: rotated ? view.originX + cover.x / view.zoom : view.originX,
    originY: rotated ? view.originY + cover.y / view.zoom : view.originY,
    scale: view.zoom * dpr,
    width: Math.max(1, Math.round(cover.width * dpr)),
    height: Math.max(1, Math.round(cover.height * dpr)),
  };
}

/** Where a frame that was rendered for `area` sits in the content space now (CSS pixels). */
export function frameBox(area: RenderArea, view: ViewMapping): { left: number; top: number; width: number; height: number } {
  // The frame has `scale` output pixels per canvas pixel; one canvas pixel is `view.zoom` CSS pixels now.
  const cssPerOutput = view.zoom / area.scale;
  return {
    left: (area.originX - view.originX) * view.zoom,
    top: (area.originY - view.originY) * view.zoom,
    width: area.width * cssPerOutput,
    height: area.height * cssPerOutput,
  };
}
