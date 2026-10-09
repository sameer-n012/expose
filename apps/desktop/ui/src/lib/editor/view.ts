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
 */
import type { RenderArea } from './api';

export interface View {
  readonly zoom: number;
  readonly originX: number;
  readonly originY: number;
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

/** A view with the given zoom and the document centered in the viewport. */
export function centeredView(viewport: Size, doc: Size, zoom: number): View {
  const z = clampZoom(zoom);
  return {
    zoom: z,
    originX: doc.width / 2 - viewport.width / 2 / z,
    originY: doc.height / 2 - viewport.height / 2 / z,
  };
}

/** The view that shows the whole document. */
export function fitView(viewport: Size, doc: Size, margin: number = FIT_MARGIN): View {
  return centeredView(viewport, doc, fitZoom(viewport, doc, margin));
}

export function canvasToScreen(view: View, p: Point): Point {
  return { x: (p.x - view.originX) * view.zoom, y: (p.y - view.originY) * view.zoom };
}

export function screenToCanvas(view: View, p: Point): Point {
  return { x: p.x / view.zoom + view.originX, y: p.y / view.zoom + view.originY };
}

/** Changes the zoom so that the canvas point under `anchor` (screen) stays under it. */
export function zoomAbout(view: View, newZoom: number, anchor: Point): View {
  const z = clampZoom(newZoom);
  const under = screenToCanvas(view, anchor);
  return { zoom: z, originX: under.x - anchor.x / z, originY: under.y - anchor.y / z };
}

/** Moves the view by a screen distance (content follows the pointer). */
export function panBy(view: View, dx: number, dy: number): View {
  return { ...view, originX: view.originX - dx / view.zoom, originY: view.originY - dy / view.zoom };
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
 */
export function clampView(view: View, viewport: Size, doc: Size, slack: number = PAN_SLACK): View {
  const zoom = clampZoom(view.zoom);
  return {
    zoom,
    originX: clampAxis(view.originX, doc.width, viewport.width, zoom, slack),
    originY: clampAxis(view.originY, doc.height, viewport.height, zoom, slack),
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

/** The frame request for a view at the given device pixel ratio. */
export function renderArea(view: View, viewport: Size, devicePixelRatio: number): RenderArea {
  const dpr = Number.isFinite(devicePixelRatio) && devicePixelRatio > 0 ? devicePixelRatio : 1;
  return {
    originX: view.originX,
    originY: view.originY,
    scale: view.zoom * dpr,
    width: Math.max(1, Math.round(viewport.width * dpr)),
    height: Math.max(1, Math.round(viewport.height * dpr)),
  };
}

/** Where a frame that was rendered for `area` sits in the viewport now (CSS pixels). */
export function frameBox(area: RenderArea, view: View): { left: number; top: number; width: number; height: number } {
  // The frame has `scale` output pixels per canvas pixel; one canvas pixel is `view.zoom` CSS pixels now.
  const cssPerOutput = view.zoom / area.scale;
  return {
    left: (area.originX - view.originX) * view.zoom,
    top: (area.originY - view.originY) * view.zoom,
    width: area.width * cssPerOutput,
    height: area.height * cssPerOutput,
  };
}
