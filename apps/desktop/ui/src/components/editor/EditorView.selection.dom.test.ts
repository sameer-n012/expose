// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, EditDto, EditorApi, LayerInfo, SelectionOutline } from '../../lib/editor/api';
import { strokeStubs } from '../../lib/editor/fakeStrokeApi';
import { EDITOR_ACTION, editorActions } from '../../lib/editor/editorActions';
import { EditorStore } from '../../lib/stores/editor.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import EditorView from './EditorView.svelte';
import FeatherDialog from './FeatherDialog.svelte';
import LayersPanel from './LayersPanel.svelte';
import SelectionOverlay from './SelectionOverlay.svelte';

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
    adjustment: null,
    isolated: null,
    children: null,
    locks: NONE,
    ...extra,
  };
}

interface Sel {
  has: boolean;
  version: number;
}

const flush = () => new Promise((r) => setTimeout(r, 0));

function outline(version: number, contours: number[][] = [[0, 0, 10, 0, 10, 10]]): SelectionOutline {
  return { selectionVersion: version, step: 1, contours };
}

async function makeStore(layers: LayerInfo[], start: Sel = { has: false, version: 0 }) {
  let revision = 0;
  const sel: Sel = { ...start };
  const doc = (): DocumentInfo => ({
    documentId: 1,
    name: 'doc',
    width: 100,
    height: 80,
    layers,
    canUndo: false,
    canRedo: false,
    revision,
    dirty: false,
    savedPhotoId: null,
    sourcePhotoId: 1,
    hasSelection: sel.has,
    selectionVersion: sel.version,
  });
  const api = {
    open: vi.fn(async () => doc()),
    newDocument: vi.fn(async () => doc()),
    render: vi.fn(async () => new ArrayBuffer(0)),
    edit: vi.fn(async (_id: number, edit: EditDto, _gesture: number | null) => {
      revision += 1;
      if (edit.type === 'selectAll' || edit.type === 'selectShape') {
        sel.has = true;
        sel.version += 1;
      } else if (edit.type === 'deselect') {
        sel.has = false;
        sel.version += 1;
      }
      return doc();
    }),
    undo: vi.fn(async () => doc()),
    redo: vi.fn(async () => doc()),
    save: vi.fn(),
    close: vi.fn(async () => undefined),
    selectionOutline: vi.fn(async (_id: number, _step: number) => outline(sel.version)),
    ...strokeStubs(),
  } satisfies EditorApi;
  const store = new EditorStore({
    library: { focusIndex: null, photoAt: () => undefined, selectedIds: [] },
    modules: { active: 'editor', switchTo: vi.fn() } as never,
    toasts: new ToastStore(),
    api,
  });
  await store.openPhoto(1);
  return { store, api, sel };
}

afterEach(cleanup);

