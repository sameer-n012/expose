/**
 * The frame presenter: puts received frames on a canvas, in order, at most
 * once per display frame.
 *
 * # Steps
 *
 * 1. `present()` accepts a frame only if its `seq` is larger than the
 *    `seq` of all frames accepted before (in the same generation). Thus a
 *    stale frame never draws over a newer frame.
 * 2. The presenter prepares the accepted frames one at a time, in order.
 *    In `bitmap` mode it calls `createImageBitmap`, so the browser can
 *    convert and upload the pixels off the main thread. In `put` mode, or
 *    when `createImageBitmap` does not exist (for example in tests), the
 *    source is an `ImageData` over the received buffer (no copy).
 * 3. A prepared frame waits for the next animation frame. Then the
 *    presenter draws only the newest prepared frame. Older prepared frames
 *    are dropped (their bitmaps are closed), because the display cannot
 *    show more frames than it refreshes.
 *
 * A frame that arrives while another frame waits for preparation replaces
 * it. Thus the work per received frame stays bounded when frames arrive
 * faster than the presenter can handle them.
 */
import { fitCanvas, frameImageData, type DrawMode } from './draw';
import type { FrameEvent } from './previewClient';
import type { DisplaySpace } from './transport';

/** The draw mode of the presenter when nothing else is set. */
export const DEFAULT_PRESENT_MODE: DrawMode = 'bitmap';

/** Counters of the presenter. */
export interface PresenterStats {
  /** Calls of `present()`. */
  received: number;
  /** Frames drawn on the canvas. */
  presented: number;
  /** Frames not drawn: stale, replaced by a newer frame, or from an old generation. */
  skipped: number;
  /** Frames that failed (preparation or draw). */
  failed: number;
}

/** A drawn frame, with the main-thread time of its draw step. */
export interface PresentedEvent extends FrameEvent {
  /** Time to put the prepared frame on the canvas, in milliseconds. */
  readonly drawMs: number;
  /** Time from `present()` to the drawn frame, in milliseconds. */
  readonly presentMs: number;
}

export interface FramePresenterOptions {
  /** Returns the 2D context to draw into, or null if there is none (an error). */
  readonly context: () => CanvasRenderingContext2D | null;
  readonly mode?: DrawMode;
  readonly space?: DisplaySpace;
  /**
   * Runs a function at the next display frame. Default:
   * `requestAnimationFrame`, or `setTimeout(0)` where it does not exist.
   */
  readonly schedule?: (callback: () => void) => void;
  /** The `createImageBitmap` function. Default: the global one, if it exists. For tests. */
  readonly createBitmap?: ((image: ImageData) => Promise<ImageBitmap>) | null;
  readonly onPresented?: (event: PresentedEvent) => void;
  readonly onError?: (error: unknown) => void;
  /** The clock in milliseconds. Default `performance.now`. */
  readonly now?: () => number;
}

/** A prepared frame: a bitmap, or an `ImageData` for `putImageData`. */
interface Prepared {
  readonly event: FrameEvent;
  readonly source: ImageBitmap | ImageData;
  readonly generation: number;
  readonly receivedAt: number;
}

function defaultSchedule(callback: () => void): void {
  if (typeof requestAnimationFrame === 'function') {
    requestAnimationFrame(() => callback());
  } else {
    setTimeout(callback, 0);
  }
}

function defaultCreateBitmap(): ((image: ImageData) => Promise<ImageBitmap>) | null {
  return typeof createImageBitmap === 'function' ? (image) => createImageBitmap(image) : null;
}

function isBitmap(source: ImageBitmap | ImageData): source is ImageBitmap {
  return 'close' in source && typeof source.close === 'function';
}

export class FramePresenter {
  readonly stats: PresenterStats = { received: 0, presented: 0, skipped: 0, failed: 0 };

  readonly #context: () => CanvasRenderingContext2D | null;
  readonly #mode: DrawMode;
  readonly #space: DisplaySpace;
  readonly #schedule: (callback: () => void) => void;
  readonly #createBitmap: ((image: ImageData) => Promise<ImageBitmap>) | null;
  readonly #onPresented: (event: PresentedEvent) => void;
  readonly #onError: (error: unknown) => void;
  readonly #now: () => number;

  #generation = 0;
  #lastAccepted = -1;
  /** The newest accepted frame that waits for preparation. */
  #pending: { event: FrameEvent; receivedAt: number } | null = null;
  #preparing = false;
  /** The newest prepared frame that waits for the display frame. */
  #ready: Prepared | null = null;
  #drawScheduled = false;
  #disposed = false;

