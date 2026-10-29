// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, EditDto, EditorApi, LayerInfo } from '../../lib/editor/api';
import { strokeStubs } from '../../lib/editor/fakeStrokeApi';
import { EDITOR_ACTION, editorActions } from '../../lib/editor/editorActions';
import { EditorStore } from '../../lib/stores/editor.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import EditorView from './EditorView.svelte';

const NONE = { transparency: false, pixels: false, position: false, all: false };

function layer(id: number, extra: Partial<LayerInfo> = {}): LayerInfo {
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
    ...extra,
  };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

async function makeStore(layers: LayerInfo[]) {
  const doc = (): DocumentInfo => ({
    documentId: 1,
    name: 'doc',
    width: 100,
    height: 80,
    layers,
    canUndo: false,
    canRedo: false,
    history: { steps: [], cursor: 0 },
    revision: 0,
    dirty: false,
    savedPhotoId: null,
    sourcePhotoId: 1,
    hasSelection: false,
    selectionVersion: 0,
    canReselect: false,
  });
  const api = {
    open: vi.fn(async () => doc()),
    newDocument: vi.fn(async () => doc()),
    render: vi.fn(async () => new ArrayBuffer(0)),
    edit: vi.fn(async (_id: number, _edit: EditDto, _gesture: number | null) => doc()),
    undo: vi.fn(async () => doc()),
    redo: vi.fn(async () => doc()),
    historyJump: vi.fn(async () => doc()),
    save: vi.fn(),
    close: vi.fn(async () => undefined),
    selectionOutline: vi.fn(async () => ({ selectionVersion: 0, step: 1, contours: [] })),
    ...strokeStubs(),
  } satisfies EditorApi;
  const toasts = new ToastStore();
  const store = new EditorStore({
    library: { focusIndex: null, photoAt: () => undefined, selectedIds: [] },
    modules: { active: 'editor', switchTo: vi.fn() } as never,
    toasts,
    api,
  });
  await store.openPhoto(1);
  return { store, api, toasts };
}

afterEach(cleanup);

describe('flood tools', () => {
  async function ready(layers: LayerInfo[] = [layer(1)]) {
    const made = await makeStore(layers);
    render(EditorView, { editor: made.store, toasts: made.toasts });
    await flush();
    // happy-dom has no layout: set the view by hand (100 % zoom, origin 0).
    made.store.view = { zoom: 1, originX: 0, originY: 0, rotation: 0 };
    await flush();
    return { ...made, viewport: screen.getByTestId('editor-viewport') };
  }

  const point = (x: number, y: number, extra: object = {}) => ({ clientX: x, clientY: y, button: 0, pointerId: 1, ...extra });
  const press = (store: EditorStore, id: string) => editorActions(store).find((a) => a.id === id)?.run();

  it('binds W and G', async () => {
    const { store } = await makeStore([layer(1)]);
    const keys = (id: string) => editorActions(store).find((a) => a.id === id)?.defaultBinding;
    expect(keys(EDITOR_ACTION.magicWandTool)).toBe('W');
    expect(keys(EDITOR_ACTION.paintBucketTool)).toBe('G');
  });

  it('W selects the wand; a click sends magicWand, Shift adds', async () => {
    const { store, api, viewport } = await ready();
    press(store, EDITOR_ACTION.magicWandTool);
    expect(store.tool).toBe('magicWand');
    await fireEvent.pointerDown(viewport, point(10.7, 20.2));
    await flush();
    expect(api.edit.mock.calls[0]?.[1]).toEqual({
      type: 'magicWand',
      x: 10,
      y: 20,
      mode: 'replace',
      tolerance: 32,
      contiguous: true,
      antiAlias: true,
      sampleAllLayers: false,
      layerId: 1,
    });
    await fireEvent.pointerDown(viewport, point(5, 5, { shiftKey: true }));
    await flush();
    expect(api.edit.mock.calls[1]?.[1]).toMatchObject({ type: 'magicWand', mode: 'add' });
  });

  it('the wand without a layer and without sampling all layers shows a toast', async () => {
    const { store, api, toasts, viewport } = await ready();
    store.tool = 'magicWand';
    store.select(null);
    await fireEvent.pointerDown(viewport, point(10, 10));
    await flush();
    expect(api.edit).not.toHaveBeenCalled();
    expect(toasts.items.length).toBe(1);
    store.wand.sampleAllLayers = true;
    await fireEvent.pointerDown(viewport, point(10, 10));
    await flush();
    expect(api.edit.mock.calls[0]?.[1]).toMatchObject({ type: 'magicWand', layerId: null, sampleAllLayers: true });
  });

  it('G and a click send fill with the foreground color', async () => {
    const { store, api, viewport } = await ready();
    store.foreground = '#ff8000';
    store.bucket.opacity = 0.5;
    press(store, EDITOR_ACTION.paintBucketTool);
    expect(store.tool).toBe('paintBucket');
    await fireEvent.pointerDown(viewport, point(30, 40));
    await flush();
    expect(api.edit.mock.calls[0]?.[1]).toEqual({
      type: 'fill',
      layerId: 1,
      x: 30,
      y: 40,
      color: [1, 128 / 255, 0],
      opacity: 0.5,
      blendMode: 'normal',
      tolerance: 32,
      contiguous: true,
      antiAlias: true,
      sampleAllLayers: false,
    });
  });

  it('the bucket on a mask target shows a toast and sends nothing', async () => {
    const { store, api, toasts, viewport } = await ready([layer(1, { hasMask: true, maskEnabled: true })]);
    store.selectMaskTarget(1);
    store.tool = 'paintBucket';
    await fireEvent.pointerDown(viewport, point(10, 10));
    await flush();
    expect(api.edit).not.toHaveBeenCalled();
    expect(toasts.items.length).toBe(1);
  });

  it('the bucket on an adjustment layer shows a toast and sends nothing', async () => {
    const { store, api, toasts, viewport } = await ready([layer(1, { kind: 'adjustment' })]);
    store.tool = 'paintBucket';
    await fireEvent.pointerDown(viewport, point(10, 10));
    await flush();
    expect(api.edit).not.toHaveBeenCalled();
    expect(toasts.items.length).toBe(1);
  });

  it('a click outside the canvas sends nothing', async () => {
    const { store, api, viewport } = await ready();
    for (const tool of ['magicWand', 'paintBucket'] as const) {
      store.tool = tool;
      await fireEvent.pointerDown(viewport, point(150, 10));
      await fireEvent.pointerDown(viewport, point(-1, 10));
      await fireEvent.pointerDown(viewport, point(10, 80));
    }
    await flush();
    expect(api.edit).not.toHaveBeenCalled();
  });
});
