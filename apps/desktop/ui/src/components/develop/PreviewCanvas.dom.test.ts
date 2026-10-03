/**
 * DOM tests of PreviewCanvas: frame requests for settings changes, drawing,
 * and error display. The transport is a fake; the 2D context is a stub,
 * because happy-dom has no canvas rendering.
 */
import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DevelopSettingsJson, FrameTransport } from '../../lib/develop/transport';
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

beforeEach(() => {
  putImageData = vi.fn();
  vi.stubGlobal('ImageData', FakeImageData);
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(function (this: HTMLCanvasElement) {
    // The component uses only `canvas` and `putImageData` in `put` mode.
    return { canvas: this, putImageData } as unknown as CanvasRenderingContext2D;
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

function fakeTransport(result: (settings: DevelopSettingsJson) => Promise<ArrayBuffer>) {
  const render = vi.fn((_session: number, settings: DevelopSettingsJson) => result(settings));
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
    expect(renderFrame).toHaveBeenCalledWith(1, { tone: { exposure: 0 } });
    const canvas = screen.getByLabelText('Develop preview') as HTMLCanvasElement;
    expect([canvas.width, canvas.height]).toEqual([4, 3]);
    expect(onframe).toHaveBeenCalledOnce();

    await view.rerender({ session, settings: { tone: { exposure: 1 } }, transport, onframe });
    await waitFor(() => expect(putImageData).toHaveBeenCalledTimes(2));
    expect(renderFrame).toHaveBeenLastCalledWith(1, { tone: { exposure: 1 } });
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
