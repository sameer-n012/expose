/**
 * DOM tests of PreviewCanvas: frame requests for settings changes, drawing,
 * and error display. The transport is a fake; the 2D context is a stub,
 * because happy-dom has no canvas rendering.
 */
import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DevelopSettingsJson, FrameTransport, RenderOptions } from '../../lib/develop/transport';
import { makeFrameBody } from '../../test/frames';
import PreviewCanvas from './PreviewCanvas.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

/** Minimal ImageData for happy-dom. */
class FakeImageData {
  constructor(
    readonly data: Uint8ClampedArray,
    readonly width: number,
    readonly height: number,
  ) {}
}

let putImageData: ReturnType<typeof vi.fn>;
let drawImage: ReturnType<typeof vi.fn>;

beforeEach(() => {
  putImageData = vi.fn();
  drawImage = vi.fn();
  vi.stubGlobal('ImageData', FakeImageData);
  // No `createImageBitmap`: the presenter uses its `putImageData` fallback.
  vi.stubGlobal('createImageBitmap', undefined);
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(function (this: HTMLCanvasElement) {
    // The presenter uses only `canvas`, `putImageData`, and `drawImage`.
    return { canvas: this, putImageData, drawImage } as unknown as CanvasRenderingContext2D;
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

function fakeTransport(result: (settings: DevelopSettingsJson) => Promise<ArrayBuffer>) {
  const render = vi.fn((_session: number, settings: DevelopSettingsJson, _options?: RenderOptions) => result(settings));
  const transport: FrameTransport = { name: 'ipc', render };
  return { transport, render };
}

const session = { session: 1, width: 4, height: 3 };

describe('PreviewCanvas', () => {
  it('requests a frame for the settings and draws it', async () => {
    let seq = 0;
    const { transport, render: renderFrame } = fakeTransport(() => Promise.resolve(makeFrameBody(4, 3, seq++)));
    const onframe = vi.fn();
    const view = render(PreviewCanvas, { session, settings: { tone: { exposure: 0 } }, transport, onframe });

    await waitFor(() => expect(putImageData).toHaveBeenCalledOnce());
    // happy-dom has no layout: the box size is 0, so no target is sent.
    expect(renderFrame).toHaveBeenCalledWith(1, { tone: { exposure: 0 } }, { longEdge: null });
    const canvas = screen.getByLabelText('Develop preview') as HTMLCanvasElement;
    expect([canvas.width, canvas.height]).toEqual([4, 3]);
    expect(onframe).toHaveBeenCalledOnce();

    await view.rerender({ session, settings: { tone: { exposure: 1 } }, transport, onframe });
    await waitFor(() => expect(putImageData).toHaveBeenCalledTimes(2));
    expect(renderFrame).toHaveBeenLastCalledWith(1, { tone: { exposure: 1 } }, { longEdge: null });
  });

  it('sends the device-pixel size of the box as the target long edge', async () => {
    vi.spyOn(HTMLDivElement.prototype, 'getBoundingClientRect').mockReturnValue({
      width: 400,
      height: 300,
    } as DOMRect);
    vi.stubGlobal('devicePixelRatio', 2);
    const { transport, render: renderFrame } = fakeTransport(() => Promise.resolve(makeFrameBody(4, 3)));
    const big = { session: 2, width: 2560, height: 1707 };
    render(PreviewCanvas, { session: big, settings: {}, transport });
    await waitFor(() => expect(renderFrame).toHaveBeenCalled());
    // 400 CSS px * 2 = 800 device px wide (the width limits) -> 896.
    expect(renderFrame).toHaveBeenCalledWith(2, {}, { longEdge: 896 });
  });

  it('draws through createImageBitmap when it exists', async () => {
    const close = vi.fn();
    const bitmap = { width: 4, height: 3, close };
    vi.stubGlobal('createImageBitmap', vi.fn(() => Promise.resolve(bitmap)));
    const { transport } = fakeTransport(() => Promise.resolve(makeFrameBody(4, 3)));
    const onframe = vi.fn();
    render(PreviewCanvas, { session, settings: {}, transport, onframe });
    await waitFor(() => expect(drawImage).toHaveBeenCalledWith(bitmap, 0, 0));
    expect(close).toHaveBeenCalledOnce();
    expect(putImageData).not.toHaveBeenCalled();
    expect(onframe).toHaveBeenCalledOnce();
  });

  it('requests nothing without a session', async () => {
    const { transport, render: renderFrame } = fakeTransport(() => Promise.resolve(makeFrameBody(4, 3)));
    render(PreviewCanvas, { session: null, settings: {}, transport });
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(renderFrame).not.toHaveBeenCalled();
  });

  it('shows the message of a failed frame', async () => {
    const { transport } = fakeTransport(() => Promise.reject({ kind: 'internal', message: 'The GPU was lost.' }));
    const onerror = vi.fn();
    render(PreviewCanvas, { session, settings: {}, transport, onerror });
    expect((await screen.findByRole('alert')).textContent).toBe('The GPU was lost.');
    expect(onerror).toHaveBeenCalledOnce();
  });

  it('shows no message for a superseded frame', async () => {
    const { transport, render: renderFrame } = fakeTransport(() =>
      Promise.reject({ kind: 'superseded', message: 'replaced' }),
    );
    render(PreviewCanvas, { session, settings: {}, transport });
    await waitFor(() => expect(renderFrame).toHaveBeenCalled());
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(screen.queryByRole('alert')).toBeNull();
  });
});
