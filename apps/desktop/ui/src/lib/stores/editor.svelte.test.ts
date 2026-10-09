import { describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, EditDto, EditorApi, LayerInfo } from '../editor/api';
import { EditorStore, GESTURE_IDLE_MS } from './editor.svelte';
import { ToastStore } from './toasts.svelte';

function layer(id: number, extra: Partial<LayerInfo> = {}): LayerInfo {
  return {
    id,
    name: `L${id}`,
    kind: 'pixel',
    visible: true,
    opacity: 1,
    fillOpacity: 1,
    blendMode: 'normal',
    hasMask: false,
    adjustment: null,
    isolated: null,
    children: null,
    ...extra,
  };
}

function info(documentId: number, revision: number, layers: LayerInfo[] = [layer(1), layer(2)], extra: Partial<DocumentInfo> = {}): DocumentInfo {
  return { documentId, width: 4000, height: 3000, layers, canUndo: revision > 0, canRedo: false, revision, ...extra };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

function setup() {
  let revision = 0;
  let nextDoc = 1;
  const api = {
    open: vi.fn(async (_photo: number, _space: unknown) => info(nextDoc++, 0)),
    render: vi.fn(async () => new ArrayBuffer(0)),
    edit: vi.fn(async (documentId: number, _edit: EditDto, _gesture: number | null) => info(documentId, ++revision)),
    undo: vi.fn(async (documentId: number) => info(documentId, ++revision)),
    redo: vi.fn(async (documentId: number) => info(documentId, ++revision)),
    close: vi.fn(async (_documentId: number) => undefined),
  } satisfies EditorApi;
  const toasts = new ToastStore();
  const modules = { active: 'editor' as 'library' | 'develop' | 'editor', switchTo: vi.fn() };
  const library = { focusIndex: null as number | null, photoAt: () => undefined, selectedIds: [] as number[] };
  let clock = 0;
  const store = new EditorStore({ library, modules: modules as never, toasts, api, now: () => clock });
  return { api, store, toasts, modules, library, advance: (ms: number) => (clock += ms) };
}

describe('EditorStore documents', () => {
  it('opens a document, selects the top layer, and fits the view', async () => {
    const { store, api } = setup();
    store.setViewport({ width: 800, height: 600 });
    await store.openPhoto(5);
    expect(api.open).toHaveBeenCalledWith(5, null);
    expect(store.status).toBe('ready');
    expect(store.selectedId).toBe(2);
    expect(store.view?.zoom).toBeLessThan(1);
    expect(store.zoomPercent).not.toBeNull();
  });

  it('closes the old document before it opens another', async () => {
    const { store, api } = setup();
    await store.openPhoto(5);
    await store.openPhoto(6);
    expect(api.close).toHaveBeenCalledWith(1);
    expect(store.info?.documentId).toBe(2);
    expect(store.photoId).toBe(6);
  });

  it('closes a document that opens too late', async () => {
    const { store, api } = setup();
    let release!: () => void;
    api.open.mockImplementationOnce(
      () =>
        new Promise<DocumentInfo>((resolve) => {
          release = () => resolve(info(9, 0));
        }),
    );
    const first = store.openPhoto(1);
    await store.openPhoto(2);
    release();
    await first;
    expect(api.close).toHaveBeenCalledWith(9);
    expect(store.info?.documentId).toBe(1);
  });

  it('shows an open error in the status and a toast', async () => {
    const { store, api, toasts } = setup();
    api.open.mockRejectedValueOnce({ kind: 'unavailable', message: 'no gpu' });
    await store.openPhoto(1);
    expect(store.status).toBe('error');
    expect(store.loadError).toContain('no gpu');
    expect(toasts.items[0]?.message).toContain('GPU');
  });

  it('opens the candidate only while the editor is active and empty', () => {
    const { store, library, modules } = setup();
    expect(store.openTarget).toBeNull();
    library.selectedIds = [3];
    expect(store.openTarget).toBe(3);
    modules.active = 'develop';
    expect(store.openTarget).toBeNull();
  });

  it('editInEditor switches the module and does not reopen the same photo', async () => {
    const { store, api, modules } = setup();
    await store.editInEditor(4);
    expect(modules.switchTo).toHaveBeenCalledWith('editor');
    await store.editInEditor(4);
    expect(api.open).toHaveBeenCalledTimes(1);
  });
});

describe('EditorStore edits', () => {
  it('sends edits in order and replaces the info', async () => {
    const { store, api } = setup();
    await store.openPhoto(1);
    store.setVisible(1, false);
    store.rename(1, '  Sky ');
    await tick();
    expect(api.edit.mock.calls.map((c) => c[1].type)).toEqual(['setVisible', 'rename']);
    expect(api.edit.mock.calls[1]?.[1]).toMatchObject({ name: 'Sky' });
    expect(store.info?.revision).toBe(2);
    expect(store.idle).toBe(true);
  });

  it('does not send an empty or unchanged name', async () => {
    const { store, api } = setup();
    await store.openPhoto(1);
    store.rename(1, '   ');
    store.rename(1, 'L1');
    await tick();
    expect(api.edit).not.toHaveBeenCalled();
  });

  it('keeps one live edit in flight and the newest waiting value', async () => {
    const { store, api } = setup();
    await store.openPhoto(1);
    let release!: () => void;
    api.edit.mockImplementationOnce(
      (documentId) =>
        new Promise<DocumentInfo>((resolve) => {
          release = () => resolve(info(documentId, 1));
        }),
    );
    const op = (opacity: number): EditDto => ({ type: 'setOpacity', layerId: 1, opacity });
    store.liveEdit('opacity:1', op(0.9));
    await tick();
    expect(api.edit).toHaveBeenCalledTimes(1);
    // The first edit is in flight. Newer values wait, and the newest wins.
    store.liveEdit('opacity:1', op(0.8));
    store.liveEdit('opacity:1', op(0.7));
    await tick();
    expect(api.edit).toHaveBeenCalledTimes(1);
    release();
    await tick();
    expect(api.edit).toHaveBeenCalledTimes(2);
    expect(api.edit.mock.calls[1]?.[1]).toEqual(op(0.7));
  });

  it('uses one gesture per drag and a new one after the drag ends', async () => {
    const { store, api, advance } = setup();
    await store.openPhoto(1);
    const op = (opacity: number): EditDto => ({ type: 'setOpacity', layerId: 1, opacity });
    store.liveEdit('opacity:1', op(0.9));
    await tick();
    advance(50);
    store.liveEdit('opacity:1', op(0.8));
    await tick();
    store.endGesture();
    store.liveEdit('opacity:1', op(0.7));
    await tick();
    advance(GESTURE_IDLE_MS + 1);
    store.liveEdit('opacity:1', op(0.6));
    await tick();
    const gestures = api.edit.mock.calls.map((c) => c[2]);
    expect(gestures[0]).toBe(gestures[1]);
    expect(gestures[2]).not.toBe(gestures[1]);
    expect(gestures[3]).not.toBe(gestures[2]);
    expect(gestures.every((g) => typeof g === 'number')).toBe(true);
  });

  it('sends a null gesture for a discrete edit', async () => {
    const { store, api } = setup();
    await store.openPhoto(1);
    store.setVisible(2, false);
    await tick();
    expect(api.edit.mock.calls[0]?.[2]).toBeNull();
  });

  it('selects the new layer after a duplicate and clears a deleted selection', async () => {
    const { store, api } = setup();
    await store.openPhoto(1);
    api.edit.mockResolvedValueOnce(info(1, 1, [layer(1), layer(2), layer(3)]));
    store.edit({ type: 'duplicate', layerId: 2 });
    await tick();
    expect(store.selectedId).toBe(3);
    api.edit.mockResolvedValueOnce(info(1, 2, [layer(1), layer(2)]));
    store.edit({ type: 'delete', layerId: 3 });
    await tick();
    expect(store.selectedId).toBeNull();
  });

  it('moves a layer up through the contract', async () => {
    const { store, api } = setup();
    await store.openPhoto(1);
    store.move(1, 'up');
    store.move(2, 'up');
    await tick();
    expect(api.edit).toHaveBeenCalledTimes(1);
    expect(api.edit.mock.calls[0]?.[1]).toEqual({ type: 'move', layerId: 1, parentId: null, index: 1 });
  });

  it('runs undo and redo and replaces the info', async () => {
    const { store, api } = setup();
    await store.openPhoto(1);
    store.undo();
    store.redo();
    await tick();
    expect(api.undo).toHaveBeenCalledWith(1);
    expect(api.redo).toHaveBeenCalledWith(1);
    expect(store.info?.revision).toBe(2);
  });

  it('shows a toast when an edit fails and continues with the next edit', async () => {
    const { store, api, toasts } = setup();
    await store.openPhoto(1);
    api.edit.mockRejectedValueOnce({ kind: 'invalidInput', message: 'bad' });
    store.setVisible(1, false);
    store.setVisible(2, false);
    await tick();
    expect(toasts.items).toHaveLength(1);
    expect(api.edit).toHaveBeenCalledTimes(2);
    expect(store.idle).toBe(true);
  });

  it('drops edits for a document that closed', async () => {
    const { store, api } = setup();
    await store.openPhoto(1);
    store.setVisible(1, false);
    await store.dispose();
    await tick();
    expect(store.info).toBeNull();
    expect(api.edit.mock.calls.length).toBeLessThanOrEqual(1);
  });
});

describe('EditorStore view', () => {
  it('refits on viewport change while in fit mode and keeps zoom otherwise', async () => {
    const { store } = setup();
    store.setViewport({ width: 800, height: 600 });
    await store.openPhoto(1);
    const fitted = store.view?.zoom ?? 0;
    store.setViewport({ width: 1600, height: 1200 });
    expect(store.view?.zoom ?? 0).toBeGreaterThan(fitted);
    store.zoomTo(1);
    expect(store.view?.zoom).toBe(1);
    expect(store.fit).toBe(false);
    store.setViewport({ width: 900, height: 700 });
    expect(store.view?.zoom).toBe(1);
    store.fitToWindow();
    expect(store.fit).toBe(true);
    expect(store.view?.zoom).toBeLessThan(1);
  });

  it('zooms about a point and pans', async () => {
    const { store } = setup();
    store.setViewport({ width: 800, height: 600 });
    await store.openPhoto(1);
    store.zoomTo(1);
    const before = store.view;
    store.zoomBy(2, { x: 100, y: 100 });
    expect(store.view?.zoom).toBe(2);
    store.pan(30, 0);
    expect(store.view?.originX).not.toBe(before?.originX);
    store.zoomStep(-1);
    expect(store.view?.zoom).toBeCloseTo(1.6);
  });

  it('ignores view commands without a document', () => {
    const { store } = setup();
    store.zoomStep(1);
    store.pan(1, 1);
    expect(store.view).toBeNull();
  });
});
