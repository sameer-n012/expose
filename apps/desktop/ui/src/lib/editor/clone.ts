/**
 * Pure helpers of the clone stamp and the healing brush.
 *
 * The user sets a source point with Alt+click. A stroke copies the pixels at
 * `destination + offset`. The offset is the source point minus the first
 * point of the stroke, rounded to whole canvas pixels.
 */
import type { StrokeTargetDto } from './api';
import type { BrushSettings } from './brush';
import type { Point } from './view';

/** The tools that copy pixels. */
export type CloneTool = 'cloneStamp' | 'healingBrush';

/** True for the clone stamp and the healing brush. */
export function isCloneTool(tool: string): tool is CloneTool {
  return tool === 'cloneStamp' || tool === 'healingBrush';
}

/** The source point and the stored offset of one document tab. */
export interface CloneState {
  /** The point of the last Alt+click, in canvas pixels, or null. */
  readonly source: Point | null;
  /** The offset of the last stroke, or null (not known yet). */
  readonly offset: readonly [number, number] | null;
}

export const EMPTY_CLONE_STATE: CloneState = { source: null, offset: null };

/** A new source point. It clears the stored offset. */
export function withSource(source: Point): CloneState {
  return { source: { x: source.x, y: source.y }, offset: null };
}

/**
 * The offset for a stroke that starts at `start`, and the state after it.
 *
 * - Aligned and an offset is stored: the stored offset.
 * - Else: `round(source - start)`, which is stored.
 * - No source point: null (no stroke can start).
 */
export function strokeOffset(state: CloneState, start: Point, aligned: boolean): { offset: [number, number]; state: CloneState } | null {
  if (state.source === null) {
    return null;
  }
  if (aligned && state.offset !== null) {
    return { offset: [state.offset[0], state.offset[1]], state };
  }
  const offset: [number, number] = [Math.round(state.source.x - start.x), Math.round(state.source.y - start.y)];
  return { offset, state: { source: state.source, offset } };
}

/** The state when a stroke ends: a non-aligned stroke forgets its offset, so the next one starts at the source point. */
export function afterStroke(state: CloneState, aligned: boolean): CloneState {
  return aligned ? state : { source: state.source, offset: null };
}

/** The stroke target that the shell receives. */
export function cloneTarget(tool: CloneTool, offset: readonly [number, number], sampleAllLayers: boolean): StrokeTargetDto {
  return { type: tool === 'cloneStamp' ? 'clone' : 'heal', offset: [offset[0], offset[1]], sampleAllLayers };
}

/**
 * Where the source crosshair is, in canvas pixels: `pointer + offset` while
 * the pointer is on the canvas and an offset is known, else the source point.
 */
export function sourceMarker(state: CloneState, pointer: Point | null): Point | null {
  if (state.source === null) {
    return null;
  }
  if (pointer !== null && state.offset !== null) {
    return { x: pointer.x + state.offset[0], y: pointer.y + state.offset[1] };
  }
  return state.source;
}

/** The start settings of the clone stamp and the healing brush. */
export function defaultCloneBrush(): BrushSettings {
  return { size: 40, hardness: 0.5, opacity: 1, flow: 1, blendMode: 'normal', spacing: 0.15, pressureSize: false, pressureOpacity: false };
}