  constructor(options: FramePresenterOptions) {
    this.#context = options.context;
    this.#mode = options.mode ?? DEFAULT_PRESENT_MODE;
    this.#space = options.space ?? 'srgb';
    this.#schedule = options.schedule ?? defaultSchedule;
    this.#createBitmap = options.createBitmap === undefined ? defaultCreateBitmap() : options.createBitmap;
    this.#onPresented = options.onPresented ?? (() => undefined);
    this.#onError = options.onError ?? ((error: unknown) => console.error('preview frame draw failed', error));
    this.#now = options.now ?? (() => performance.now());
  }

  /** True when no frame waits for preparation or for a draw. */
  get idle(): boolean {
    return this.#pending === null && !this.#preparing && this.#ready === null;
  }

  /** Accepts a frame for the canvas (see the module documentation). */
  present(event: FrameEvent): void {
    if (this.#disposed) {
      return;
    }
    this.stats.received += 1;
    if (event.frame.seq <= this.#lastAccepted) {
      this.stats.skipped += 1;
      return;
    }
    this.#lastAccepted = event.frame.seq;
    if (this.#pending !== null) {
      this.stats.skipped += 1;
    }
    this.#pending = { event, receivedAt: this.#now() };
    void this.#prepareNext();
  }

  /**
   * Starts a new generation (for example, a new session, where the frame
   * numbers start again at 0). Frames of the old generation are not drawn.
   */
  reset(): void {
    this.#generation += 1;
    this.#lastAccepted = -1;
    if (this.#pending !== null) {
      this.stats.skipped += 1;
      this.#pending = null;
    }
    this.#dropReady();
  }

  /** Stops the presenter and frees the waiting bitmaps. */
  dispose(): void {
    this.#disposed = true;
    this.#pending = null;
    this.#dropReady();
  }

  async #prepareNext(): Promise<void> {
    if (this.#preparing) {
      return;
    }
    this.#preparing = true;
    try {
      while (this.#pending !== null && !this.#disposed) {
        const { event, receivedAt } = this.#pending;
        this.#pending = null;
        const generation = this.#generation;
        let source: ImageBitmap | ImageData;
        try {
          const image = frameImageData(event.frame, this.#space);
          source = this.#mode === 'bitmap' && this.#createBitmap !== null ? await this.#createBitmap(image) : image;
        } catch (error: unknown) {
          this.stats.failed += 1;
          this.#onError(error);
          continue;
        }
        if (this.#disposed || generation !== this.#generation) {
          this.stats.skipped += 1;
          if (isBitmap(source)) {
            source.close();
          }
          continue;
        }
        // Preparation is in order, so this frame is newer than the frame
        // that waits for the display frame.
        this.#dropReady();
        this.#ready = { event, source, generation, receivedAt };
        this.#scheduleDraw();
      }
    } finally {
      this.#preparing = false;
    }
  }

  #scheduleDraw(): void {
    if (this.#drawScheduled) {
      return;
    }
    this.#drawScheduled = true;
    this.#schedule(() => {
      this.#drawScheduled = false;
      this.#draw();
    });
  }

  #draw(): void {
    const ready = this.#ready;
    this.#ready = null;
    if (ready === null) {
      return;
    }
    if (this.#disposed || ready.generation !== this.#generation) {
      this.stats.skipped += 1;
      if (isBitmap(ready.source)) {
        ready.source.close();
      }
      return;
    }
    const started = this.#now();
    try {
      const ctx = this.#context();
      if (ctx === null) {
        throw new Error('The web view has no 2D canvas.');
      }
      const { frame } = ready.event;
      fitCanvas(ctx.canvas, frame.width, frame.height);
      if (isBitmap(ready.source)) {
        ctx.drawImage(ready.source, 0, 0);
      } else {
        ctx.putImageData(ready.source, 0, 0);
      }
    } catch (error: unknown) {
      this.stats.failed += 1;
      this.#onError(error);
      return;
    } finally {
      if (isBitmap(ready.source)) {
        ready.source.close();
      }
    }
    this.stats.presented += 1;
    const finished = this.#now();
    try {
      this.#onPresented({ ...ready.event, drawMs: finished - started, presentMs: finished - ready.receivedAt });
    } catch (error: unknown) {
      this.#onError(error);
    }
  }

  #dropReady(): void {
    if (this.#ready === null) {
      return;
    }
    this.stats.skipped += 1;
    if (isBitmap(this.#ready.source)) {
      this.#ready.source.close();
    }
    this.#ready = null;
  }
}
