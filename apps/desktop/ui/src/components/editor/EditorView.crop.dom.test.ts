// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, EditDto, EditorApi, LayerInfo, RenderArea } from '../../lib/editor/api';
import { strokeStubs } from '../../lib/editor/fakeStrokeApi';
import { EDITOR_ACTION, editorActions } from '../../lib/editor/editorActions';
import { EditorStore } from '../../lib/stores/editor.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import EditorView from './EditorView.svelte';

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

const flush = () => new Promise((r) => setTimeout(r, 0));

async function makeStore() {
  let size = { width: 100, height: 80 };
  const doc = (): DocumentInfo => ({
    documentId: 1,
    name: 'doc',
    ...size,
    layers: [layer(1), layer(2)],
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
    edit: vi.fn(async (_id: number, edit: EditDto, _gesture: number | null) => {
      if (edit.type === 'cropCanvas') {
        size = { width: edit.width, height: edit.height };
      }
      return doc();
    }),
    undo: vi.fn(async () => doc()),
    redo: vi.fn(async () => doc()),
    historyJump: vi.fn(async () => doc()),
    save: vi.fn(),
    close: vi.fn(async () => undefined),
    selectionOutline: vi.fn(async () => ({ selectionVersion: 0, step: 1, contours: [] })),
    ...strokeStubs(),
    layerBounds: vi.fn(async () => null),
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

async function ready() {
  const made = await makeStore();
  render(EditorView, { editor: made.store, toasts: made.toasts });
  await flush();
  // happy-dom has no layout: set the viewport and the view by hand (100 % zoom, origin 0).
  made.store.setViewport({ width: 800, height: 600 });
  made.store.view = { zoom: 1, originX: 0, originY: 0, rotation: 0 };
  await flush();
  return { ...made, viewport: screen.getByTestId('editor-viewport') };
}

/** Drags the right handle of the full box (100 x 80) to x = 60. */
async function dragRightHandle(viewport: HTMLElement) {
  await fireEvent.pointerDown(viewport, point(100, 40));
  await fireEvent.pointerMove(viewport, point(60, 40));
  await fireEvent.pointerUp(viewport, point(60, 40));
}

describe('crop tool', () => {
  it('C is bound and starts the mode with the full canvas', async () => {
    const { store } = await ready();
    expect(editorActions(store).find((a) => a.id === EDITOR_ACTION.cropTool)?.defaultBinding).toBe('C');
    press(store, EDITOR_ACTION.cropTool);
    await flush();
    expect(store.tool).toBe('crop');
    expect(store.crop?.box).toEqual({ centerX: 50, centerY: 40, width: 100, height: 80, angle: 0 });
    expect(screen.getByTestId('crop-overlay')).toBeTruthy();
    expect(screen.getByTestId('crop-options')).toBeTruthy();
  });

  it('a handle drag and Enter send cropCanvas, and the view fits the new canvas', async () => {
    const { store, api, viewport } = await ready();
    press(store, EDITOR_ACTION.cropTool);
    await flush();
    await dragRightHandle(viewport);
    expect(store.crop?.box.width).toBe(60);
    expect(api.edit).not.toHaveBeenCalled();
    await fireEvent.keyDown(window, { key: 'Enter' });
    await flush();
    expect(api.edit).toHaveBeenCalledTimes(1);
    expect(api.edit.mock.calls[0]?.[1]).toEqual({
      type: 'cropCanvas',
      centerX: 30,
      centerY: 40,
      width: 60,
      height: 80,
      angle: 0,
      deleteCropped: true,
      interpolation: 'bicubic',
    });
    expect(store.info?.width).toBe(60);
    // The box is the new full canvas and the view fits it (zoom 1 is the cap of the fit zoom).
    expect(store.crop?.box).toEqual({ centerX: 30, centerY: 40, width: 60, height: 80, angle: 0 });
    expect(store.fit).toBe(true);
    expect(store.view?.originX).toBeCloseTo(30 - 400, 6);
  });

  it('the options change the edit', async () => {
    const { store, api, viewport } = await ready();
    press(store, EDITOR_ACTION.cropTool);
    await flush();
    await dragRightHandle(viewport);
    await fireEvent.click(screen.getByLabelText('Delete cropped pixels'));
    await fireEvent.change(screen.getByLabelText('Crop angle'), { target: { value: '2.5' } });
    await fireEvent.click(screen.getByText('Commit'));
    await flush();
    const edit = api.edit.mock.calls[0]?.[1];
    expect(edit?.type).toBe('cropCanvas');
    if (edit?.type === 'cropCanvas') {
      expect(edit.deleteCropped).toBe(false);
      expect(edit.angle).toBe(2.5);
    }
  });

  it('a drag outside of the box rotates it', async () => {
    const { store, viewport } = await ready();
    press(store, EDITOR_ACTION.cropTool);
    await flush();
    // Grab right of the box at the height of its center, move down: the box turns clockwise.
    await fireEvent.pointerDown(viewport, point(150, 40));
    await fireEvent.pointerMove(viewport, point(150, 60));
    await fireEvent.pointerUp(viewport, point(150, 60));
    expect(store.crop?.box.angle).toBeGreaterThan(0);
    expect(store.crop?.box.angle).toBeLessThanOrEqual(45);
  });

  it('a locked aspect keeps the ratio while a handle moves', async () => {
    const { store, viewport } = await ready();
    press(store, EDITOR_ACTION.cropTool);
    await flush();
    store.setCropAspect('1:1');
    expect(store.crop?.box.width).toBe(80);
    expect(store.crop?.box.height).toBe(80);
    const right = (store.crop?.box.centerX ?? 0) + 40;
    await fireEvent.pointerDown(viewport, point(right, 40));
    await fireEvent.pointerMove(viewport, point(right - 30, 40));
    await fireEvent.pointerUp(viewport, point(right - 30, 40));
    expect(store.crop?.box.width).toBe(50);
    expect(store.crop?.box.height).toBe(50);
  });

  it('Escape resets a changed box, then leaves the tool without an edit', async () => {
    const { store, api, viewport } = await ready();
    press(store, EDITOR_ACTION.cropTool);
    await flush();
    await dragRightHandle(viewport);
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(store.crop?.box.width).toBe(100);
    expect(store.tool).toBe('crop');
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(store.crop).toBeNull();
    expect(store.tool).toBe('hand');
    expect(api.edit).not.toHaveBeenCalled();
  });

  it('Enter with the whole canvas sends nothing', async () => {
    const { store, api } = await ready();
    press(store, EDITOR_ACTION.cropTool);
    await flush();
    await fireEvent.keyDown(window, { key: 'Enter' });
    await flush();
    expect(api.edit).not.toHaveBeenCalled();
    expect(store.crop).not.toBeNull();
  });

  it('Mod+Z only resets the box', async () => {
    const { store, api, viewport } = await ready();
    press(store, EDITOR_ACTION.cropTool);
    await flush();
    await dragRightHandle(viewport);
    press(store, EDITOR_ACTION.undo);
    await flush();
    expect(api.undo).not.toHaveBeenCalled();
    expect(store.crop?.box.width).toBe(100);
  });

  it('a tool change, another layer, or another tab ends the mode', async () => {
    const { store } = await ready();
    press(store, EDITOR_ACTION.cropTool);
    await flush();
    store.setTool('hand');
    expect(store.crop).toBeNull();
    press(store, EDITOR_ACTION.cropTool);
    await flush();
    store.select(1);
    expect(store.crop).toBeNull();
    expect(store.tool).toBe('hand');
  });

  it('a failed edit shows a toast and keeps the box', async () => {
    const { store, api, toasts, viewport } = await ready();
    api.edit.mockRejectedValueOnce({ kind: 'invalidInput', message: 'bad' });
    press(store, EDITOR_ACTION.cropTool);
    await flush();
    await dragRightHandle(viewport);
    await fireEvent.keyDown(window, { key: 'Enter' });
    await flush();
    expect(store.crop?.box.width).toBe(60);
    expect(store.cropBusy).toBe(false);
    expect(toasts.items.some((t) => t.message.startsWith('Edit the document failed'))).toBe(true);
  });
});
