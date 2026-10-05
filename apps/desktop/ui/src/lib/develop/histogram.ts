/**
 * The RGB histogram of a preview frame, and the throttled feed that
 * computes it.
 *
 * The input is the frame that the user sees: 8-bit, display-encoded RGBA.
 * Thus the histogram matches the screen.
 *
 * # Subsampling
 *
 * `computeHistogram` reads every `step`-th pixel in x and in y. For a large
 * frame, `histogramStep` returns 4 (1/16 of the pixels), so the work stays
 * small on the main thread. The shape of the histogram stays the same.
 */

/** Counts of 8-bit values for each channel, and for the luma. */
export interface Histogram {
  readonly r: Uint32Array;
  readonly g: Uint32Array;
  readonly b: Uint32Array;
  /** Luma of the display values (Rec. 709 weights), rounded to 8 bits. */
  readonly luma: Uint32Array;
  /** The number of sampled pixels. */
  readonly samples: number;
}

/** The share of sampled pixels at 0 (or 255) above which the clip indicator lights. */
export const CLIP_THRESHOLD = 0.001;

/** Pixel counts above which `histogramStep` returns 2 and 4. */
const MEDIUM_PIXELS = 250_000;
const LARGE_PIXELS = 1_000_000;

/** The sampling step for a frame size: 1 (small), 2 (medium), or 4 (large). */
export function histogramStep(width: number, height: number): number {
  const pixels = width * height;
  if (pixels > LARGE_PIXELS) {
    return 4;
  }
  return pixels > MEDIUM_PIXELS ? 2 : 1;
}

/**
 * Counts the values of every `step`-th pixel in x and y (starting at the
 * origin). `step` below 1 or not an integer is rounded to a whole number of
 * at least 1. Alpha is ignored (display frames are opaque).
 */
export function computeHistogram(
  rgba: Uint8ClampedArray | Uint8Array,
  width: number,
  height: number,
  step: number,
): Histogram {
  const stride = Math.max(1, Math.floor(step));
  const r = new Uint32Array(256);
  const g = new Uint32Array(256);
  const b = new Uint32Array(256);
  const luma = new Uint32Array(256);
  let samples = 0;
  for (let y = 0; y < height; y += stride) {
    let i = y * width * 4;
    for (let x = 0; x < width; x += stride) {
      const red = rgba[i] ?? 0;
      const green = rgba[i + 1] ?? 0;
      const blue = rgba[i + 2] ?? 0;
      r[red] = (r[red] ?? 0) + 1;
      g[green] = (g[green] ?? 0) + 1;
      b[blue] = (b[blue] ?? 0) + 1;
      const l = Math.round(0.2126 * red + 0.7152 * green + 0.0722 * blue);
      luma[l] = (luma[l] ?? 0) + 1;
      samples += 1;
      i += stride * 4;
    }
  }
  return { r, g, b, luma, samples };
}

/** Which ends of the range have clipped values in any channel. */
export interface Clipping {
  readonly shadows: boolean;
  readonly highlights: boolean;
}

/** True when more than `threshold` of the sampled pixels sit at 0 (or at 255) in any channel. */
export function clipping(histogram: Histogram, threshold: number = CLIP_THRESHOLD): Clipping {
  if (histogram.samples === 0) {
    return { shadows: false, highlights: false };
  }
  const limit = histogram.samples * threshold;
  const channels = [histogram.r, histogram.g, histogram.b];
  return {
    shadows: channels.some((c) => (c[0] ?? 0) > limit),
    highlights: channels.some((c) => (c[255] ?? 0) > limit),
  };
}

/** The minimum time between two histogram computations, in milliseconds. */
export const HISTOGRAM_INTERVAL_MS = 100;

/** The part of a frame that the feed needs. */
export interface HistogramSource {
  readonly width: number;
  readonly height: number;
  readonly pixels: Uint8ClampedArray | Uint8Array;
}

export interface HistogramFeedOptions {
  readonly onHistogram: (histogram: Histogram) => void;
  readonly intervalMs?: number;
  /** The clock in milliseconds. Default `performance.now`. */
  readonly now?: () => number;
}

/**
 * Computes the histogram of the newest pushed frame, at most once per
 * interval. `push` only stores the frame reference and sets a timer, so it
 * never blocks a draw. A frame that a newer frame replaces is not read.
 */
export class HistogramFeed {
  readonly #onHistogram: (histogram: Histogram) => void;
  readonly #interval: number;
  readonly #now: () => number;
  #latest: HistogramSource | null = null;
  #timer: ReturnType<typeof setTimeout> | null = null;
  #lastRun = Number.NEGATIVE_INFINITY;

  constructor(options: HistogramFeedOptions) {
    this.#onHistogram = options.onHistogram;
    this.#interval = options.intervalMs ?? HISTOGRAM_INTERVAL_MS;
    this.#now = options.now ?? (() => performance.now());
  }

  /** Offers a frame. The latest frame wins. */
  push(frame: HistogramSource): void {
    this.#latest = frame;
    if (this.#timer !== null) {
      return;
    }
    const wait = Math.max(0, this.#lastRun + this.#interval - this.#now());
    this.#timer = setTimeout(() => this.#run(), wait);
  }

  /** Drops the waiting frame and the timer. */
  cancel(): void {
    this.#latest = null;
    if (this.#timer !== null) {
      clearTimeout(this.#timer);
      this.#timer = null;
    }
  }

  #run(): void {
    this.#timer = null;
    const frame = this.#latest;
    this.#latest = null;
    if (frame === null) {
      return;
    }
    this.#lastRun = this.#now();
    this.#onHistogram(
      computeHistogram(frame.pixels, frame.width, frame.height, histogramStep(frame.width, frame.height)),
    );
  }
}
