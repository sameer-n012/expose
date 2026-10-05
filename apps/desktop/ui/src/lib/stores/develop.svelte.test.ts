import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DevelopDocument } from '../develop/settings';
import { DevelopStore, SAVE_DELAY_MS, type DevelopApi } from './develop.svelte';
import { ToastStore } from './toasts.svelte';

function doc(extra: Record<string, unknown> = {}): DevelopDocument {
  return {
    version: 1,
    white_balance: { mode: 'as_shot', temperature: 5500, tint: 0 },
    tone: { exposure: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0 },
    tone_curve: { master: [{ x: 0, y: 0 }, { x: 1, y: 1 }] },
    ...extra,
  };
}

function setup(initial: DevelopDocument = doc()) {
  const api = {
    getSettings: vi.fn(async (_id: number) => initial),
    setSettings: vi.fn(async (_id: number, s: DevelopDocument) => s),
    openPreview: vi.fn(async (id: number) => ({ session: id * 10, width: 4, height: 3 }) as never),
    closePreview: vi.fn(async (_s: number) => undefined),
  } satisfies DevelopApi;
  const toasts = new ToastStore();
  const modules = { active: 'develop' as const };
  const library = {
    focusIndex: null as number | null,
    photoAt: () => undefined,
    selectedIds: [] as number[],
  };
  const store = new DevelopStore({ library, modules, toasts, api });
  return { api, store, toasts, library, modules };
}

describe('DevelopStore', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('is empty without a photo and follows the first selected photo', async () => {
    const { store, library } = setup();
    expect(store.target).toBeNull();
    library.selectedIds = [7, 8];
    expect(store.target).toBe(7);
    await store.activate(null);
    expect(store.status).toBe('empty');
  });

  it('loads the settings and opens a session', async () => {
    const { store, api } = setup();
    await store.activate(3);
    expect(store.status).toBe('ready');
    expect(store.settings?.tone.exposure).toBe(0);
    expect(store.session?.session).toBe(30);
    expect(api.openPreview).toHaveBeenCalledWith(3);
  });

  it('shows a stored settings error', async () => {
    const { store, api } = setup();
    api.getSettings.mockRejectedValueOnce({ kind: 'internal', message: 'bad stored settings' });
    await store.activate(3);
    expect(store.status).toBe('error');
    expect(store.loadError).toBe('bad stored settings');
    expect(api.closePreview).toHaveBeenCalledWith(30);
  });

  it('saves after the quiet time, once for many edits', async () => {
    const { store, api } = setup();
    await store.activate(3);
    store.setSlider('exposure', 0.5);
    store.setSlider('exposure', 1);
    expect(api.setSettings).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS + 1);
    expect(api.setSettings).toHaveBeenCalledTimes(1);
    expect(api.setSettings.mock.calls[0]?.[1].tone.exposure).toBe(1);
  });

  it('keeps unknown fields when it saves', async () => {
    const { store, api } = setup(doc({ future: { a: 1 }, tone: { exposure: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0, vibrance: 9 } }));
    await store.activate(3);
    store.setSlider('contrast', 20);
    await store.flush();
    const saved = api.setSettings.mock.calls[0]?.[1];
    expect(saved?.future).toEqual({ a: 1 });
    expect(saved?.tone.vibrance).toBe(9);
    expect(saved?.tone.contrast).toBe(20);
    expect(saved?.tone_curve).toEqual({ master: [{ x: 0, y: 0 }, { x: 1, y: 1 }] });
  });

  it('switches the white balance to custom when a white balance slider moves', async () => {
    const { store } = setup();
    await store.activate(3);
    store.setSlider('temperature', 6500);
    expect(store.settings?.white_balance).toMatchObject({ mode: 'custom', temperature: 6500 });
  });

  it('clamps slider values and ignores non-finite values', async () => {
    const { store } = setup();
    await store.activate(3);
    store.setSlider('exposure', 99);
    expect(store.settings?.tone.exposure).toBe(5);
    store.setSlider('exposure', Number.NaN);
    expect(store.settings?.tone.exposure).toBe(5);
  });

  it('resets one slider and all sliders', async () => {
    const { store } = setup();
    await store.activate(3);
    store.setSlider('contrast', 30);
    store.setSlider('tint', 10);
    store.resetSlider('contrast');
    expect(store.settings?.tone.contrast).toBe(0);
    expect(store.settings?.white_balance.tint).toBe(10);
    store.resetAll();
    expect(store.settings?.white_balance).toMatchObject({ mode: 'as_shot', tint: 0 });
  });

  it('flushes the edits when the photo changes', async () => {
    const { store, api } = setup();
    await store.activate(3);
    store.setSlider('shadows', 40);
    await store.activate(4);
    expect(api.setSettings).toHaveBeenCalledTimes(1);
    expect(api.setSettings.mock.calls[0]?.[0]).toBe(3);
    expect(api.closePreview).toHaveBeenCalledWith(30);
    expect(store.photoId).toBe(4);
  });

  it('flushes when the module changes (target becomes null) and on dispose', async () => {
    const { store, api, modules, library } = setup();
    library.selectedIds = [3];
    await store.activate(store.target);
    store.setSlider('whites', 10);
    (modules as { active: string }).active = 'library';
    expect(store.target).toBeNull();
    await store.activate(store.target);
    expect(api.setSettings).toHaveBeenCalledTimes(1);

    await store.activate(3);
    store.setSlider('blacks', -10);
    await store.dispose();
    expect(api.setSettings).toHaveBeenCalledTimes(2);
  });

  it('sets, clamps, saves, and resets sharpening', async () => {
    const { store, api } = setup();
    await store.activate(3);
    store.setSharpening('amount', 60);
    store.setSharpening('radius', 9);
    expect(store.settings?.sharpening).toEqual({ amount: 60, radius: 3, detail: 25, masking: 0 });
    store.setSharpening('masking', Number.NaN);
    expect(store.settings?.sharpening?.masking).toBe(0);
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
    expect(api.setSettings).toHaveBeenCalledTimes(1);
    store.resetSharpenKey('amount');
    expect(store.settings?.sharpening?.amount).toBe(0);
    store.resetSharpening();
    expect(store.settings?.sharpening).toEqual({ amount: 0, radius: 1, detail: 25, masking: 0 });
  });

  it('does not add a sharpening field when resetting an old document', async () => {
    const { store } = setup();
    await store.activate(3);
    store.resetSharpening();
    expect(store.settings).not.toHaveProperty('sharpening');
  });

  it('shows a toast when a save fails', async () => {
    const { store, api, toasts } = setup();
    api.setSettings.mockRejectedValueOnce({ kind: 'invalidInput', message: 'no' });
    await store.activate(3);
    store.setSlider('contrast', 5);
    await store.flush();
    expect(toasts.items.some((t) => t.kind === 'error')).toBe(true);
  });

  it('drops the answer for an old photo', async () => {
    const { store, api } = setup();
    let release: (value: DevelopDocument) => void = () => undefined;
    api.getSettings.mockImplementationOnce(() => new Promise((resolve) => (release = resolve)));
    const first = store.activate(3);
    const second = store.activate(4);
    release(doc());
    await Promise.all([first, second]);
    expect(store.photoId).toBe(4);
    expect(store.session?.session).toBe(40);
    expect(api.closePreview).toHaveBeenCalledWith(30);
  });
});
