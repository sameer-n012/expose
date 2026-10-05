/**
 * DOM tests of LoupeImage: the thumbnail placeholder, then the preview
 * canvas after the first frame. The transport is a fake; the 2D context is
 * a stub, because happy-dom has no canvas rendering.
 */
import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Photo } from '../../lib/catalog';
import type { DevelopDocument } from '../../lib/develop/settings';
import type { FrameTransport } from '../../lib/develop/transport';
import type { LoupeApi } from '../../lib/library/loupe.svelte';
import { makeFrameBody } from '../../test/frames';
import LoupeImage from './LoupeImage.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

class FakeImageData {
  constructor(
    readonly data: Uint8ClampedArray,
    readonly width: number,
    readonly height: number,
  ) {}
}

beforeEach(() => {
  vi.stubGlobal('ImageData', FakeImageData);
  vi.stubGlobal('createImageBitmap', undefined);
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(function (this: HTMLCanvasElement) {
    return { canvas: this, putImageData: vi.fn(), drawImage: vi.fn() } as unknown as CanvasRenderingContext2D;
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const photo = { id: 5, fileName: 'a.jpg', fileModifiedMs: 1, captureTimeMs: null } as unknown as Photo;

function makeApi(): LoupeApi {
  return {
    getSettings: vi.fn(async () => ({ version: 1 }) as unknown as DevelopDocument),
    openPreview: vi.fn(async () => ({ session: 1, width: 4, height: 3 }) as never),
    closePreview: vi.fn(async () => undefined),
  };
}

describe('LoupeImage', () => {
  it('shows the thumbnail, then the canvas after the first frame', async () => {
    let release: (body: ArrayBuffer) => void = () => undefined;
    const transport: FrameTransport = {
      name: 'ipc',
      render: () => new Promise((resolve) => (release = resolve)),
    };
    render(LoupeImage, { photo, api: makeApi(), transport });

    const thumb = screen.getByTestId('loupe-thumbnail') as HTMLImageElement;
    expect(thumb.src).toContain('/5');
    const canvasBox = await screen.findByTestId('loupe-canvas');
    expect(canvasBox.classList.contains('hidden')).toBe(true);

    await waitFor(() => expect(release).not.toBe(undefined));
    release(makeFrameBody(4, 3, 0));
    await waitFor(() => expect(canvasBox.classList.contains('hidden')).toBe(false));
  });

  it('keeps the thumbnail and shows a note when the preview does not open', async () => {
    const api = makeApi();
    api.openPreview = vi.fn(async () => {
      throw new Error('no decode');
    });
    render(LoupeImage, { photo, api });
    await screen.findByTestId('loupe-error');
    expect(screen.getByTestId('loupe-thumbnail')).toBeTruthy();
    expect(screen.queryByTestId('loupe-canvas')).toBeNull();
  });

  it('closes the session when it is destroyed', async () => {
    const api = makeApi();
    const view = render(LoupeImage, { photo, api, transport: { name: 'ipc', render: () => new Promise(() => undefined) } });
    await screen.findByTestId('loupe-canvas');
    view.unmount();
    expect(api.closePreview).toHaveBeenCalledWith(1);
  });
});
