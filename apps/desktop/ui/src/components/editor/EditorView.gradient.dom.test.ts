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

describe('gradient tool', () => {
  async function ready(layers: LayerInfo[] = [layer(1)]) {
    const made = await makeStore(layers);
    render(EditorView, { editor: made.store, toasts: made.toasts });
    await flush();
    // happy-dom has no layout: set the view by hand (100 % zoom, origin 0).
    made.store.view = { zoom: 1, originX: 0, originY: 0, rotation: 0 };
    made.store.tool = 'gradient';
    await flush();
    return { ...made, viewport: screen.getByTestId('editor-viewport') };
  }

  const point = (x: number, y: number, extra: object = {}) => ({ clientX: x, clientY: y, button: 0, pointerId: 1, ...extra });
  const press = (store: EditorStore, id: string) => editorActions(store).find((a) => a.id === id)?.run();

  it('G cycles between the paint bucket and the gradient tool', async () => {
    const { store } = await makeStore([layer(1)]);
    expect(editorActions(store).find((a) => a.id === EDITOR_ACTION.paintBucketTool)?.defaultBinding).toBe('G');
    press(store, EDITOR_ACTION.paintBucketTool);
    expect(store.tool).toBe('paintBucket');
    press(store, EDITOR_ACTION.paintBucketTool);
    expect(store.tool).toBe('gradient');
    press(store, EDITOR_ACTION.paintBucketTool);
    expect(store.tool).toBe('paintBucket');
    store.setTool('gradient');
    store.setTool('hand');
    press(store, EDITOR_ACTION.paintBucketTool);
    expect(store.tool).toBe('gradient');
  });

  it('a drag sends one gradient edit with the contract fields', async () => {
    const { store, api, viewport } = await ready();
    store.foreground = '#ff0000';
    store.background = '#0000ff';
    store.gradient.shape = 'radial';
    store.gradient.opacity = 0.5;
    store.gradient.reverse = true;
    await fireEvent.pointerDown(viewport, point(10.5, 20.25));
    await fireEvent.pointerMove(viewport, point(40, 60));
    expect(screen.getByTestId('gradient-overlay')).toBeTruthy();
    await fireEvent.pointerUp(viewport, point(50.5, 60.25));
    await flush();
    expect(api.edit).toHaveBeenCalledTimes(1);
    expect(api.edit.mock.calls[0]?.[1]).toEqual({
      type: 'gradient',
      layerId: 1,
      target: 'pixels',
      start: [10.5, 20.25],
      end: [50.5, 60.25],
      shape: 'radial',
      stops: [
        { position: 0, color: [1, 0, 0], opacity: 1 },
        { position: 1, color: [0, 0, 1], opacity: 1 },
      ],
      opacity: 0.5,
      blendMode: 'normal',
      reverse: true,
    });
    expect(screen.queryByTestId('gradient-overlay')).toBeNull();
  });

  it('Shift snaps the end to 45 degree steps', async () => {
    const { api, viewport } = await ready();
    await fireEvent.pointerDown(viewport, point(10, 10));
    await fireEvent.pointerUp(viewport, point(50, 14, { shiftKey: true }));
    await flush();
    const edit = api.edit.mock.calls[0]?.[1];
    expect(edit?.type === 'gradient' ? edit.end[1] : null).toBeCloseTo(10, 6);
  });

  it('a short drag sends nothing', async () => {
    const { api, viewport } = await ready();
    await fireEvent.pointerDown(viewport, point(10, 10));
    await fireEvent.pointerUp(viewport, point(11, 10));
    await flush();
    expect(api.edit).not.toHaveBeenCalled();
  });

  it('Escape cancels the drag', async () => {
    const { api, viewport } = await ready();
    await fireEvent.pointerDown(viewport, point(10, 10));
    await fireEvent.pointerMove(viewport, point(40, 40));
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByTestId('gradient-overlay')).toBeNull();
    await fireEvent.pointerUp(viewport, point(60, 60));
    await flush();
    expect(api.edit).not.toHaveBeenCalled();
  });

  it('the mask target sends target mask and the normal mode', async () => {
    const { store, api, viewport } = await ready([layer(1, { hasMask: true, maskEnabled: true })]);
    store.gradient.blendMode = 'multiply';
    store.selectMaskTarget(1);
    await fireEvent.pointerDown(viewport, point(10, 10));
    await fireEvent.pointerUp(viewport, point(60, 10));
    await flush();
    expect(api.edit.mock.calls[0]?.[1]).toMatchObject({ type: 'gradient', target: 'mask', blendMode: 'normal' });
  });

  it('a pixel target on a non-pixel layer shows a toast and sends nothing', async () => {
    const { api, toasts, viewport } = await ready([layer(1, { kind: 'adjustment' })]);
    await fireEvent.pointerDown(viewport, point(10, 10));
    await fireEvent.pointerUp(viewport, point(60, 10));
    await flush();
    expect(api.edit).not.toHaveBeenCalled();
    expect(toasts.items.length).toBe(1);
  });
});
