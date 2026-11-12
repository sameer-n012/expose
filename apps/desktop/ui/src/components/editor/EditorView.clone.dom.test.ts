// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, EditorApi, LayerInfo, SampledColor } from '../../lib/editor/api';
import { EDITOR_ACTION, editorActions } from '../../lib/editor/editorActions';
import { thumbnailBuffer } from '../../lib/editor/fakeStrokeApi';
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
  let revision = 0;
  const doc = (): DocumentInfo => ({
    documentId: 1,
    name: 'doc',
    width: 100,
    height: 80,
    layers,
    canUndo: false,
    canRedo: false,
    history: { steps: [], cursor: 0 },
    revision,
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
    edit: vi.fn(async () => doc()),
    undo: vi.fn(async () => doc()),
    redo: vi.fn(async () => doc()),
    historyJump: vi.fn(async () => doc()),
    save: vi.fn(),
    close: vi.fn(async () => undefined),
    selectionOutline: vi.fn(async () => ({ selectionVersion: 0, step: 1, contours: [] })),
    maskThumbnail: vi.fn(async (..._args: unknown[]) => thumbnailBuffer(2, 2)),
    layerBounds: vi.fn(async (..._args: unknown[]) => null),
    sampleColor: vi.fn(async (..._args: unknown[]): Promise<SampledColor> => ({ color: '#336699', alpha: 1 })),
    strokeBegin: vi.fn(async (..._args: unknown[]) => 5),
    strokePoints: vi.fn(async (..._args: unknown[]) => null),
    strokeEnd: vi.fn(async (..._args: unknown[]) => {
      revision += 1;
      return doc();
    }),
    strokeCancel: vi.fn(async (..._args: unknown[]) => doc()),
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

const find = (store: EditorStore, id: string) => editorActions(store).find((a) => a.id === id);
const point = (x: number, y: number, extra: object = {}) => ({ clientX: x, clientY: y, button: 0, pointerId: 1, ...extra });

async function ready(tool: 'cloneStamp' | 'healingBrush', layers: LayerInfo[] = [layer(1)]) {
  const made = await makeStore(layers);
  render(EditorView, { editor: made.store, toasts: made.toasts });
  await flush();
  // happy-dom has no layout: set the view by hand (100 % zoom, origin 0).
  made.store.view = { zoom: 1, originX: 0, originY: 0, rotation: 0 };
  made.store.setTool(tool);
  await flush();
  return { ...made, viewport: screen.getByTestId('editor-viewport') };
}

async function drag(viewport: HTMLElement, from: [number, number], to: [number, number]) {
  await fireEvent.pointerDown(viewport, point(from[0], from[1]));
  await fireEvent.pointerMove(viewport, point(to[0], to[1]));
  await flush();
  await fireEvent.pointerUp(viewport, point(to[0], to[1]));
  await flush();
  await flush();
}

describe('clone shortcuts', () => {
  it('binds S and J and selects the tools', async () => {
    const { store } = await makeStore([layer(1)]);
    expect(find(store, EDITOR_ACTION.cloneStampTool)?.defaultBinding).toBe('S');
    expect(find(store, EDITOR_ACTION.healingBrushTool)?.defaultBinding).toBe('J');
    find(store, EDITOR_ACTION.cloneStampTool)?.run();
    expect(store.tool).toBe('cloneStamp');
    find(store, EDITOR_ACTION.healingBrushTool)?.run();
    expect(store.tool).toBe('healingBrush');
    find(store, EDITOR_ACTION.brushSizeUp)?.run();
    expect(store.clone.size).toBe(45);
    find(store, EDITOR_ACTION.brushHarder)?.run();
    expect(store.clone.hardness).toBe(0.75);
  });

  it('does not select the tools on a layer that is not a pixel layer', async () => {
    const { store } = await makeStore([layer(1, { kind: 'adjustment' })]);
    find(store, EDITOR_ACTION.cloneStampTool)?.run();
    expect(store.tool).toBe('hand');
  });
});

describe('EditorView clone strokes', () => {
  it('Alt+click sets the source point and starts no stroke', async () => {
    const { store, api, viewport } = await ready('cloneStamp');
    await fireEvent.pointerDown(viewport, point(60, 40, { altKey: true }));
    await fireEvent.pointerUp(viewport, point(60, 40, { altKey: true }));
    await flush();
    expect(api.strokeBegin).not.toHaveBeenCalled();
    expect(store.cloneState.source).toEqual({ x: 60, y: 40 });
    expect(store.cloneState.offset).toBeNull();
    expect(api.sampleColor).not.toHaveBeenCalled();
  });

  it('a stroke sends a clone target with the offset', async () => {
    const { api, viewport } = await ready('cloneStamp');
    await fireEvent.pointerDown(viewport, point(60, 40, { altKey: true }));
    await drag(viewport, [10, 20], [30, 40]);
    expect(api.strokeBegin).toHaveBeenCalledTimes(1);
    const [, layerId, target, brush] = api.strokeBegin.mock.calls[0] ?? [];
    expect(layerId).toBe(1);
    expect(target).toEqual({ type: 'clone', offset: [50, 20], sampleAllLayers: false });
    expect(brush).toMatchObject({ size: 40, blendMode: 'normal' });
    expect(api.strokeEnd).toHaveBeenCalledWith(1, 5);
  });

  it('the healing brush sends a heal target and the sample setting', async () => {
    const { store, api, viewport } = await ready('healingBrush');
    store.cloneSampleAll = true;
    await fireEvent.pointerDown(viewport, point(0, 0, { altKey: true }));
    await drag(viewport, [10, 10], [20, 20]);
    expect(api.strokeBegin.mock.calls[0]?.[2]).toEqual({ type: 'heal', offset: [-10, -10], sampleAllLayers: true });
  });

  it('shows a toast and starts no stroke without a source point', async () => {
    const { toasts, api, viewport } = await ready('cloneStamp');
    await drag(viewport, [10, 10], [20, 20]);
    expect(api.strokeBegin).not.toHaveBeenCalled();
    expect(toasts.items.some((t) => t.message === 'Alt+click to set the source point')).toBe(true);
  });

  it('shows a toast and starts no stroke when the mask is the paint target', async () => {
    const { store, toasts, api, viewport } = await ready('cloneStamp', [layer(1, { hasMask: true, maskEnabled: true })]);
    store.selectMaskTarget(1);
    await fireEvent.pointerDown(viewport, point(5, 5, { altKey: true }));
    await drag(viewport, [10, 10], [20, 20]);
    expect(api.strokeBegin).not.toHaveBeenCalled();
    expect(toasts.items.some((t) => t.message === 'Clone stamp and healing brush work on layer pixels only')).toBe(true);
  });

  it('aligned: the second stroke uses the stored offset; a new source resets it', async () => {
    const { api, viewport } = await ready('cloneStamp');
    await fireEvent.pointerDown(viewport, point(60, 40, { altKey: true }));
    await drag(viewport, [10, 20], [12, 22]);
    await drag(viewport, [50, 50], [52, 52]);
    expect(api.strokeBegin.mock.calls[0]?.[2]).toEqual({ type: 'clone', offset: [50, 20], sampleAllLayers: false });
    expect(api.strokeBegin.mock.calls[1]?.[2]).toEqual({ type: 'clone', offset: [50, 20], sampleAllLayers: false });
    await fireEvent.pointerDown(viewport, point(0, 0, { altKey: true }));
    await drag(viewport, [50, 50], [52, 52]);
    expect(api.strokeBegin.mock.calls[2]?.[2]).toEqual({ type: 'clone', offset: [-50, -50], sampleAllLayers: false });
  });

  it('not aligned: each stroke starts again at the source point', async () => {
    const { store, api, viewport } = await ready('cloneStamp');
    store.cloneAligned = false;
    await fireEvent.pointerDown(viewport, point(60, 40, { altKey: true }));
    await drag(viewport, [10, 20], [12, 22]);
    await drag(viewport, [50, 50], [52, 52]);
    expect(api.strokeBegin.mock.calls[1]?.[2]).toEqual({ type: 'clone', offset: [10, -10], sampleAllLayers: false });
  });

  it('draws the source crosshair at the source point, then at pointer plus offset', async () => {
    const { viewport } = await ready('cloneStamp');
    expect(screen.queryByTestId('clone-source')).toBeNull();
    await fireEvent.pointerDown(viewport, point(60, 40, { altKey: true }));
    await fireEvent.pointerUp(viewport, point(60, 40, { altKey: true }));
    await flush();
    const marker = screen.getByTestId('clone-source');
    expect([marker.getAttribute('data-x'), marker.getAttribute('data-y')]).toEqual(['60', '40']);
    await drag(viewport, [10, 20], [30, 40]);
    await fireEvent.pointerMove(viewport, point(100, 5));
    await flush();
    const moved = screen.getByTestId('clone-source');
    expect([moved.getAttribute('data-x'), moved.getAttribute('data-y')]).toEqual(['150', '25']);
  });
});
