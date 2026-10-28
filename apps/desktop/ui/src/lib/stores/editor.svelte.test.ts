import { describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, EditDto, EditorApi, LayerInfo, NewDocumentSpec, SaveResult } from '../editor/api';
import { strokeStubs } from '../editor/fakeStrokeApi';
import { defaultSpec } from '../editor/newDocument';
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
    maskEnabled: false,
    adjustment: null,
    isolated: null,
    children: null,
    locks: { transparency: false, pixels: false, position: false, all: false },
    ...extra,
  };
}

function info(documentId: number, revision: number, layers: LayerInfo[] = [layer(1), layer(2)], extra: Partial<DocumentInfo> = {}): DocumentInfo {
  return { documentId, width: 4000, height: 3000, layers, canUndo: revision > 0, canRedo: false, revision, dirty: false, savedPhotoId: null, sourcePhotoId: 1, hasSelection: false, selectionVersion: 0, ...extra };
}

function saveResult(documentId: number, revision: number, photoId = 77, created = true): SaveResult {
  return {
    photoId,
    documentPath: '/photos/a-edit.exdoc',
    imagePath: '/photos/a-edit.tif',
    created,
    info: info(documentId, revision, undefined, { dirty: false, savedPhotoId: photoId }),
  };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

function setup() {
  let revision = 0;
  let nextDoc = 1;
  const api = {
    open: vi.fn(async (_photo: number, _space: unknown) => info(nextDoc++, 0)),
    newDocument: vi.fn(async (_spec: NewDocumentSpec, _space: unknown) => info(nextDoc++, 0, [layer(1)], { sourcePhotoId: null })),
    render: vi.fn(async () => new ArrayBuffer(0)),
    edit: vi.fn(async (documentId: number, _edit: EditDto, _gesture: number | null) => info(documentId, ++revision)),
    undo: vi.fn(async (documentId: number) => info(documentId, ++revision)),
    redo: vi.fn(async (documentId: number) => info(documentId, ++revision)),
    save: vi.fn(async (documentId: number) => saveResult(documentId, revision)),
    close: vi.fn(async (_documentId: number) => undefined),
    selectionOutline: vi.fn(async (_documentId: number, _step: number) => ({ selectionVersion: 0, step: 1, contours: [] })),
    ...strokeStubs(),
  } satisfies EditorApi;
  const toasts = new ToastStore();
  const modules = { active: 'editor' as 'library' | 'develop' | 'editor', switchTo: vi.fn() };
  const library = { focusIndex: null as number | null, photoAt: () => undefined, selectedIds: [] as number[] };
  const onSaved = vi.fn((_result: SaveResult) => undefined);
  let clock = 0;
  const store = new EditorStore({ library, modules: modules as never, toasts, api, now: () => clock, onSaved });
  return { api, store, toasts, modules, library, onSaved, advance: (ms: number) => (clock += ms) };
}

/** Makes the next edits of the fake shell report a dirty document. */
function editsAreDirty(api: ReturnType<typeof setup>['api']): void {
  let revision = 100;
  api.edit.mockImplementation(async (documentId: number) => info(documentId, ++revision, undefined, { dirty: true }));
  api.save.mockImplementation(async (documentId: number) => saveResult(documentId, revision));
}

describe('EditorStore save', () => {
  it('waits for queued edits, saves, and takes the clean info', async () => {
    const { store, api, toasts, onSaved } = setup();
    await store.openPhoto(5);
    editsAreDirty(api);
    store.edit({ type: 'setVisible', layerId: 1, visible: false });
    expect(store.canSave).toBe(true);
    const saved = store.save();
    expect(store.saving).toBe(true);
    expect(store.canSave).toBe(false);
    expect(await saved).toBe(true);
    // The edit ran before the save.
    expect(api.edit.mock.invocationCallOrder[0]).toBeLessThan(api.save.mock.invocationCallOrder[0] ?? 0);
    expect(store.saving).toBe(false);
    expect(store.dirty).toBe(false);
    expect(store.info?.savedPhotoId).toBe(77);
    expect(onSaved).toHaveBeenCalledOnce();
    expect(toasts.items.at(-1)?.message).toBe('Saved to library: a-edit.tif');
  });

  it('a second save while one runs shares the run', async () => {
    const { store, api } = setup();
    await store.openPhoto(5);
    const a = store.save();
    const b = store.save();
    expect(await a).toBe(true);
    expect(await b).toBe(true);
    expect(api.save).toHaveBeenCalledOnce();
  });

  it('an edit after the save snapshot keeps the newer info and stays dirty', async () => {
    const { store, api } = setup();
    await store.openPhoto(5);
    editsAreDirty(api);
    let release!: () => void;
    api.save.mockImplementationOnce(
      (documentId: number) =>
        new Promise<SaveResult>((resolve) => {
          release = () => resolve(saveResult(documentId, 0));
        }),
    );
    const saved = store.save();
    await tick();
    store.edit({ type: 'setVisible', layerId: 1, visible: false });
    await tick();
    release();
    await saved;
    expect(store.info?.revision).toBe(101);
    expect(store.dirty).toBe(true);
    expect(store.info?.savedPhotoId).toBe(77);
  });

  it('a failed save shows an error and keeps the document dirty', async () => {
    const { store, api, toasts, onSaved } = setup();
    await store.openPhoto(5);
    editsAreDirty(api);
    store.edit({ type: 'setVisible', layerId: 1, visible: false });
    api.save.mockRejectedValueOnce({ kind: 'invalidInput', message: 'would overwrite' });
    expect(await store.save()).toBe(false);
    expect(store.dirty).toBe(true);
    expect(store.saving).toBe(false);
    expect(onSaved).not.toHaveBeenCalled();
    expect(toasts.items.at(-1)?.kind).toBe('error');
    expect(toasts.items.at(-1)?.message).toBe('Save the document failed.');
  });

  it('a failed plain save with an io error suggests Save As', async () => {
    const { store, api, toasts } = setup();
    await store.openPhoto(5);
    api.save.mockRejectedValueOnce({ kind: 'io', message: 'read-only folder' });
    expect(await store.save()).toBe(false);
    expect(toasts.items.at(-1)?.message).toContain('Save As');
  });

  it('Save As asks for a folder and passes it to the save', async () => {
    const { api, toasts, modules, library, onSaved } = setup();
    const pickFolder = vi.fn(async () => '/elsewhere');
    const store = new EditorStore({ library, modules: modules as never, toasts, api, onSaved, pickFolder });
    await store.openPhoto(5);
    expect(await store.saveAs()).toBe(true);
    expect(api.save).toHaveBeenCalledWith(1, '/elsewhere');
    expect(onSaved).toHaveBeenCalledOnce();
  });

  it('Save As does nothing when the folder dialog is cancelled', async () => {
    const { api, toasts, modules, library } = setup();
    const store = new EditorStore({ library, modules: modules as never, toasts, api, pickFolder: async () => null });
    await store.openPhoto(5);
    expect(await store.saveAs()).toBe(false);
    expect(api.save).not.toHaveBeenCalled();
  });

  it('a plain save passes no folder', async () => {
    const { store, api } = setup();
    await store.openPhoto(5);
    await store.save();
    expect(api.save).toHaveBeenCalledWith(1, undefined);
  });

  it('does nothing with no document', async () => {
    const { store, api } = setup();
    expect(await store.save()).toBe(false);
    expect(api.save).not.toHaveBeenCalled();
  });
});

describe('EditorStore unsaved changes', () => {
  async function dirtyStore() {
    const s = setup();
    await s.store.openPhoto(5);
    editsAreDirty(s.api);
    s.store.edit({ type: 'setVisible', layerId: 1, visible: false });
    await tick();
    return s;
  }

  it('a clean document opens another photo with no question', async () => {
    const { store, api } = setup();
    await store.openPhoto(5);
    await store.openPhoto(6);
    expect(store.leavePrompt).toBeNull();
    expect(api.open).toHaveBeenCalledTimes(2);
  });

  it('Cancel keeps the dirty document open', async () => {
    const { store, api } = await dirtyStore();
    const opening = store.openPhoto(6);
    await tick();
    expect(store.leavePrompt).not.toBeNull();
    store.answerLeave('cancel');
    await opening;
    expect(store.leavePrompt).toBeNull();
    expect(api.open).toHaveBeenCalledOnce();
    expect(api.close).not.toHaveBeenCalled();
    expect(store.photoId).toBe(5);
    expect(store.dirty).toBe(true);
  });

  it("Don't save opens the other photo and drops the changes", async () => {
    const { store, api } = await dirtyStore();
    const opening = store.openPhoto(6);
    await tick();
    store.answerLeave('discard');
    await opening;
    expect(api.save).not.toHaveBeenCalled();
    expect(api.close).toHaveBeenCalledWith(1);
    expect(store.photoId).toBe(6);
  });

  it('Save saves first, then opens the other photo', async () => {
    const { store, api } = await dirtyStore();
    const opening = store.openPhoto(6);
    await tick();
    store.answerLeave('save');
    await opening;
    expect(api.save).toHaveBeenCalledWith(1, undefined);
    expect(api.save.mock.invocationCallOrder[0]).toBeLessThan(api.close.mock.invocationCallOrder[0] ?? 0);
    expect(store.photoId).toBe(6);
  });

  it('a failed save on Save keeps the document open', async () => {
    const { store, api } = await dirtyStore();
    api.save.mockRejectedValueOnce({ kind: 'io', message: 'read-only' });
    const opening = store.openPhoto(6);
    await tick();
    store.answerLeave('save');
    await opening;
    expect(api.close).not.toHaveBeenCalled();
    expect(store.photoId).toBe(5);
  });

  it('the saved photo of the open document keeps the document', async () => {
    const { store, api } = setup();
    await store.openPhoto(5);
    await store.save();
    await store.editInEditor(77);
    expect(api.open).toHaveBeenCalledOnce();
    expect(store.leavePrompt).toBeNull();
  });
});

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

describe('EditorStore new document', () => {
  it('makes a new document, shows it, and counts the untitled name', async () => {
    const { store, api } = setup();
    expect(store.suggestedName).toBe('Untitled-1');
    expect(await store.newDocument(defaultSpec('Untitled-1'))).toBe(true);
    expect(api.newDocument).toHaveBeenCalledWith(defaultSpec('Untitled-1'), null);
    expect(store.status).toBe('ready');
    expect(store.photoId).toBeNull();
    expect(store.suggestedName).toBe('Untitled-2');
  });

  it('asks about unsaved changes first and keeps the document on Cancel', async () => {
    const { store, api } = setup();
    await store.openPhoto(5);
    editsAreDirty(api);
    store.edit({ type: 'setVisible', layerId: 1, visible: false });
    const run = store.newDocument(defaultSpec('Untitled-1'));
    await tick();
    expect(store.leavePrompt).not.toBeNull();
    store.answerLeave('cancel');
    expect(await run).toBe(false);
    expect(api.newDocument).not.toHaveBeenCalled();
    expect(store.photoId).toBe(5);
  });

  it('shows an error and keeps the name when the shell refuses', async () => {
    const { store, api, toasts } = setup();
    api.newDocument.mockRejectedValueOnce({ kind: 'invalid', message: 'bad' });
    expect(await store.newDocument(defaultSpec('Untitled-1'))).toBe(false);
    expect(store.status).toBe('error');
    expect(toasts.items.at(-1)?.message).toContain('new document');
    expect(store.suggestedName).toBe('Untitled-1');
  });

  it('requestNewDocument shows the Editor and opens the dialog', () => {
    const { store, modules } = setup();
    store.requestNewDocument();
    expect(modules.switchTo).toHaveBeenCalledWith('editor');
    expect(store.newDocumentOpen).toBe(true);
    store.closeNewDocument();
    expect(store.newDocumentOpen).toBe(false);
  });

  it('Save of a new document asks for a folder (Save As)', async () => {
    const { api, toasts, modules, library } = setup();
    const pickFolder = vi.fn(async () => '/docs');
    const store = new EditorStore({ library, modules: modules as never, toasts, api, pickFolder });
    await store.newDocument(defaultSpec('Untitled-1'));
    expect(await store.save()).toBe(true);
    expect(pickFolder).toHaveBeenCalledOnce();
    expect(api.save).toHaveBeenCalledWith(1, '/docs');
  });

  it('Save of a photo document does not ask for a folder', async () => {
    const { api, toasts, modules, library } = setup();
    const pickFolder = vi.fn(async () => '/docs');
    const store = new EditorStore({ library, modules: modules as never, toasts, api, pickFolder });
    await store.openPhoto(5);
    await store.save();
    expect(pickFolder).not.toHaveBeenCalled();
  });
});

describe('EditorStore layers', () => {
  it('addPixelLayer sends the edit above the selection and selects the new layer', async () => {
    const { store, api } = setup();
    await store.openPhoto(5);
    store.select(1);
    api.edit.mockImplementationOnce(async (documentId: number) => info(documentId, 1, [layer(1), layer(3), layer(2)]));
    store.addPixelLayer();
    await tick();
    expect(api.edit).toHaveBeenCalledWith(1, { type: 'addPixelLayer', aboveLayerId: 1 }, null);
    expect(store.selectedId).toBe(3);
  });

  it('selectedLocks combines the locks of the ancestors', async () => {
    const { store, api } = setup();
    const all = { transparency: false, pixels: false, position: false, all: true };
    api.open.mockImplementationOnce(async () => info(1, 0, [layer(7, { kind: 'group', isolated: true, locks: all, children: [layer(8)] })]));
    await store.openPhoto(5);
    store.select(8);
    expect(store.selectedLocks.all).toBe(true);
    store.select(null);
    expect(store.selectedLocks.all).toBe(false);
  });
});