describe('SelectionOverlay', () => {
  it('draws nothing without a selection and asks for nothing', async () => {
    const { store, api } = await makeStore([layer(1)]);
    store.view = { zoom: 1, originX: 0, originY: 0 };
    render(SelectionOverlay, { editor: store });
    await flush();
    expect(screen.queryByTestId('selection-outline')).toBeNull();
    expect(api.selectionOutline).not.toHaveBeenCalled();
  });

  it('fetches the outline when the selection version changes', async () => {
    const { store, api } = await makeStore([layer(1)]);
    store.view = { zoom: 1, originX: 0, originY: 0 };
    render(SelectionOverlay, { editor: store });
    store.selectAll();
    await flush();
    await flush();
    expect(api.selectionOutline).toHaveBeenCalledTimes(1);
    expect(api.selectionOutline).toHaveBeenLastCalledWith(1, 1);
    const svg = screen.getByTestId('selection-outline');
    expect(svg.querySelector('path')?.getAttribute('d')).toBe('M0 0 L10 0 L10 10 Z');
    // A pan or a zoom (same step) does not ask again.
    store.view = { zoom: 2, originX: 5, originY: 5 };
    await flush();
    expect(api.selectionOutline).toHaveBeenCalledTimes(1);
    // A new version asks again.
    store.selectAll();
    await flush();
    await flush();
    expect(api.selectionOutline).toHaveBeenCalledTimes(2);
  });

  it('asks again when the zoom step changes', async () => {
    const { store, api } = await makeStore([layer(1)], { has: true, version: 5 });
    store.view = { zoom: 1, originX: 0, originY: 0 };
    render(SelectionOverlay, { editor: store });
    await flush();
    store.view = { zoom: 0.25, originX: 0, originY: 0 };
    await flush();
    expect(api.selectionOutline.mock.calls.map((c) => c[1])).toEqual([1, 4]);
  });

  it('drops a late answer of an old request', async () => {
    const { store, api, sel } = await makeStore([layer(1)], { has: true, version: 1 });
    store.view = { zoom: 1, originX: 0, originY: 0 };
    let releaseFirst: (value: SelectionOutline) => void = () => undefined;
    api.selectionOutline.mockImplementationOnce(() => new Promise<SelectionOutline>((resolve) => (releaseFirst = resolve)));
    render(SelectionOverlay, { editor: store });
    await flush();
    // The selection changes while the first request runs.
    store.selectAll();
    await flush();
    await flush();
    expect(sel.version).toBe(2);
    expect(screen.getByTestId('selection-outline').querySelector('path')?.getAttribute('d')).toBe('M0 0 L10 0 L10 10 Z');
    releaseFirst(outline(1, [[0, 0, 99, 0, 99, 99]]));
    await flush();
    expect(screen.getByTestId('selection-outline').querySelector('path')?.getAttribute('d')).toBe('M0 0 L10 0 L10 10 Z');
  });

  it('clears the outline after a deselect', async () => {
    const { store } = await makeStore([layer(1)], { has: true, version: 1 });
    store.view = { zoom: 1, originX: 0, originY: 0 };
    render(SelectionOverlay, { editor: store });
    await flush();
    expect(screen.queryByTestId('selection-outline')).not.toBeNull();
    store.deselect();
    await flush();
    await flush();
    expect(screen.queryByTestId('selection-outline')).toBeNull();
  });
});

describe('selection shortcuts', () => {
  const find = (store: EditorStore, id: string) => editorActions(store).find((a) => a.id === id);

  it('binds the keys', async () => {
    const { store } = await makeStore([layer(1)]);
    const keys = (id: string) => find(store, id)?.defaultBinding;
    expect(keys(EDITOR_ACTION.selectAll)).toBe('Mod+A');
    expect(keys(EDITOR_ACTION.deselect)).toBe('Mod+D');
    expect(keys(EDITOR_ACTION.invertSelection)).toBe('Mod+Shift+I');
    expect(keys(EDITOR_ACTION.featherSelection)).toBe('Shift+F6');
    expect(keys(EDITOR_ACTION.marqueeTool)).toBe('M');
    expect(keys(EDITOR_ACTION.lassoTool)).toBe('L');
    expect(keys(EDITOR_ACTION.handTool)).toBe('H');
  });

  it('does nothing for deselect, invert, and feather without a selection', async () => {
    const { store, api } = await makeStore([layer(1)]);
    find(store, EDITOR_ACTION.deselect)?.run();
    find(store, EDITOR_ACTION.invertSelection)?.run();
    find(store, EDITOR_ACTION.featherSelection)?.run();
    await flush();
    expect(api.edit).not.toHaveBeenCalled();
    expect(store.featherDialogOpen).toBe(false);
  });

  it('sends the edits with a selection', async () => {
    const { store, api } = await makeStore([layer(1)], { has: true, version: 1 });
    find(store, EDITOR_ACTION.invertSelection)?.run();
    await flush();
    find(store, EDITOR_ACTION.deselect)?.run();
    await flush();
    expect(api.edit.mock.calls.map((c) => c[1].type)).toEqual(['invertSelection', 'deselect']);
    find(store, EDITOR_ACTION.selectAll)?.run();
    await flush();
    expect(api.edit.mock.calls[2]?.[1]).toEqual({ type: 'selectAll' });
  });

  it('M cycles the marquee tools, L and H set tools', async () => {
    const { store } = await makeStore([layer(1)]);
    find(store, EDITOR_ACTION.marqueeTool)?.run();
    expect(store.tool).toBe('rectSelect');
    find(store, EDITOR_ACTION.marqueeTool)?.run();
    expect(store.tool).toBe('ellipseSelect');
    find(store, EDITOR_ACTION.lassoTool)?.run();
    expect(store.tool).toBe('lasso');
    find(store, EDITOR_ACTION.lassoTool)?.run();
    expect(store.tool).toBe('polygonLasso');
    find(store, EDITOR_ACTION.handTool)?.run();
    expect(store.tool).toBe('hand');
    // L returns to the lasso kind that was used last.
    find(store, EDITOR_ACTION.lassoTool)?.run();
    expect(store.tool).toBe('polygonLasso');
  });
});

