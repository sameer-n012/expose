// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, EditorApi, LayerInfo, SampledColor } from '../../lib/editor/api';
import { EDITOR_ACTION, editorActions } from '../../lib/editor/editorActions';
import { thumbnailBuffer } from '../../lib/editor/fakeStrokeApi';
import { EditorStore } from '../../lib/stores/editor.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import EditorView from './EditorView.svelte';
import LayersPanel from './LayersPanel.svelte';
import SelectionToolbar from './SelectionToolbar.svelte';

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
    save: vi.fn(),
    close: vi.fn(async () => undefined),
    selectionOutline: vi.fn(async () => ({ selectionVersion: 0, step: 1, contours: [] })),
    maskThumbnail: vi.fn(async (..._args: unknown[]) => thumbnailBuffer(2, 2)),
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

describe('brush shortcuts', () => {
  it('binds the keys', async () => {
    const { store } = await makeStore([layer(1)]);
    const keys = (id: string) => find(store, id)?.defaultBinding;
    expect(keys(EDITOR_ACTION.brushTool)).toBe('B');
    expect(keys(EDITOR_ACTION.eraserTool)).toBe('E');
    expect(keys(EDITOR_ACTION.brushSizeDown)).toBe('[');
    expect(keys(EDITOR_ACTION.brushSizeUp)).toBe(']');
    expect(keys(EDITOR_ACTION.brushSofter)).toBe('{');
    expect(keys(EDITOR_ACTION.brushHarder)).toBe('}');
    expect(keys(EDITOR_ACTION.swapColors)).toBe('X');
    expect(keys(EDITOR_ACTION.resetColors)).toBe('D');
  });

  it('selects the tools and changes size, hardness, and colors', async () => {
    const { store } = await makeStore([layer(1)]);
    find(store, EDITOR_ACTION.brushTool)?.run();
    expect(store.tool).toBe('brush');
    find(store, EDITOR_ACTION.brushSizeUp)?.run();
    expect(store.brush.size).toBe(35);
    find(store, EDITOR_ACTION.brushSizeDown)?.run();
    find(store, EDITOR_ACTION.brushSizeDown)?.run();
    expect(store.brush.size).toBe(25);
    find(store, EDITOR_ACTION.brushHarder)?.run();
    expect(store.brush.hardness).toBe(1);
    find(store, EDITOR_ACTION.brushSofter)?.run();
    expect(store.brush.hardness).toBe(0.75);
    find(store, EDITOR_ACTION.eraserTool)?.run();
    expect(store.tool).toBe('eraser');
    // The eraser has its own settings.
    expect(store.eraser.size).toBe(50);
    find(store, EDITOR_ACTION.swapColors)?.run();
    expect([store.foreground, store.background]).toEqual(['#ffffff', '#000000']);
    store.foreground = '#336699';
    find(store, EDITOR_ACTION.resetColors)?.run();
    expect([store.foreground, store.background]).toEqual(['#000000', '#ffffff']);
  });

  it('does not select a paint tool on a layer that cannot take paint', async () => {
    const { store } = await makeStore([layer(1, { kind: 'adjustment' })]);
    find(store, EDITOR_ACTION.brushTool)?.run();
    expect(store.tool).toBe('hand');
    render(SelectionToolbar, { editor: store });
    const button = screen.getByRole<HTMLButtonElement>('button', { name: 'Brush' });
    expect(button.disabled).toBe(true);
    expect(button.title).toMatch(/pixel layer/);
  });
});

