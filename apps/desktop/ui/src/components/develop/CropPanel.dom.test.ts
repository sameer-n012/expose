/** DOM tests of the Crop panel and the crop overlay: controls update the store. */
import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import { pixelAspect } from '../../lib/develop/cropGeometry';
import type { DevelopDocument } from '../../lib/develop/settings';
import { DevelopStore, type DevelopApi } from '../../lib/stores/develop.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import CropOverlay from './CropOverlay.svelte';
import CropPanel from './CropPanel.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const INITIAL: DevelopDocument = {
  version: 1,
  white_balance: { mode: 'as_shot', temperature: 5500, tint: 0 },
  tone: { exposure: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0 },
};

const SIZE = { width: 300, height: 200 };

async function makeStore(session = true): Promise<DevelopStore> {
  const api: DevelopApi = {
    getSettings: async () => INITIAL,
    setSettings: vi.fn(async (_id, s) => s),
    openPreview: session
      ? async () => ({ session: 1, ...SIZE })
      : async () => {
          throw new Error('no preview');
        },
    closePreview: async () => undefined,
  };
  const store = new DevelopStore({
    library: { focusIndex: null, photoAt: () => undefined, selectedIds: [] },
    modules: { active: 'develop' },
    toasts: new ToastStore(),
    api,
  });
  await store.activate(1);
  return store;
}

