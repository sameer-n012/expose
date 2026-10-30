// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, EditDto, EditorApi, LayerBounds, LayerInfo, RenderArea } from '../../lib/editor/api';
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
const BOUNDS: LayerBounds = { x: 10, y: 10, width: 50, height: 40 };

async function makeStore(layers: LayerInfo[], bounds: LayerBounds | null) {
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
    render: vi.fn(async (_id: number, _area: RenderArea) => new ArrayBuffer(0)),
    edit: vi.fn(async (_id: number, _edit: EditDto, _gesture: number | null) => doc()),
    undo: vi.fn(async () => doc()),
    redo: vi.fn(async () => doc()),
    historyJump: vi.fn(async () => doc()),
    save: vi.fn(),
    close: vi.fn(async () => undefined),
    selectionOutline: vi.fn(async () => ({ selectionVersion: 0, step: 1, contours: [] })),
    ...strokeStubs(),
    layerBounds: vi.fn(async (_id: number, _layer: number): Promise<LayerBounds | null> => bounds),
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

const point = (x: number, y: number, extra: object = {}) => ({ clientX: x, clientY: y, button: 0, pointerId: 1, ...extra });
const press = (store: EditorStore, id: string) => editorActions(store).find((a) => a.id === id)?.run();

async function ready(layers: LayerInfo[] = [layer(1)], bounds: LayerBounds | null = BOUNDS) {
  const made = await makeStore(layers, bounds);
  render(EditorView, { editor: made.store, toasts: made.toasts });
  await flush();
  // happy-dom has no layout: set the viewport and the view by hand (100 % zoom, origin 0).
  made.store.setViewport({ width: 800, height: 600 });
  made.store.view = { zoom: 1, originX: 0, originY: 0, rotation: 0 };
  await flush();
  return { ...made, viewport: screen.getByTestId('editor-viewport') };
}

function lastPreview(api: { render: { mock: { calls: unknown[][] } } }) {
  const area = api.render.mock.calls[api.render.mock.calls.length - 1]?.[1] as RenderArea | undefined;
  return area?.transformPreview ?? null;
}

function expectMatrix(actual: readonly number[] | undefined, expected: number[]): void {
  expect(actual?.length).toBe(6);
  expected.forEach((value, i) => expect(actual?.[i]).toBeCloseTo(value, 9));
}

describe('free transform', () => {
  it('Mod+T is bound and enters the mode after layerBounds', async () => {
    const { store, api } = await ready();
    expect(editorActions(store).find((a) => a.id === EDITOR_ACTION.freeTransform)?.defaultBinding).toBe('Mod+T');
    press(store, EDITOR_ACTION.freeTransform);
    await flush();
    expect(api.layerBounds).toHaveBeenCalledWith(1, 1);
    expect(store.transform?.layerId).toBe(1);
    expect(screen.getByTestId('transform-overlay')).toBeTruthy();
  });

  it('null bounds show a toast and stay out of the mode', async () => {
    const { store, toasts } = await ready([layer(1)], null);
    press(store, EDITOR_ACTION.freeTransform);
    await flush();
    expect(store.transform).toBeNull();
    expect(toasts.items.some((t) => t.message === 'The layer is empty.')).toBe(true);
  });

  it('a layer that is not a pixel layer shows a toast without a call', async () => {
    const { store, api, toasts } = await ready([layer(1, { kind: 'adjustment' })]);
    press(store, EDITOR_ACTION.freeTransform);
    await flush();
    expect(api.layerBounds).not.toHaveBeenCalled();
    expect(store.transform).toBeNull();
    expect(toasts.items.some((t) => t.message.includes('pixel layer'))).toBe(true);
  });

  it('a drag inside the box asks for a frame with a translate matrix', async () => {
    const { store, api, viewport } = await ready();
    press(store, EDITOR_ACTION.freeTransform);
    await flush();
    await fireEvent.pointerDown(viewport, point(20, 20));
    await fireEvent.pointerMove(viewport, point(50, 25));
    await flush();
    expect(lastPreview(api)?.layerId).toBe(1);
    expectMatrix(lastPreview(api)?.matrix, [1, 0, 0, 1, 30, 5]);
    await fireEvent.pointerUp(viewport, point(50, 25));
    expect(api.edit).not.toHaveBeenCalled();
  });

  it('Enter sends transformLayer with the matrix and leaves the mode', async () => {
    const { store, api, viewport } = await ready();
    press(store, EDITOR_ACTION.freeTransform);
    await flush();
    await fireEvent.pointerDown(viewport, point(20, 20));
    await fireEvent.pointerMove(viewport, point(50, 25));
    await fireEvent.pointerUp(viewport, point(50, 25));
    await fireEvent.keyDown(window, { key: 'Enter' });
    await flush();
    expect(api.edit).toHaveBeenCalledTimes(1);
    const edit = api.edit.mock.calls[0]?.[1];
    expect(edit?.type).toBe('transformLayer');
    if (edit?.type === 'transformLayer') {
      expect(edit.layerId).toBe(1);
      expect(edit.interpolation).toBe('bicubic');
      expectMatrix(edit.matrix, [1, 0, 0, 1, 30, 5]);
    }
    expect(store.transform).toBeNull();
    await flush();
    expect(lastPreview(api)).toBeNull();
  });

  it('a failed commit shows a toast and stays in the mode', async () => {
    const { store, api, toasts } = await ready();
    api.edit.mockRejectedValueOnce({ kind: 'invalidInput', message: 'bad' });
    press(store, EDITOR_ACTION.freeTransform);
    await flush();
    await fireEvent.keyDown(window, { key: 'ArrowRight' });
    await fireEvent.keyDown(window, { key: 'Enter' });
    await flush();
    expect(store.transform).not.toBeNull();
    expect(store.transformBusy).toBe(false);
    expect(toasts.items.some((t) => t.message.startsWith('Edit the document failed'))).toBe(true);
  });

  it('an identity commit leaves the mode without an edit', async () => {
    const { store, api } = await ready();
    press(store, EDITOR_ACTION.freeTransform);
    await flush();
    await fireEvent.keyDown(window, { key: 'Enter' });
    await flush();
    expect(store.transform).toBeNull();
    expect(api.edit).not.toHaveBeenCalled();
  });

  it('Escape cancels without an edit and clears the preview', async () => {
    const { store, api, viewport } = await ready();
    press(store, EDITOR_ACTION.freeTransform);
    await flush();
    await fireEvent.pointerDown(viewport, point(20, 20));
    await fireEvent.pointerMove(viewport, point(50, 25));
    await fireEvent.pointerUp(viewport, point(50, 25));
    await flush();
    expect(lastPreview(api)).not.toBeNull();
    await fireEvent.keyDown(window, { key: 'Escape' });
    await flush();
    expect(store.transform).toBeNull();
    expect(api.edit).not.toHaveBeenCalled();
    expect(lastPreview(api)).toBeNull();
  });

  it('Mod+Z cancels the mode and sends no undo', async () => {
    const { store, api } = await ready();
    press(store, EDITOR_ACTION.freeTransform);
    await flush();
    press(store, EDITOR_ACTION.undo);
    await flush();
    expect(store.transform).toBeNull();
    expect(api.undo).not.toHaveBeenCalled();
  });

  it('arrow keys move by 1 px and Shift+arrow by 10 px', async () => {
    const { store, api } = await ready();
    press(store, EDITOR_ACTION.freeTransform);
    await flush();
    await fireEvent.keyDown(window, { key: 'ArrowRight' });
    await fireEvent.keyDown(window, { key: 'ArrowDown', shiftKey: true });
    await flush();
    expectMatrix(lastPreview(api)?.matrix, [1, 0, 0, 1, 1, 10]);
  });

  it('a tool change or another layer cancels the mode', async () => {
    const { store } = await ready([layer(1), layer(2)]);
    press(store, EDITOR_ACTION.freeTransform);
    await flush();
    store.setTool('hand');
    expect(store.transform).toBeNull();
    press(store, EDITOR_ACTION.freeTransform);
    await flush();
    expect(store.transform).not.toBeNull();
    store.select(1);
    expect(store.transform).toBeNull();
  });

  it('the options bar shows the fields and the Cancel button leaves the mode', async () => {
    const { store } = await ready();
    press(store, EDITOR_ACTION.freeTransform);
    await flush();
    expect(screen.getByTestId('transform-options')).toBeTruthy();
    const width = screen.getByLabelText('W %') as HTMLInputElement;
    expect(width.value).toBe('100');
    await fireEvent.click(screen.getByText('Cancel'));
    expect(store.transform).toBeNull();
    expect(screen.queryByTestId('transform-options')).toBeNull();
  });
});
