import { describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, EditorApi, LayerInfo } from '../editor/api';
import { strokeStubs } from '../editor/fakeStrokeApi';
import { EDITOR_ACTION, editorActions } from '../editor/editorActions';
import { canvasToScreen, rotatePoint, unrotatePoint } from '../editor/view';
import { EditorStore } from './editor.svelte';
import { ToastStore } from './toasts.svelte';

const NONE = { transparency: false, pixels: false, position: false, all: false };

function layer(id: number): LayerInfo {
  return {
    id,
    name: `Layer ${id}`,
    kind: 'pixel',
    visible: true,
    opacity: 1,
    fillOpacity: 1,
    blendMode: 'normal',
    hasMask: false,
    maskEnabled: false,
    maskVersion: null,
    adjustment: null,
    isolated: null,
    children: null,
    locks: NONE,
  };
}

function info(documentId: number): DocumentInfo {
  return {
    documentId,
    name: `doc${documentId}`,
    width: 4000,
    height: 3000,
    layers: [layer(1)],
    canUndo: false,
    canRedo: false,
    revision: 0,
    dirty: false,
    savedPhotoId: null,
    sourcePhotoId: 1,
    hasSelection: false,
    selectionVersion: 0,
    canReselect: false,
  };
}

async function setup() {
  let nextDoc = 1;
  const api = {
    open: vi.fn(async (_photo: number, _space: unknown) => info(nextDoc++)),
    newDocument: vi.fn(async () => info(nextDoc++)),
    render: vi.fn(async () => new ArrayBuffer(0)),
    edit: vi.fn(async (documentId: number) => info(documentId)),
    undo: vi.fn(async (documentId: number) => info(documentId)),
    redo: vi.fn(async (documentId: number) => info(documentId)),
    save: vi.fn(),
    close: vi.fn(async () => undefined),
    selectionOutline: vi.fn(async () => ({ selectionVersion: 0, step: 1, contours: [] })),
    ...strokeStubs(),
  } satisfies EditorApi;
  const store = new EditorStore({
    library: { focusIndex: null, photoAt: () => undefined, selectedIds: [] },
    modules: { active: 'editor', switchTo: vi.fn() } as never,
    toasts: new ToastStore(),
    api,
  });
  store.setViewport({ width: 800, height: 600 });
  await store.openPhoto(1);
  return { store, api };
}

describe('EditorStore view rotation', () => {
  it('sets and normalizes the rotation without an edit', async () => {
    const { store, api } = await setup();
    expect(store.rotation).toBe(0);
    store.setRotation(390);
    expect(store.rotation).toBe(30);
    expect(store.view?.rotation).toBe(30);
    expect(api.edit).not.toHaveBeenCalled();
    store.resetRotation();
    expect(store.rotation).toBe(0);
  });

  it('keeps the rotation through fit and 100 %', async () => {
    const { store } = await setup();
    store.setRotation(-45);
    store.fitToWindow();
    expect(store.rotation).toBe(-45);
    store.zoomTo(1);
    expect(store.rotation).toBe(-45);
    // A viewport change fits the view again and keeps the rotation.
    store.fitToWindow();
    store.setViewport({ width: 1000, height: 700 });
    expect(store.rotation).toBe(-45);
  });

  it('is per tab', async () => {
    const { store } = await setup();
    const first = store.activeTab;
    store.setRotation(90);
    await store.openPhoto(2);
    expect(store.tabs.length).toBe(2);
    expect(store.rotation).toBe(0);
    store.setRotation(15);
    if (first === null) throw new Error('no first tab');
    store.activate(first);
    expect(store.rotation).toBe(90);
  });

  it('pans so that the content follows the pointer', async () => {
    const { store } = await setup();
    store.zoomTo(1);
    store.setRotation(90);
    const viewport = store.viewport;
    const canvasPoint = { x: 2000, y: 1500 };
    const screenOf = () => {
      const view = store.view;
      if (view === null) throw new Error('no view');
      return rotatePoint(canvasToScreen(view, canvasPoint), viewport, view.rotation);
    };
    const before = screenOf();
    store.pan(30, -20);
    const after = screenOf();
    expect(after.x - before.x).toBeCloseTo(30);
    expect(after.y - before.y).toBeCloseTo(-20);
  });

  it('wheel zoom keeps the point under the pointer', async () => {
    const { store } = await setup();
    store.zoomTo(1);
    store.setRotation(30);
    const viewport = store.viewport;
    const pointer = { x: 120, y: 450 };
    const anchor = unrotatePoint(pointer, viewport, 30);
    const before = store.view;
    store.zoomBy(2, anchor);
    const after = store.view;
    if (before === null || after === null) throw new Error('no view');
    // The canvas point under the pointer is the same before and after.
    const under = (v: typeof before) => ({ x: anchor.x / v.zoom + v.originX, y: anchor.y / v.zoom + v.originY });
    expect(under(after).x).toBeCloseTo(under(before).x);
    expect(under(after).y).toBeCloseTo(under(before).y);
  });

  it('has the rotate tool key and a reset action without a key', async () => {
    const { store } = await setup();
    const actions = editorActions(store);
    const rotate = actions.find((a) => a.id === EDITOR_ACTION.rotateTool);
    const reset = actions.find((a) => a.id === EDITOR_ACTION.resetRotation);
    expect(rotate?.defaultBinding).toBe('R');
    expect(rotate?.scope).toBe('editor');
    expect(reset?.defaultBinding).toBeNull();
    rotate?.run();
    expect(store.tool).toBe('rotate');
    store.setRotation(60);
    reset?.run();
    expect(store.rotation).toBe(0);
  });
});
