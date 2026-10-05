/**
 * DOM tests of the crop tool in the shell: the R, Escape, and Enter keys,
 * the geometry flag of the frame requests, and the CSS transform of the
 * preview. The `ipc` transport goes through the mocked `invoke`.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createAppState, type AppState } from '../../lib/appState';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import { FakeCatalog, mockElementSize } from '../../test/fakeCatalog';
import { makeFrameBody } from '../../test/frames';
import AppShell from '../shell/AppShell.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

/** Minimal ImageData for happy-dom. */
class FakeImageData {
  constructor(
    readonly data: Uint8ClampedArray,
    readonly width: number,
    readonly height: number,
  ) {}
}

interface RenderArgs {
  readonly settings: Record<string, unknown>;
  readonly applyGeometry: boolean;
}

let app: AppState;
let toasts: ToastStore;
let restoreSize: () => void;
let renders: RenderArgs[];

beforeEach(() => {
  vi.stubGlobal('ImageData', FakeImageData);
  vi.stubGlobal('createImageBitmap', undefined);
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(function (this: HTMLCanvasElement) {
    return { canvas: this, putImageData: vi.fn(), drawImage: vi.fn() } as unknown as CanvasRenderingContext2D;
  });
  renders = [];
  let seq = 0;
  const fake = new FakeCatalog(3);
  vi.mocked(invoke).mockImplementation((command, args) => {
    if (command === 'preview_render') {
      renders.push(args as unknown as RenderArgs);
      return Promise.resolve(makeFrameBody(4, 3, seq++));
    }
    return fake.invoke(command, args);
  });
  restoreSize = mockElementSize(1000, 600);
  toasts = new ToastStore();
  app = createAppState({ platform: 'mac', toasts });
});

afterEach(() => {
  app.dispose();
  toasts.clear();
  restoreSize();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const lastRender = (): RenderArgs | undefined => renders.at(-1);

async function press(key: string): Promise<void> {
  await fireEvent.keyDown(window, { key });
}

async function openDevelop(): Promise<void> {
  render(AppShell, { modules: app.modules, layout: app.layout, library: app.library, registry: app.registry, toasts });
  await screen.findByRole('toolbar', { name: 'Library toolbar' });
  await waitFor(() => expect(app.library.photoAt(0)).toBeDefined());
  app.library.click(0, { toggle: false, range: false });
  await press('d');
  await waitFor(() => expect(lastRender()?.applyGeometry).toBe(true));
}

function canvas(): HTMLCanvasElement {
  return screen.getByLabelText('Develop preview') as HTMLCanvasElement;
}

describe('crop tool', () => {
  it('R starts the crop tool and requests frames without geometry', async () => {
    await openDevelop();
    await press('r');
    expect(screen.getByRole('button', { name: 'Done' })).toBeTruthy();
    await waitFor(() => expect(lastRender()?.applyGeometry).toBe(false));
    expect(screen.getByTestId('crop-overlay')).toBeTruthy();
  });

  it('shows the angle as a CSS transform and Escape restores the old crop', async () => {
    await openDevelop();
    await press('r');
    await fireEvent.input(screen.getByRole('slider', { name: 'Angle' }), { target: { value: '90' } });
    // 90 is clamped to 45 degrees. The 4:3 image in the 1000 x 600 box zooms out to 600 / 1400 (about 0.4286) to fit the rotated bounding box.
    await waitFor(() => expect(canvas().style.transform).toMatch(/^matrix\(0\.4285/));
    // The crop edits do not request frames in crop mode (no crop field is sent).
    expect(lastRender()?.settings['crop']).toBeUndefined();

    await press('Escape');
    expect(screen.queryByRole('button', { name: 'Done' })).toBeNull();
    expect(screen.getByRole<HTMLInputElement>('spinbutton', { name: 'Angle value' }).value).toBe('0.0');
    await waitFor(() => expect(lastRender()?.applyGeometry).toBe(true));
    expect(lastRender()?.settings['crop']).toBeUndefined();
    expect(canvas().style.transform === '' || canvas().style.transform === 'none').toBe(true);
    expect(screen.queryByTestId('crop-overlay')).toBeNull();
  });

  it('Enter keeps the new crop and returns to frames with geometry', async () => {
    await openDevelop();
    await press('r');
    await fireEvent.input(screen.getByRole('slider', { name: 'Angle' }), { target: { value: '5' } });
    await press('o');
    await press('Enter');
    expect(screen.queryByRole('button', { name: 'Done' })).toBeNull();
    await waitFor(() => expect(lastRender()?.applyGeometry).toBe(true));
    expect(lastRender()?.settings['crop']).toMatchObject({ angle: 5 });
    // Enter and O do nothing outside the crop tool.
    expect(app.registry.has('develop.crop.commit')).toBe(false);
  });
});
