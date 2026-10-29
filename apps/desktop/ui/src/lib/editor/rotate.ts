/**
 * Pure logic of the "Rotate view" tool (ADR 0026).
 *
 * A drag turns the view about the viewport center by the change of the
 * pointer angle around that center. Shift snaps the result to steps of
 * `ROTATION_SNAP` degrees. Escape (or a cancelled pointer) gives back the
 * angle at the start of the drag. The points are screen points (viewport
 * pixels, before the rotation).
 */
import { ROTATION_SNAP, normalizeRotation, type Point, type Size } from './view';

/** A rotate drag that runs. */
export interface RotateDrag {
  /** The view rotation at the start of the drag, in degrees. */
  readonly startRotation: number;
  /** The pointer angle around the viewport center at the start, in degrees. */
  readonly startAngle: number;
}

/**
 * The angle of a screen point around the viewport center, in degrees.
 * The angle grows clockwise on the screen (y points down), like a CSS rotation.
 * The center itself gives 0.
 */
export function pointerAngle(p: Point, viewport: Size): number {
  return (Math.atan2(p.y - viewport.height / 2, p.x - viewport.width / 2) * 180) / Math.PI;
}

/** Starts a drag at a screen point. */
export function beginRotateDrag(rotation: number, p: Point, viewport: Size): RotateDrag {
  return { startRotation: rotation, startAngle: pointerAngle(p, viewport) };
}

/** Rounds an angle to the nearest snap step, in (-180, 180]. */
export function snapRotation(degrees: number, step: number = ROTATION_SNAP): number {
  return normalizeRotation(Math.round(degrees / step) * step);
}

/** The view rotation for the pointer at `p`. With `snap`, the result goes to the nearest snap step. */
export function dragRotation(drag: RotateDrag, p: Point, viewport: Size, snap: boolean): number {
  const next = normalizeRotation(drag.startRotation + pointerAngle(p, viewport) - drag.startAngle);
  return snap ? snapRotation(next) : next;
}

/** The view rotation after Escape or a cancelled pointer: the angle at the start. */
export function cancelRotateDrag(drag: RotateDrag): number {
  return drag.startRotation;
}

/** The text of the rotation readout, for example "Rotation 30°". Rounds to 0.1°. */
export function rotationLabel(rotation: number): string {
  const rounded = Math.round(rotation * 10) / 10;
  return `Rotation ${rounded + 0}°`;
}
