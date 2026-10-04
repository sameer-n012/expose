/**
 * Draws preview frames into a 2D canvas.
 *
 * - `put` (default): `putImageData` with an `ImageData` over the received
 *   buffer. One copy into the canvas backing store, no decode.
 * - `bitmap`: `createImageBitmap(ImageData)` and then `drawImage`. The
 *   browser can upload the bitmap to the GPU off the main thread.
 *
 * The transport benchmark measures both. The application draws through
 * `FramePresenter` (`presenter.ts`), which uses the helpers of this
 * module.
 */
import type { Frame } from './frame';
import type { DisplaySpace } from './transport';

export type DrawMode = 'put' | 'bitmap';

/** The draw mode that the application uses. */
export const DEFAULT_DRAW_MODE: DrawMode = 'put';

/**
 * Makes the canvas the size of the frame (if it is not already). A size
 * change clears the canvas, so do it only when the size changes.
 */
export function fitCanvas(canvas: HTMLCanvasElement, width: number, height: number): void {
  if (canvas.width !== width) {
    canvas.width = width;
  }
  if (canvas.height !== height) {
    canvas.height = height;
  }
}

/** Gets the 2D context for frames in `space`. Returns null if the web view gives none. */
export function frameContext(canvas: HTMLCanvasElement, space: DisplaySpace = 'srgb'): CanvasRenderingContext2D | null {
  const colorSpace: PredefinedColorSpace = space === 'displayP3' ? 'display-p3' : 'srgb';
  return canvas.getContext('2d', { alpha: false, colorSpace });
}

/** Makes an `ImageData` over the frame pixels (no copy). */
export function frameImageData(frame: Frame, space: DisplaySpace = 'srgb'): ImageData {
  const colorSpace: PredefinedColorSpace = space === 'displayP3' ? 'display-p3' : 'srgb';
  return new ImageData(frame.pixels, frame.width, frame.height, { colorSpace });
}

/** Draws a frame at the origin of the canvas, in the given mode. */
export async function drawFrame(
  ctx: CanvasRenderingContext2D,
  frame: Frame,
  mode: DrawMode = DEFAULT_DRAW_MODE,
  space: DisplaySpace = 'srgb',
): Promise<void> {
  fitCanvas(ctx.canvas, frame.width, frame.height);
  const image = frameImageData(frame, space);
  if (mode === 'put') {
    ctx.putImageData(image, 0, 0);
    return;
  }
  const bitmap = await createImageBitmap(image);
  try {
    ctx.drawImage(bitmap, 0, 0);
  } finally {
    bitmap.close();
  }
}
