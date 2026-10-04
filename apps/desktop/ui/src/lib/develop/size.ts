/**
 * The target size of preview frames.
 *
 * The shell renders frames at the session size (default long edge 2560).
 * When the canvas on the screen is smaller, a smaller frame has the same
 * visible detail and costs less to render, transfer, and draw. The UI
 * sends a target long edge with each frame request; the shell clamps it
 * again (see `effective_long_edge` in `src-tauri/src/preview/engine.rs`).
 */

/** The smallest target long edge. Keep equal to `MIN_LONG_EDGE` in the shell. */
export const MIN_TARGET_LONG_EDGE = 64;

/** The largest target long edge. Keep equal to `MAX_LONG_EDGE` in the shell. */
export const MAX_TARGET_LONG_EDGE = 8192;

/**
 * The target long edge is a multiple of this step. Reason: a window resize
 * changes the box size by small amounts. Each new size makes the shell
 * remake the GPU view of the session, so small changes must not change
 * the target.
 */
export const TARGET_LONG_EDGE_STEP = 128;

/** A width and a height. */
export interface Size {
  readonly width: number;
  readonly height: number;
}

/**
 * Returns the long edge in device pixels of an image of size `image` when
 * it is fitted ("contain") into a box of `box` CSS pixels at
 * `devicePixelRatio`. The result is rounded up to a multiple of
 * `TARGET_LONG_EDGE_STEP`, clamped to
 * `MIN_TARGET_LONG_EDGE..=MAX_TARGET_LONG_EDGE`, and not larger than the
 * long edge of `image` (the shell does not enlarge frames).
 *
 * Returns null if a size is not a positive finite number (for example, a
 * hidden box with size 0). Then send no target.
 */
export function targetLongEdge(box: Size, devicePixelRatio: number, image: Size): number | null {
  const values = [box.width, box.height, devicePixelRatio, image.width, image.height];
  if (!values.every((value) => Number.isFinite(value) && value > 0)) {
    return null;
  }
  const scale = Math.min(
    (box.width * devicePixelRatio) / image.width,
    (box.height * devicePixelRatio) / image.height,
  );
  const imageLong = Math.max(image.width, image.height);
  const needed = imageLong * scale;
  const stepped = Math.ceil(needed / TARGET_LONG_EDGE_STEP) * TARGET_LONG_EDGE_STEP;
  const clamped = Math.min(Math.max(stepped, MIN_TARGET_LONG_EDGE), MAX_TARGET_LONG_EDGE);
  return Math.min(clamped, Math.round(imageLong));
}
