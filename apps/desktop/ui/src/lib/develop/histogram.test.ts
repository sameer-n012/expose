import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  clipping,
  computeHistogram,
  HistogramFeed,
  histogramStep,
  type Histogram,
} from './histogram';

function sum(bins: Uint32Array): number {
  return bins.reduce((a, b) => a + b, 0);
}

describe('computeHistogram', () => {
  it('counts each channel and the luma', () => {
    // Two pixels: pure red and white.
    const rgba = new Uint8ClampedArray([255, 0, 0, 255, 255, 255, 255, 255]);
    const h = computeHistogram(rgba, 2, 1, 1);
    expect(h.samples).toBe(2);
    expect(h.r[255]).toBe(2);
    expect(h.g[0]).toBe(1);
    expect(h.g[255]).toBe(1);
    expect(h.b[0]).toBe(1);
    expect(h.luma[Math.round(0.2126 * 255)]).toBe(1);
    expect(h.luma[255]).toBe(1);
    expect(sum(h.luma)).toBe(2);
  });

  it('reads every step-th pixel in x and y', () => {
    // 4x4 frame: pixel (x, y) has red = 10 * (y * 4 + x).
    const rgba = new Uint8ClampedArray(4 * 4 * 4);
    for (let p = 0; p < 16; p++) {
      rgba[p * 4] = 10 * p;
      rgba[p * 4 + 3] = 255;
    }
    const h = computeHistogram(rgba, 4, 4, 2);
    expect(h.samples).toBe(4);
    // Pixels (0,0), (2,0), (0,2), (2,2) are the numbers 0, 2, 8, 10.
    for (const p of [0, 2, 8, 10]) {
      expect(h.r[10 * p]).toBe(1);
    }
    expect(sum(h.r)).toBe(4);
  });

  it('treats a step below 1 as 1', () => {
    const rgba = new Uint8ClampedArray(2 * 2 * 4);
    expect(computeHistogram(rgba, 2, 2, 0).samples).toBe(4);
  });

  it('chooses the step from the frame size', () => {
    expect(histogramStep(400, 300)).toBe(1);
    expect(histogramStep(1000, 600)).toBe(2);
    expect(histogramStep(2560, 1440)).toBe(4);
  });
});

describe('clipping', () => {
  function histogramWith(samples: number, at0: number, at255: number): Histogram {
    const r = new Uint32Array(256);
    r[0] = at0;
    r[255] = at255;
    return { r, g: new Uint32Array(256), b: new Uint32Array(256), luma: new Uint32Array(256), samples };
  }

  it('lights an end above 0.1 percent of the samples', () => {
    expect(clipping(histogramWith(10000, 11, 0))).toEqual({ shadows: true, highlights: false });
    expect(clipping(histogramWith(10000, 0, 11))).toEqual({ shadows: false, highlights: true });
  });

  it('does not light at exactly 0.1 percent or below', () => {
    expect(clipping(histogramWith(10000, 10, 10))).toEqual({ shadows: false, highlights: false });
  });

  it('is off without samples', () => {
    expect(clipping(histogramWith(0, 0, 0))).toEqual({ shadows: false, highlights: false });
  });
});

describe('HistogramFeed', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  function frame(value: number) {
    return { width: 1, height: 1, pixels: new Uint8ClampedArray([value, 0, 0, 255]) };
  }

  it('computes the latest frame at most once per interval', () => {
    let clock = 0;
    const seen: number[] = [];
    const feed = new HistogramFeed({
      onHistogram: (h) => seen.push(h.r.findIndex((n) => n > 0)),
      intervalMs: 100,
      now: () => clock,
    });
    feed.push(frame(1));
    feed.push(frame(2));
    feed.push(frame(3));
    vi.advanceTimersByTime(0);
    // The first run is at once; the latest frame won.
    expect(seen).toEqual([3]);

    clock = 30;
    feed.push(frame(4));
    feed.push(frame(5));
    vi.advanceTimersByTime(69);
    expect(seen).toEqual([3]);
    clock = 100;
    vi.advanceTimersByTime(1);
    expect(seen).toEqual([3, 5]);
  });

  it('drops the waiting frame on cancel', () => {
    const onHistogram = vi.fn();
    const feed = new HistogramFeed({ onHistogram, now: () => 0 });
    feed.push(frame(1));
    feed.cancel();
    vi.advanceTimersByTime(1000);
    expect(onHistogram).not.toHaveBeenCalled();
  });
});
