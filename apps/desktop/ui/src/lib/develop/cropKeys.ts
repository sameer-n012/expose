/**
 * Keyboard control of the crop frame (pure helpers).
 *
 * The crop frame has focus:
 * - Arrow keys move the rect by 1 % of the frame (Shift: 10 %).
 * - Alt + arrow keys resize the rect from its bottom-right corner by the
 *   same step. Right and down make it larger, left and up make it smaller.
 *   A locked aspect stays.
 *
 * The result stays inside the rotated image (see `cropGeometry.ts`).
 */
import { moveRect, resizeRect, type CropFrame, type NormalizedRect } from './cropGeometry';

/** The step of an arrow key, as a part of the frame. */
export const KEY_STEP = 0.01;
/** The step with Shift held. */
export const KEY_STEP_LARGE = 0.1;

/** The text of the tooltip and the accessible name of the crop frame. */
export const CROP_KEYS_HELP =
  'Crop frame. Arrow keys move it, Shift for big steps. Alt and arrow keys resize it from the bottom right.';

/** The parts of a keyboard event that the crop frame reads. */
export interface CropKey {
  readonly key: string;
  readonly shiftKey: boolean;
  readonly altKey: boolean;
}

const DIRECTIONS: Readonly<Record<string, readonly [number, number]>> = {
  ArrowLeft: [-1, 0],
  ArrowRight: [1, 0],
  ArrowUp: [0, -1],
  ArrowDown: [0, 1],
};

/**
 * The rect after a key, or null when the key is not a crop key (the caller
 * then lets the event go on, so Enter, Escape, O, and X still work).
 * `ratio` is the locked pixel aspect, or null.
 */
export function applyCropKey(
  frame: CropFrame,
  rect: NormalizedRect,
  key: CropKey,
  ratio: number | null,
): NormalizedRect | null {
  const direction = DIRECTIONS[key.key];
  if (direction === undefined) {
    return null;
  }
  const step = key.shiftKey ? KEY_STEP_LARGE : KEY_STEP;
  const dx = direction[0] * step;
  const dy = direction[1] * step;
  return key.altKey ? resizeRect(frame, rect, 'se', dx, dy, ratio) : moveRect(frame, rect, dx, dy);
}