describe('Feather dialog', () => {
  it('sends the radius on OK and closes', async () => {
    const { store, api } = await makeStore([layer(1)], { has: true, version: 1 });
    store.openFeatherDialog();
    expect(store.featherDialogOpen).toBe(true);
    render(FeatherDialog, { editor: store });
    const input = screen.getByLabelText('Radius (px)');
    await fireEvent.input(input, { target: { value: '12' } });
    await fireEvent.click(screen.getByRole('button', { name: 'OK' }));
    await flush();
    expect(api.edit.mock.calls[0]?.[1]).toEqual({ type: 'featherSelection', radius: 12 });
    expect(store.featherDialogOpen).toBe(false);
  });

  it('refuses a radius above 250', async () => {
    const { store, api } = await makeStore([layer(1)], { has: true, version: 1 });
    render(FeatherDialog, { editor: store });
    await fireEvent.input(screen.getByLabelText('Radius (px)'), { target: { value: '251' } });
    expect(screen.getByRole<HTMLButtonElement>('button', { name: 'OK' }).disabled).toBe(true);
    await fireEvent.click(screen.getByRole('button', { name: 'OK' }));
    expect(api.edit).not.toHaveBeenCalled();
  });
});

describe('Add mask with a selection', () => {
  it('keeps the old behavior without a selection', async () => {
    const { store, api } = await makeStore([layer(1)]);
    render(LayersPanel, { editor: store });
    await fireEvent.click(screen.getByRole('button', { name: 'Add mask' }));
    await flush();
    expect(api.edit.mock.calls[0]?.[1]).toEqual({ type: 'addMask', layerId: 1, fill: 'reveal' });
  });

  it('click makes the mask from the selection, Alt+click adds a hide mask', async () => {
    const { store, api } = await makeStore([layer(1)], { has: true, version: 1 });
    render(LayersPanel, { editor: store });
    const button = screen.getByRole('button', { name: 'Add mask' });
    await fireEvent.click(button);
    await flush();
    expect(api.edit.mock.calls[0]?.[1]).toEqual({ type: 'maskFromSelection', layerId: 1 });
    await fireEvent.click(button, { altKey: true });
    await flush();
    expect(api.edit.mock.calls[1]?.[1]).toEqual({ type: 'addMask', layerId: 1, fill: 'hide' });
  });

  it('the Mask menu has Mask from Selection, also for a layer with a mask', async () => {
    const { store, api } = await makeStore([layer(1, { hasMask: true, maskEnabled: true })], { has: true, version: 1 });
    render(LayersPanel, { editor: store });
    await fireEvent.click(screen.getByRole('button', { name: /^Mask$/ }));
    await fireEvent.click(screen.getByRole('menuitem', { name: 'Mask from Selection' }));
    await flush();
    expect(api.edit.mock.calls[0]?.[1]).toEqual({ type: 'maskFromSelection', layerId: 1 });
  });

  it('the menu item is off without a selection or on a pixels-locked layer', async () => {
    const { store } = await makeStore([layer(1)]);
    render(LayersPanel, { editor: store });
    await fireEvent.click(screen.getByRole('button', { name: /^Mask$/ }));
    expect(screen.getByRole<HTMLButtonElement>('menuitem', { name: 'Mask from Selection' }).disabled).toBe(true);
    cleanup();
    const locked = await makeStore([layer(1, { locks: { ...NONE, pixels: true } })], { has: true, version: 1 });
    render(LayersPanel, { editor: locked.store });
    expect(screen.getByRole<HTMLButtonElement>('button', { name: 'Add mask' }).disabled).toBe(true);
  });
});