describe('paint target', () => {
  it('a click on the mask chip chooses the mask, a click on the layer row chooses the pixels', async () => {
    const { store } = await makeStore([layer(1, { hasMask: true, maskEnabled: true })]);
    render(LayersPanel, { editor: store });
    expect(store.paintTarget).toBe('pixels');
    const chip = screen.getByRole('button', { name: 'Mask of Layer 1 is on' });
    await fireEvent.click(chip);
    expect(store.paintTarget).toBe('mask');
    expect(chip.getAttribute('data-paint-target')).toBe('true');
    await fireEvent.click(screen.getByRole('button', { name: 'Layer 1' }));
    expect(store.paintTarget).toBe('pixels');
    expect(chip.getAttribute('data-paint-target')).toBe('false');
  });

  it('is the pixels when the selection moves to another layer', async () => {
    const { store } = await makeStore([layer(1, { hasMask: true, maskEnabled: true }), layer(2)]);
    store.selectMaskTarget(1);
    expect(store.paintTarget).toBe('mask');
    store.select(2);
    expect(store.paintTarget).toBe('pixels');
    store.select(1);
    expect(store.paintTarget).toBe('pixels');
  });

  it('allows the brush on a mask of an adjustment layer', async () => {
    const { store } = await makeStore([layer(1, { kind: 'adjustment', hasMask: true, maskEnabled: true })]);
    expect(store.paintBlockReason).not.toBeNull();
    store.selectMaskTarget(1);
    expect(store.paintBlockReason).toBeNull();
  });
});