describe('CropPanel', () => {
  it('starts, cancels, and ends the crop tool', async () => {
    const store = await makeStore();
    render(CropPanel, { develop: store });
    await fireEvent.click(screen.getByRole('button', { name: 'Crop' }));
    expect(store.cropActive).toBe(true);
    await fireEvent.input(screen.getByRole('slider', { name: 'Angle' }), { target: { value: '12' } });
    expect(store.crop?.angle).toBe(12);
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(store.cropActive).toBe(false);
    expect(store.crop?.angle).toBe(0);

    await fireEvent.click(screen.getByRole('button', { name: 'Crop' }));
    await fireEvent.input(screen.getByRole('slider', { name: 'Angle' }), { target: { value: '-4.5' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Done' }));
    expect(store.cropActive).toBe(false);
    expect(store.crop?.angle).toBe(-4.5);
    expect(screen.getByRole<HTMLInputElement>('spinbutton', { name: 'Angle value' }).value).toBe('-4.5');
  });

  it('the angle shrinks the rect, and a double-click resets the angle', async () => {
    const store = await makeStore();
    render(CropPanel, { develop: store });
    await fireEvent.input(screen.getByRole('slider', { name: 'Angle' }), { target: { value: '10' } });
    expect(store.crop?.rect.width).toBeLessThan(1);
    await fireEvent.dblClick(screen.getByRole('slider', { name: 'Angle' }));
    expect(store.crop?.angle).toBe(0);
    expect(store.crop?.rect.width).toBeCloseTo(1, 9);
  });

  it('shows the fine grid only while the angle drags in crop mode', async () => {
    const store = await makeStore();
    render(CropPanel, { develop: store });
    const slider = screen.getByRole('slider', { name: 'Angle' });
    await fireEvent.pointerDown(slider);
    expect(store.straightening).toBe(false);
    store.enterCrop();
    await fireEvent.pointerDown(slider);
    expect(store.straightening).toBe(true);
    await fireEvent.pointerUp(window);
    expect(store.straightening).toBe(false);
  });

  it('flips, sets the aspect, locks, swaps, and resets', async () => {
    const store = await makeStore();
    render(CropPanel, { develop: store });
    const flipH = screen.getByRole('button', { name: 'Flip horizontal' });
    await fireEvent.click(flipH);
    expect(store.crop?.flip_horizontal).toBe(true);
    expect(flipH.getAttribute('aria-pressed')).toBe('true');
    await fireEvent.click(screen.getByRole('button', { name: 'Flip vertical' }));
    expect(store.crop?.flip_vertical).toBe(true);

    await fireEvent.change(screen.getByRole('combobox', { name: 'Aspect' }), { target: { value: '1:1' } });
    expect(store.crop?.aspect).toEqual({ ratio: { width: 1, height: 1 } });
    expect(pixelAspect(SIZE, store.crop?.rect ?? { left: 0, top: 0, width: 1, height: 1 })).toBeCloseTo(1, 9);

    const lock = screen.getByRole('button', { name: 'Lock aspect' });
    expect(lock.getAttribute('aria-pressed')).toBe('true');
    await fireEvent.click(lock);
    expect(store.crop?.aspect).toBe('free');
    await fireEvent.click(lock);
    expect(store.crop?.aspect).toEqual({ ratio: { width: 1, height: 1 } });

    await fireEvent.change(screen.getByRole('combobox', { name: 'Aspect' }), { target: { value: '3:2' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Swap orientation' }));
    expect(store.crop?.aspect).toEqual({ ratio: { width: 2, height: 3 } });

    await fireEvent.change(screen.getByRole('combobox', { name: 'Overlay' }), { target: { value: 'golden' } });
    expect(store.cropOverlay).toBe('golden');

    await fireEvent.click(screen.getByRole('button', { name: 'Reset crop' }));
    expect(store.crop).toEqual({
      rect: { left: 0, top: 0, width: 1, height: 1 },
      angle: 0,
      flip_horizontal: false,
      flip_vertical: false,
      aspect: 'original',
    });
  });

  it('is disabled without a preview session', async () => {
    const store = await makeStore(false);
    render(CropPanel, { develop: store });
    expect(screen.getByRole<HTMLButtonElement>('button', { name: 'Crop' }).disabled).toBe(true);
    expect(screen.getByRole<HTMLInputElement>('slider', { name: 'Angle' }).disabled).toBe(true);
  });
});

describe('CropOverlay', () => {
  const content = { x: 0, y: 0, width: 300, height: 200 };

  it('moves the rect with a drag inside and clamps it at the frame', async () => {
    const store = await makeStore();
    store.setCropAspect('free');
    store.setCropRect({ left: 0.25, top: 0.25, width: 0.5, height: 0.5 });
    render(CropOverlay, { develop: store, content });
    const rect = screen.getByTestId('crop-rect');
    expect(rect.style.left).toBe('25%');
    await fireEvent.pointerDown(rect, { pointerId: 1, button: 0, clientX: 100, clientY: 100 });
    await fireEvent.pointerMove(rect, { pointerId: 1, clientX: 130, clientY: 100 });
    expect(store.crop?.rect.left).toBeCloseTo(0.35, 9);
    // Far to the right: the rect stops at the frame edge.
    await fireEvent.pointerMove(rect, { pointerId: 1, clientX: 400, clientY: 100 });
    expect(store.crop?.rect.left).toBeCloseTo(0.5, 6);
    await fireEvent.pointerUp(rect, { pointerId: 1 });
    await fireEvent.pointerMove(rect, { pointerId: 1, clientX: 0, clientY: 100 });
    expect(store.crop?.rect.left).toBeCloseTo(0.5, 6);
  });

  it('resizes with a corner handle and keeps the locked ratio', async () => {
    const store = await makeStore();
    store.setCropAspect({ ratio: { width: 1, height: 1 } });
    store.setCropRect({ left: 0.4, top: 0.35, width: 0.2, height: 0.3 });
    render(CropOverlay, { develop: store, content });
    const handle = screen.getByTestId('crop-rect').querySelector('[data-handle="se"]');
    expect(handle).not.toBeNull();
    await fireEvent.pointerDown(handle as Element, { pointerId: 2, button: 0, clientX: 180, clientY: 130 });
    await fireEvent.pointerMove(handle as Element, { pointerId: 2, clientX: 210, clientY: 131 });
    const r = store.crop?.rect;
    expect(r?.left).toBeCloseTo(0.4, 9);
    expect(r?.width).toBeCloseTo(0.3, 9);
    expect(pixelAspect(SIZE, r ?? { left: 0, top: 0, width: 1, height: 1 })).toBeCloseTo(1, 9);
  });

  it('shows the guide lines of the overlay', async () => {
    const store = await makeStore();
    const { container } = render(CropOverlay, { develop: store, content });
    expect(container.querySelectorAll('line')).toHaveLength(4);
  });
});
