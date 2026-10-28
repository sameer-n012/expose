/**
 * Pure geometry of the selection tools.
 *
 * Gestures run in screen space (CSS pixels of the viewport). The shapes
 * that go to the shell are in canvas pixels. `view.ts` converts between them.
 */
import type { SelectionModeId, SelectionShapeDto } from './api';
import { screenToCanvas, type Point, type View } from './view';

/** The tools of the Editor canvas. */
export type EditorTool = 'hand' | 'rectSelect' | 'ellipseSelect' | 'polygonLasso' | 'brush' | 'eraser';

/** The range of the feather radius, in canvas pixels. */
export const MAX_FEATHER = 250;
/** A drag shorter than this (screen pixels) is a click. */
export const CLICK_DISTANCE = 2;
/** A click this close (screen pixels) to the first polygon point closes the polygon. */
export const CLOSE_DISTANCE = 6;

/** The modifier keys of a pointer event. */
export interface Modifiers {
  readonly shiftKey: boolean;
  readonly altKey: boolean;
}

/** None: replace. Shift: add. Alt: subtract. Shift+Alt: intersect. */
export function modeFromModifiers(keys: Modifiers): SelectionModeId {
  if (keys.shiftKey && keys.altKey) {
    return 'intersect';
  }
  if (keys.shiftKey) {
    return 'add';
  }
  return keys.altKey ? 'subtract' : 'replace';
}

/** True for the tools that select an area. */
export function isSelectionTool(tool: EditorTool): boolean {
  return tool === 'rectSelect' || tool === 'ellipseSelect' || tool === 'polygonLasso';
}

/** The next tool when `M` is pressed: rectangle, then ellipse, then rectangle again. */
export function cycleMarquee(tool: EditorTool): EditorTool {
  return tool === 'rectSelect' ? 'ellipseSelect' : 'rectSelect';
}

/** Limits a feather radius to 0..=MAX_FEATHER. A value that is not a number gives 0. */
export function clampFeather(value: number): number {
  return Number.isFinite(value) ? Math.min(MAX_FEATHER, Math.max(0, value)) : 0;
}

function distance(a: Point, b: Point): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

/** True if a drag from `a` to `b` (screen) is too short to be a shape. */
export function isClick(a: Point, b: Point): boolean {
  return distance(a, b) < CLICK_DISTANCE;
}

/** A box with positive size. A drag in any direction gives the same box. */
export interface Box {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

/** The box between two corners, in any order. */
export function boxBetween(a: Point, b: Point): Box {
  return { x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), width: Math.abs(a.x - b.x), height: Math.abs(a.y - b.y) };
}

/**
 * The shape of a rectangle or ellipse drag. `a` and `b` are screen points.
 * The corners are rounded to whole canvas pixels. Returns null if the
 * rounded box has no area.
 */
export function dragShape(tool: 'rectSelect' | 'ellipseSelect', a: Point, b: Point, view: View): SelectionShapeDto | null {
  const start = screenToCanvas(view, a);
  const end = screenToCanvas(view, b);
  const x0 = Math.round(Math.min(start.x, end.x));
  const y0 = Math.round(Math.min(start.y, end.y));
  const x1 = Math.round(Math.max(start.x, end.x));
  const y1 = Math.round(Math.max(start.y, end.y));
  if (x1 <= x0 || y1 <= y0) {
    return null;
  }
  return { type: tool === 'rectSelect' ? 'rect' : 'ellipse', x: x0, y: y0, width: x1 - x0, height: y1 - y0 };
}

/** The shape of a closed polygon, from points in canvas pixels. Null for fewer than 3 points. */
export function polygonShape(points: readonly Point[]): SelectionShapeDto | null {
  if (points.length < 3) {
    return null;
  }
  return { type: 'polygon', points: points.map((p): [number, number] => [Math.round(p.x), Math.round(p.y)]) };
}

/**
 * True if a click at `pointer` closes the polygon: there are at least 3
 * points and the pointer is within `CLOSE_DISTANCE` of the first point.
 * All points are in screen space.
 */
export function closesPolygon(screenPoints: readonly Point[], pointer: Point): boolean {
  const first = screenPoints[0];
  return first !== undefined && screenPoints.length >= 3 && distance(first, pointer) <= CLOSE_DISTANCE;
}

/** True if a new point at `pointer` is on the last point (a second click of a double-click). */
export function repeatsLastPoint(screenPoints: readonly Point[], pointer: Point): boolean {
  const last = screenPoints[screenPoints.length - 1];
  return last !== undefined && distance(last, pointer) < CLICK_DISTANCE;
}

/** The SVG path of a box (a rectangle). */
export function boxPath(box: Box): string {
  return `M${box.x} ${box.y} h${box.width} v${box.height} h${-box.width} Z`;
}

/** The SVG path of the ellipse in a box. */
export function ellipsePath(box: Box): string {
  const rx = box.width / 2;
  const ry = box.height / 2;
  const cy = box.y + ry;
  return `M${box.x} ${cy} A${rx} ${ry} 0 1 0 ${box.x + box.width} ${cy} A${rx} ${ry} 0 1 0 ${box.x} ${cy} Z`;
}

/** The SVG path of an open line through the points. Empty for no points. */
export function polylinePath(points: readonly Point[]): string {
  return points.map((p, i) => `${i === 0 ? 'M' : 'L'}${p.x} ${p.y}`).join(' ');
}

/** The sample step for an outline request: whole canvas pixels per screen pixel, at least 1. */
export function outlineStep(scale: number): number {
  if (!Number.isFinite(scale) || scale <= 0) {
    return 1;
  }
  return Math.max(1, Math.floor(1 / scale));
}

/** The SVG path of one closed contour (flat x, y pairs). Empty if there are fewer than 3 points. */
export function contourPath(contour: readonly number[]): string {
  const count = Math.floor(contour.length / 2);
  if (count < 3) {
    return '';
  }
  const parts: string[] = [];
  for (let i = 0; i < count; i += 1) {
    parts.push(`${i === 0 ? 'M' : 'L'}${contour[2 * i]} ${contour[2 * i + 1]}`);
  }
  parts.push('Z');
  return parts.join(' ');
}

/** One SVG path for all contours. */
export function contoursPath(contours: readonly (readonly number[])[]): string {
  return contours
    .map(contourPath)
    .filter((d) => d !== '')
    .join(' ');
}