describe('EditorView strokes', () => {
  async function ready(tool: 'brush' | 'eraser' | 'eyedropper', layers: LayerInfo[] = [layer(1)]) {
    const made = await makeStore(layers);
    render(EditorView, { editor: made.store, toasts: made.toasts });
    await flush();
    // happy-dom has no layout: set the view by hand (100 % zoom, origin 0).
    made.store.view = { zoom: 1, originX: 0, originY: 0 };
    made.store.setTool(tool);
    await flush();
    return { ...made, viewport: screen.getByTestId('editor-viewport') };
  }

  it('a brush drag sends begin, points, and end with a paint target', async () => {
    const { store, api, viewport } = await ready('brush');
    store.foreground = '#ff0000';
    await fireEvent.pointerDown(viewport, point(10, 20));
    await fireEvent.pointerMove(viewport, point(30, 40));
    await flush();
    await fireEvent.pointerUp(viewport, point(50, 60));
    await flush();
    await flush();
    expect(api.strokeBegin).toHaveBeenCalledTimes(1);
    const [documentId, layerId, target, brush] = api.strokeBegin.mock.calls[0] ?? [];
    expect(documentId).toBe(1);
    expect(layerId).toBe(1);
    expect(target).toEqual({ type: 'paint', color: [1, 0, 0] });
    expect(brush).toMatchObject({ size: 30, hardness: 0.8, opacity: 1, flow: 1, spacing: 0.25 });
    const sent = api.strokePoints.mock.calls.flatMap((c) => c[2] as number[]);
    expect(sent).toEqual([10, 20, 1, 30, 40, 1, 50, 60, 1]);
    expect(api.strokePoints.mock.calls[0]?.[1]).toBe(5);
    expect(api.strokeEnd).toHaveBeenCalledWith(1, 5);
    // The info of the end replaces the document info.
    expect(store.info?.revision).toBe(1);
  });

  it('the eraser on pixels sends erase', async () => {
    const { api, viewport } = await ready('eraser');
    await fireEvent.pointerDown(viewport, point(5, 5));
    await fireEvent.pointerUp(viewport, point(5, 5));
    await flush();
    await flush();
    expect(api.strokeBegin.mock.calls[0]?.[2]).toEqual({ type: 'erase' });
  });

  it('on a mask the brush sends the foreground luminance and the eraser the background luminance', async () => {
    const { store, api, viewport } = await ready('brush', [layer(1, { hasMask: true, maskEnabled: true })]);
    store.selectMaskTarget(1);
    store.foreground = '#808080';
    await fireEvent.pointerDown(viewport, point(5, 5));
    await fireEvent.pointerUp(viewport, point(5, 5));
    await flush();
    await flush();
    const brushTarget = api.strokeBegin.mock.calls[0]?.[2] as { type: string; value: number };
    expect(brushTarget.type).toBe('mask');
    expect(brushTarget.value).toBeCloseTo(128 / 255, 6);
    store.setTool('eraser');
    await flush();
    await fireEvent.pointerDown(viewport, point(5, 5));
    await fireEvent.pointerUp(viewport, point(5, 5));
    await flush();
    await flush();
    const eraserTarget = api.strokeBegin.mock.calls[1]?.[2] as { type: string; value: number };
    expect(eraserTarget.type).toBe('mask');
    expect(eraserTarget.value).toBeCloseTo(1, 6);
  });

  it('the brush sends its blend mode; the eraser and mask paint send normal', async () => {
    const { store, api, viewport } = await ready('brush', [layer(1, { hasMask: true, maskEnabled: true })]);
    store.updateBrush('brush', { blendMode: 'multiply' });
    store.updateBrush('eraser', { blendMode: 'multiply' });
    const tap = async () => {
      await fireEvent.pointerDown(viewport, point(5, 5));
      await fireEvent.pointerUp(viewport, point(5, 5));
      await flush();
      await flush();
    };
    await tap();
    expect(api.strokeBegin.mock.calls[0]?.[3]).toMatchObject({ blendMode: 'multiply' });
    store.setTool('eraser');
    await flush();
    await tap();
    expect(api.strokeBegin.mock.calls[1]?.[3]).toMatchObject({ blendMode: 'normal' });
    store.setTool('brush');
    store.selectMaskTarget(1);
    await flush();
    await tap();
    expect(api.strokeBegin.mock.calls[2]?.[3]).toMatchObject({ blendMode: 'normal' });
  });

  it('Escape cancels the stroke', async () => {
    const { api, viewport } = await ready('brush');
    await fireEvent.pointerDown(viewport, point(10, 20));
    await flush();
    await fireEvent.keyDown(window, { key: 'Escape' });
    await flush();
    await flush();
    expect(api.strokeCancel).toHaveBeenCalledWith(1, 5);
    expect(api.strokeEnd).not.toHaveBeenCalled();
  });

  it('a failed begin shows an error and drops the gesture', async () => {
    const { api, viewport, toasts } = await ready('brush');
    api.strokeBegin.mockRejectedValueOnce({ kind: 'invalidInput', message: 'locked' });
    await fireEvent.pointerDown(viewport, point(10, 20));
    await fireEvent.pointerMove(viewport, point(20, 20));
    await flush();
    await fireEvent.pointerUp(viewport, point(30, 30));
    await flush();
    await flush();
    expect(api.strokePoints).not.toHaveBeenCalled();
    expect(api.strokeEnd).not.toHaveBeenCalled();
    expect(toasts.items.some((t) => t.message.startsWith('Start the stroke failed'))).toBe(true);
  });

  it('does not start a stroke on a layer that cannot take paint', async () => {
    const { api, viewport } = await ready('brush', [layer(1, { kind: 'adjustment' })]);
    await fireEvent.pointerDown(viewport, point(10, 20));
    await fireEvent.pointerUp(viewport, point(10, 20));
    await flush();
    expect(api.strokeBegin).not.toHaveBeenCalled();
  });

  it('shows the brush circle at the pointer, with a crosshair for a small brush', async () => {
    const { store, viewport } = await ready('brush');
    await fireEvent.pointerMove(viewport, point(40, 50));
    const cursor = screen.getByTestId('brush-cursor');
    expect(cursor.querySelector('circle')?.getAttribute('r')).toBe('15');
    expect(cursor.querySelector('path')).toBeNull();
    store.updateBrush('brush', { size: 3 });
    await flush();
    expect(screen.getByTestId('brush-cursor').querySelector('path')).not.toBeNull();
  });
});

