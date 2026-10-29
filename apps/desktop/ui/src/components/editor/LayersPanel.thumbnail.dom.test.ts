// @vitest-environment happy-dom
import { cleanup, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, EditorApi, LayerInfo } from '../../lib/editor/api';
import { strokeStubs, thumbnailBuffer } from '../../lib/editor/fakeStrokeApi';
import { EditorStore } from '../../lib/stores/editor.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import LayersPanel from './LayersPanel.svelte';

const NONE = { transparency: false, pixels: false, position: false, all: false };

function masked(version: number): LayerInfo {
  return {
    id: 1,
    name: 'Layer 1',
    kind: 'pixel',
    visible: true,
    opacity: 1,
    fillOpacity: 1,
    blendMode: 'normal',
    hasMask: true,
    maskEnabled: true,
    maskVersion: version,
    adjustment: null,
    isolated: null,
    children: null,
    locks: NONE,
  };
}

function doc(layers: LayerInfo[], revision = 0): DocumentInfo {
  return { documentId: 1, name: 'doc', width: 100, height: 80, layers, canUndo: false, canRedo: false, revision, dirty: false, savedPhotoId: null, sourcePhotoId: 1, hasSelection: false, selectionVersion: 0, canReselect: false };
}

async function makeStore() {
  let layers = [masked(1)];
  let revision = 0;
  const toasts = new ToastStore();
  const api = {
    open: vi.fn(async () => doc(layers)),
    newDocument: vi.fn(async () => doc(layers)),
    render: vi.fn(async () => new ArrayBuffer(0)),
    edit: vi.fn(async () => doc(layers, ++revision)),
    undo: vi.fn(async () => doc(layers, ++revision)),
    redo: vi.fn(async () => doc(layers, ++revision)),
    save: vi.fn(),
    close: vi.fn(async () => undefined),
    selectionOutline: vi.fn(async () => ({ selectionVersion: 0, step: 1, contours: [] })),
    ...strokeStubs(),
  } satisfies EditorApi;
  const store = new EditorStore({
    library: { focusIndex: null, photoAt: () => undefined, selectedIds: [] },
    modules: { active: 'editor', switchTo: vi.fn() } as never,
    toasts,
    api,
  });
  await store.openPhoto(1);
  return { store, api, toasts, setVersion: (v: number) => (layers = [masked(v)]) };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('mask chip thumbnail', () => {
  it('fetches once per maskVersion and draws it', async () => {
    const { store, api, setVersion } = await makeStore();
    render(LayersPanel, { editor: store });
    await flush();
    expect(api.maskThumbnail).toHaveBeenCalledTimes(1);
    const [documentId, layerId, size] = api.maskThumbnail.mock.calls[0] ?? [];
    expect([documentId, layerId]).toEqual([1, 1]);
    expect(size).toBeGreaterThanOrEqual(1);
    expect(size).toBeLessThanOrEqual(256);
    expect(document.querySelector('[data-testid="mask-thumb"]')).not.toBeNull();

    // Same version: no new fetch. New version: one more fetch.
    store.setMaskEnabled(1, true);
    await flush();
    expect(api.maskThumbnail).toHaveBeenCalledTimes(1);
    setVersion(2);
    store.setMaskEnabled(1, true);
    await flush();
    expect(api.maskThumbnail).toHaveBeenCalledTimes(2);
  });

  it('drops a late result for an old version', async () => {
    const { store, api, setVersion } = await makeStore();
    let release: (buffer: ArrayBuffer) => void = () => undefined;
    api.maskThumbnail.mockImplementationOnce(() => new Promise<ArrayBuffer>((resolve) => (release = resolve)));
    render(LayersPanel, { editor: store });
    await flush();
    setVersion(2);
    store.setMaskEnabled(1, true);
    await flush();
    release(thumbnailBuffer(2, 2));
    await flush();
    const old = store.info?.layers[0];
    expect(old?.maskVersion).toBe(2);
    // The result for version 2 came from the second fetch; the late result for version 1 is not used.
    expect(api.maskThumbnail).toHaveBeenCalledTimes(2);
    expect(store.maskThumbnailOf({ ...masked(1) })).toBeNull();
    expect(store.maskThumbnailOf(masked(2))).not.toBeNull();
  });

  it('keeps the plain chip and logs on an error, without a toast', async () => {
    const { store, api, toasts } = await makeStore();
    const log = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    api.maskThumbnail.mockRejectedValueOnce({ kind: 'notFound', message: 'gone' });
    render(LayersPanel, { editor: store });
    await flush();
    expect(log).toHaveBeenCalled();
    expect(toasts.items).toHaveLength(0);
    expect(document.querySelector('[data-testid="mask-thumb"]')).toBeNull();
    expect(document.querySelector('.mask-chip')).not.toBeNull();
  });
});