describe('EditorView gestures', () => {
  async function ready(tool: 'rectSelect' | 'ellipseSelect' | 'lasso' | 'polygonLasso', start?: Sel) {
    const made = await makeStore([layer(1)], start);
    render(EditorView, { editor: made.store, toasts: new ToastStore() });
    await flush();
    // happy-dom has no layout: set the view by hand (100 % zoom, origin 0).
    made.store.view = { zoom: 1, originX: 0, originY: 0 };
    made.store.setTool(tool);
    await flush();
    return { ...made, viewport: screen.getByTestId('editor-viewport') };
  }

  const point = (x: number, y: number, extra: object = {}) => ({ clientX: x, clientY: y, button: 0, pointerId: 1, ...extra });

  it('a rectangle drag sends selectShape with the options', async () => {
    const { store, api, viewport } = await ready('rectSelect');
    store.setFeather(7);
    await fireEvent.pointerDown(viewport, point(10, 20));
    await fireEvent.pointerMove(viewport, point(40, 60));
    expect(screen.queryByTestId('selection-preview')).not.toBeNull();
    await fireEvent.pointerUp(viewport, point(40, 60));
    await flush();
    expect(api.edit.mock.calls[0]?.[1]).toEqual({
      type: 'selectShape',
      shape: { type: 'rect', x: 10, y: 20, width: 30, height: 40 },
      mode: 'replace',
      antiAlias: true,
      feather: 7,
    });
  });

  it('modifier keys at pointer-down set the mode and a negative drag works', async () => {
    const { api, viewport } = await ready('ellipseSelect');
    await fireEvent.pointerDown(viewport, point(50, 50, { shiftKey: true, altKey: true }));
    await fireEvent.pointerUp(viewport, point(30, 20));
    await flush();
    expect(api.edit.mock.calls[0]?.[1]).toMatchObject({
      shape: { type: 'ellipse', x: 30, y: 20, width: 20, height: 30 },
      mode: 'intersect',
    });
  });

  it('a click deselects only when there is a selection', async () => {
    const none = await ready('rectSelect');
    await fireEvent.pointerDown(none.viewport, point(10, 10));
    await fireEvent.pointerUp(none.viewport, point(11, 10));
    await flush();
    expect(none.api.edit).not.toHaveBeenCalled();
    cleanup();
    const some = await ready('rectSelect', { has: true, version: 1 });
    await fireEvent.pointerDown(some.viewport, point(10, 10));
    await fireEvent.pointerUp(some.viewport, point(10, 10));
    await flush();
    expect(some.api.edit.mock.calls[0]?.[1]).toEqual({ type: 'deselect' });
  });

  it('a click with a modifier does nothing', async () => {
    const { api, viewport } = await ready('rectSelect', { has: true, version: 1 });
    await fireEvent.pointerDown(viewport, point(10, 10, { shiftKey: true }));
    await fireEvent.pointerUp(viewport, point(10, 10));
    await flush();
    expect(api.edit).not.toHaveBeenCalled();
  });

  it('Escape cancels a drag', async () => {
    const { api, viewport } = await ready('rectSelect');
    await fireEvent.pointerDown(viewport, point(10, 10));
    await fireEvent.pointerMove(viewport, point(50, 50));
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByTestId('selection-preview')).toBeNull();
    await fireEvent.pointerUp(viewport, point(50, 50));
    await flush();
    expect(api.edit).not.toHaveBeenCalled();
  });

  it('the hand tool does not select', async () => {
    const { api, store, viewport } = await ready('rectSelect');
    store.setTool('hand');
    await flush();
    await fireEvent.pointerDown(viewport, point(10, 10));
    await fireEvent.pointerUp(viewport, point(60, 60));
    await flush();
    expect(api.edit).not.toHaveBeenCalled();
  });

  it('a freehand lasso drag sends one polygon edit with the mode', async () => {
    const { api, viewport } = await ready('lasso');
    await fireEvent.pointerDown(viewport, point(10, 10, { shiftKey: true }));
    await fireEvent.pointerMove(viewport, point(60, 10));
    expect(screen.queryByTestId('selection-preview')).not.toBeNull();
    await fireEvent.pointerMove(viewport, point(60, 60));
    await fireEvent.pointerUp(viewport, point(60, 60));
    await flush();
    expect(api.edit).toHaveBeenCalledTimes(1);
    expect(api.edit.mock.calls[0]?.[1]).toEqual({
      type: 'selectShape',
      shape: {
        type: 'polygon',
        points: [
          [10, 10],
          [60, 10],
          [60, 60],
        ],
      },
      mode: 'add',
      antiAlias: true,
      feather: 0,
    });
  });

  it('a freehand click deselects, Escape cancels', async () => {
    const some = await ready('lasso', { has: true, version: 1 });
    await fireEvent.pointerDown(some.viewport, point(5, 5));
    await fireEvent.pointerUp(some.viewport, point(5, 5));
    await flush();
    expect(some.api.edit.mock.calls[0]?.[1]).toEqual({ type: 'deselect' });
    cleanup();
    const none = await ready('lasso');
    await fireEvent.pointerDown(none.viewport, point(10, 10));
    await fireEvent.pointerMove(none.viewport, point(60, 10));
    await fireEvent.keyDown(window, { key: 'Escape' });
    await fireEvent.pointerUp(none.viewport, point(60, 60));
    await flush();
    expect(none.api.edit).not.toHaveBeenCalled();
  });

  it('the lasso closes on a click near the first point', async () => {
    const { api, viewport } = await ready('polygonLasso');
    for (const [x, y] of [
      [10, 10],
      [60, 10],
      [60, 60],
    ] as const) {
      await fireEvent.pointerDown(viewport, point(x, y));
      await fireEvent.pointerUp(viewport, point(x, y));
    }
    expect(api.edit).not.toHaveBeenCalled();
    await fireEvent.pointerDown(viewport, point(13, 12));
    await flush();
    expect(api.edit.mock.calls[0]?.[1]).toEqual({
      type: 'selectShape',
      shape: {
        type: 'polygon',
        points: [
          [10, 10],
          [60, 10],
          [60, 60],
        ],
      },
      mode: 'replace',
      antiAlias: true,
      feather: 0,
    });
  });

  it('the lasso closes on Enter and Backspace removes the last point', async () => {
    const { api, viewport } = await ready('polygonLasso');
    for (const [x, y] of [
      [10, 10],
      [60, 10],
      [60, 60],
      [10, 60],
    ] as const) {
      await fireEvent.pointerDown(viewport, point(x, y, { shiftKey: true }));
    }
    await fireEvent.keyDown(window, { key: 'Backspace' });
    await fireEvent.keyDown(window, { key: 'Enter' });
    await flush();
    expect(api.edit.mock.calls[0]?.[1]).toMatchObject({
      mode: 'add',
      shape: { type: 'polygon', points: [[10, 10], [60, 10], [60, 60]] },
    });
  });

  it('the lasso closes on a double-click without a repeated point', async () => {
    const { api, viewport } = await ready('polygonLasso');
    for (const [x, y] of [
      [10, 10],
      [60, 10],
      [60, 60],
      [60, 60],
    ] as const) {
      await fireEvent.pointerDown(viewport, point(x, y));
    }
    await fireEvent.dblClick(viewport);
    await flush();
    expect(api.edit.mock.calls[0]?.[1]).toMatchObject({ shape: { type: 'polygon', points: [[10, 10], [60, 10], [60, 60]] } });
  });

  it('a lasso with fewer than 3 points sends nothing; Escape cancels', async () => {
    const { api, viewport } = await ready('polygonLasso');
    await fireEvent.pointerDown(viewport, point(10, 10));
    await fireEvent.pointerDown(viewport, point(60, 10));
    await fireEvent.keyDown(window, { key: 'Enter' });
    await fireEvent.pointerDown(viewport, point(10, 10));
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByTestId('selection-preview')).toBeNull();
    await flush();
    expect(api.edit).not.toHaveBeenCalled();
  });
});