describe('eyedropper', () => {
  async function ready(tool: 'brush' | 'eyedropper', layers: LayerInfo[] = [layer(1)]) {
    const made = await makeStore(layers);
    render(EditorView, { editor: made.store, toasts: made.toasts });
    await flush();
    made.store.view = { zoom: 2, originX: 0, originY: 0 };
    made.store.setTool(tool);
    await flush();
    return { ...made, viewport: screen.getByTestId('editor-viewport') };
  }

  it('binds I', async () => {
    const { store } = await makeStore([layer(1)]);
    find(store, EDITOR_ACTION.eyedropperTool)?.run();
    expect(find(store, EDITOR_ACTION.eyedropperTool)?.defaultBinding).toBe('I');
    expect(store.tool).toBe('eyedropper');
  });

  it('a click samples the floored canvas pixel into the foreground; Alt+click into the background', async () => {
    const { store, api, viewport } = await ready('eyedropper');
    await fireEvent.pointerDown(viewport, point(21, 31));
    await flush();
    expect(api.sampleColor).toHaveBeenLastCalledWith(1, 10, 15, 1, 'composite', null);
    expect(store.foreground).toBe('#336699');
    api.sampleColor.mockResolvedValueOnce({ color: '#112233', alpha: 1 });
    await fireEvent.pointerDown(viewport, point(21, 31, { altKey: true }));
    await flush();
    expect(store.background).toBe('#112233');
    expect(store.foreground).toBe('#336699');
  });

  it('ignores a click outside the canvas', async () => {
    const { api, viewport } = await ready('eyedropper');
    await fireEvent.pointerDown(viewport, point(500, 500));
    await flush();
    expect(api.sampleColor).not.toHaveBeenCalled();
  });

  it('uses the size and source options, and the selected pixel layer', async () => {
    const { store, api, viewport } = await ready('eyedropper', [layer(7)]);
    store.sampleSize = 5;
    store.sampleSource = 'layer';
    await fireEvent.pointerDown(viewport, point(4, 4));
    await flush();
    expect(api.sampleColor).toHaveBeenLastCalledWith(1, 2, 2, 5, 'layer', 7);
  });

  it('asks for a pixel layer when the source is the current layer', async () => {
    const { store, api, viewport, toasts } = await ready('eyedropper', [layer(1, { kind: 'adjustment' })]);
    store.sampleSource = 'layer';
    await fireEvent.pointerDown(viewport, point(4, 4));
    await flush();
    expect(api.sampleColor).not.toHaveBeenCalled();
    expect(toasts.items.some((t) => t.message.includes('pixel layer'))).toBe(true);
  });

  it('shows a toast for transparent pixels and keeps the color', async () => {
    const { store, api, viewport, toasts } = await ready('eyedropper');
    api.sampleColor.mockResolvedValueOnce({ color: null, alpha: 0 });
    await fireEvent.pointerDown(viewport, point(4, 4));
    await flush();
    expect(store.foreground).toBe('#000000');
    expect(toasts.items.some((t) => t.message === 'No color here: the pixels are transparent.')).toBe(true);
  });

  it('shows an error toast when the call fails', async () => {
    const { api, viewport, toasts } = await ready('eyedropper');
    api.sampleColor.mockRejectedValueOnce({ kind: 'invalidInput', message: 'bad' });
    await fireEvent.pointerDown(viewport, point(4, 4));
    await flush();
    expect(toasts.items.some((t) => t.message.startsWith('Sample the color failed'))).toBe(true);
  });

  it('Alt+click with the brush samples and starts no stroke', async () => {
    const { store, api, viewport } = await ready('brush');
    await fireEvent.pointerDown(viewport, point(20, 20, { altKey: true }));
    await fireEvent.pointerUp(viewport, point(20, 20, { altKey: true }));
    await flush();
    expect(api.strokeBegin).not.toHaveBeenCalled();
    expect(api.sampleColor).toHaveBeenLastCalledWith(1, 10, 10, 1, 'composite', null);
    expect(store.foreground).toBe('#336699');
  });
});
