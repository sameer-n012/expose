import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { makeFrameBody } from '../../test/frames';
import { parseFrame } from './frame';
import { FramePresenter, type FramePresenterOptions, type PresentedEvent } from './presenter';
import type { FrameEvent } from './previewClient';

/** Minimal ImageData for Node. */
class FakeImageData {
  constructor(
    readonly data: Uint8ClampedArray,
    readonly width: number,
    readonly height: number,
  ) {}
}

/** A bitmap stand-in that records its frame and whether it was closed. */
class FakeBitmap {
  closed = false;
  constructor(readonly seqTag: number) {}
  close(): void {
    this.closed = true;
  }
}

beforeEach(() => {
  vi.stubGlobal('ImageData', FakeImageData);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

const event = (seq: number, width = 2, height = 2): FrameEvent => ({
  frame: parseFrame(makeFrameBody(width, height, seq, { fill: seq })),
  roundTripMs: 1,
});

function setup(overrides: Partial<FramePresenterOptions> = {}) {
  const canvas = { width: 0, height: 0 } as HTMLCanvasElement;
  const putImageData = vi.fn();
  const drawImage = vi.fn();
  const ctx = { canvas, putImageData, drawImage } as unknown as CanvasRenderingContext2D;
  const scheduled: Array<() => void> = [];
  const presented: PresentedEvent[] = [];
  const errors: unknown[] = [];
  const presenter = new FramePresenter({
    context: () => ctx,
    schedule: (callback) => scheduled.push(callback),
    createBitmap: null,
    onPresented: (e) => presented.push(e),
    onError: (e) => errors.push(e),
    ...overrides,
  });
  /** Runs the waiting display-frame callbacks. */
  const tick = (): void => {
    for (const callback of scheduled.splice(0)) {
      callback();
    }
  };
  return { presenter, canvas, putImageData, drawImage, scheduled, presented, errors, tick };
}

/** Lets pending promise callbacks run. */
const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

describe('FramePresenter', () => {
  it('draws a frame with putImageData at the next display frame', () => {
    const { presenter, canvas, putImageData, presented, scheduled, tick } = setup();
    presenter.present(event(0, 4, 3));
    expect(putImageData).not.toHaveBeenCalled();
    expect(scheduled).toHaveLength(1);
    tick();
    expect(putImageData).toHaveBeenCalledOnce();
    expect(putImageData.mock.calls[0]?.[0]).toBeInstanceOf(FakeImageData);
    expect([canvas.width, canvas.height]).toEqual([4, 3]);
    expect(presented.map((e) => e.frame.seq)).toEqual([0]);
    expect(presenter.idle).toBe(true);
  });

  it('re-fits the canvas when the frame size changes (for example, a crop)', () => {
    const { presenter, canvas, putImageData, tick } = setup();
    presenter.present(event(0, 6, 4));
    tick();
    expect([canvas.width, canvas.height]).toEqual([6, 4]);
    // A crop gives a smaller frame with another aspect ratio.
    presenter.present(event(1, 3, 4));
    tick();
    expect([canvas.width, canvas.height]).toEqual([3, 4]);
    expect(putImageData.mock.calls[1]?.[0]).toMatchObject({ width: 3, height: 4 });
    // No crop again: the full size.
    presenter.present(event(2, 6, 4));
    tick();
    expect([canvas.width, canvas.height]).toEqual([6, 4]);
    expect(putImageData).toHaveBeenCalledTimes(3);
  });

  it('never draws a stale frame after a newer one', () => {
    const { presenter, putImageData, presented, tick } = setup();
    presenter.present(event(2));
    tick();
    presenter.present(event(1));
    presenter.present(event(2));
    tick();
    expect(putImageData).toHaveBeenCalledOnce();
    expect(presented.map((e) => e.frame.seq)).toEqual([2]);
    expect(presenter.stats).toMatchObject({ received: 3, presented: 1, skipped: 2 });
  });

  it('draws only the newest frame per display frame', () => {
    const { presenter, putImageData, presented, scheduled, tick } = setup();
    presenter.present(event(0));
    presenter.present(event(1));
    presenter.present(event(2));
    expect(scheduled).toHaveLength(1);
    tick();
    expect(putImageData).toHaveBeenCalledOnce();
    expect(presented.map((e) => e.frame.seq)).toEqual([2]);
    expect(presenter.stats.skipped).toBe(2);
  });

  it('prepares bitmaps in order, replaces waiting frames, and closes unused bitmaps', async () => {
    const resolvers: Array<(bitmap: ImageBitmap) => void> = [];
    const createBitmap = vi.fn(
      () => new Promise<ImageBitmap>((resolve) => resolvers.push(resolve)),
    );
    const { presenter, drawImage, putImageData, presented, tick } = setup({ createBitmap, mode: 'bitmap' });
    presenter.present(event(0));
    presenter.present(event(1));
    presenter.present(event(2));
    // One preparation at a time; frame 1 was replaced by frame 2.
    expect(createBitmap).toHaveBeenCalledTimes(1);
    const first = new FakeBitmap(0);
    resolvers[0]?.(first as unknown as ImageBitmap);
    await flush();
    expect(createBitmap).toHaveBeenCalledTimes(2);
    const second = new FakeBitmap(2);
    resolvers[1]?.(second as unknown as ImageBitmap);
    await flush();
    // Frame 2 was ready before the display frame: frame 0 is not drawn.
    tick();
    expect(first.closed).toBe(true);
    expect(drawImage).toHaveBeenCalledOnce();
    expect(drawImage.mock.calls[0]?.[0]).toBe(second);
    expect(second.closed).toBe(true);
    expect(putImageData).not.toHaveBeenCalled();
    expect(presented.map((e) => e.frame.seq)).toEqual([2]);
    expect(presenter.stats).toMatchObject({ received: 3, presented: 1, skipped: 2 });
  });

  it('falls back to putImageData when createImageBitmap does not exist', () => {
    const { presenter, putImageData, drawImage, tick } = setup({ createBitmap: null, mode: 'bitmap' });
    presenter.present(event(0));
    tick();
    expect(putImageData).toHaveBeenCalledOnce();
    expect(drawImage).not.toHaveBeenCalled();
  });

  it('drops frames of an old generation and accepts new numbers after a reset', () => {
    const { presenter, presented, tick } = setup();
    presenter.present(event(5));
    presenter.reset();
    tick();
    expect(presented).toHaveLength(0);
    presenter.present(event(0));
    tick();
    expect(presented.map((e) => e.frame.seq)).toEqual([0]);
  });

  it('closes a bitmap that becomes ready after a reset', async () => {
    let resolve: ((bitmap: ImageBitmap) => void) | undefined;
    const createBitmap = () => new Promise<ImageBitmap>((r) => (resolve = r));
    const { presenter, drawImage, scheduled } = setup({ createBitmap, mode: 'bitmap' });
    presenter.present(event(0));
    presenter.reset();
    const bitmap = new FakeBitmap(0);
    resolve?.(bitmap as unknown as ImageBitmap);
    await flush();
    expect(bitmap.closed).toBe(true);
    expect(scheduled).toHaveLength(0);
    expect(drawImage).not.toHaveBeenCalled();
  });

  it('reports a missing 2D context and a failed preparation', async () => {
    const missing = setup({ context: () => null });
    missing.presenter.present(event(0));
    missing.tick();
    expect(missing.errors).toHaveLength(1);
    expect(missing.presenter.stats.failed).toBe(1);

    const failing = setup({ createBitmap: () => Promise.reject(new Error('decode')), mode: 'bitmap' });
    failing.presenter.present(event(0));
    await flush();
    expect(failing.errors).toHaveLength(1);
    expect(failing.scheduled).toHaveLength(0);
  });

  it('does nothing after dispose', () => {
    const { presenter, putImageData, tick } = setup();
    presenter.present(event(0));
    presenter.dispose();
    presenter.present(event(1));
    tick();
    expect(putImageData).not.toHaveBeenCalled();
  });

  it('closes a ready bitmap at dispose and does not draw it', async () => {
    const bitmap = new FakeBitmap(0);
    const { presenter, drawImage, scheduled, tick } = setup({
      createBitmap: () => Promise.resolve(bitmap as unknown as ImageBitmap),
      mode: 'bitmap',
    });
    presenter.present(event(0));
    await flush();
    expect(scheduled).toHaveLength(1);
    presenter.dispose();
    expect(bitmap.closed).toBe(true);
    // The display-frame callback that was already scheduled draws nothing.
    tick();
    expect(drawImage).not.toHaveBeenCalled();
    expect(presenter.idle).toBe(true);
  });

  it('closes a bitmap that becomes ready after dispose', async () => {
    let resolve: ((bitmap: ImageBitmap) => void) | undefined;
    const createBitmap = () => new Promise<ImageBitmap>((r) => (resolve = r));
    const { presenter, drawImage, scheduled } = setup({ createBitmap, mode: 'bitmap' });
    presenter.present(event(0));
    presenter.dispose();
    const bitmap = new FakeBitmap(0);
    resolve?.(bitmap as unknown as ImageBitmap);
    await flush();
    expect(bitmap.closed).toBe(true);
    expect(scheduled).toHaveLength(0);
    expect(drawImage).not.toHaveBeenCalled();
    expect(presenter.idle).toBe(true);
  });

  it('closes the bitmap when the draw fails', async () => {
    const bitmap = new FakeBitmap(0);
    const { presenter, errors, tick } = setup({
      context: () => null,
      createBitmap: () => Promise.resolve(bitmap as unknown as ImageBitmap),
      mode: 'bitmap',
    });
    presenter.present(event(0));
    await flush();
    tick();
    expect(errors).toHaveLength(1);
    expect(bitmap.closed).toBe(true);
  });

  it('closes every bitmap it makes in a long stream', async () => {
    const made: FakeBitmap[] = [];
    const { presenter, tick } = setup({
      createBitmap: () => {
        const bitmap = new FakeBitmap(made.length);
        made.push(bitmap);
        return Promise.resolve(bitmap as unknown as ImageBitmap);
      },
      mode: 'bitmap',
    });
    for (let seq = 0; seq < 50; seq += 1) {
      presenter.present(event(seq));
      await flush();
      if (seq % 3 === 0) {
        tick();
      }
    }
    tick();
    presenter.dispose();
    expect(made.length).toBeGreaterThan(0);
    expect(made.filter((b) => !b.closed)).toHaveLength(0);
  });
});
