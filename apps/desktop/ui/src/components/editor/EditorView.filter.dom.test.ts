// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, EditorApi, FilterDto, FilterTarget, LayerInfo, SampledColor } from '../../lib/editor/api';
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
    filterPreview: vi.fn(async (_id: number, _layer: number, _target: FilterTarget, _filter: FilterDto) => {
      revision += 1;
      return doc();
    }),
    filterCommit: vi.fn(async (_id: number) => {
      revision += 1;
      return doc();
    }),
    filterCancel: vi.fn(async (_id: number) => {
      revision += 1;
      return doc();
    }),
    strokeBegin: vi.fn(async (..._args: unknown[]) => 5),
    strokePoints: vi.fn(async (..._args: unknown[]) => null),
    strokeEnd: vi.fn(async (..._args: unknown[]) => doc()),
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

async function ready(layers: LayerInfo[] = [layer(1)]) {
  const made = await makeStore(layers);
  render(EditorView, { editor: made.store, toasts: made.toasts });
  await flush();
  return made;
}

const filterButton = () => screen.getByRole('button', { name: /^Filter/ });

async function openMenu(item: string) {
  await fireEvent.click(filterButton());
  await fireEvent.click(screen.getByRole('menuitem', { name: item }));
  await flush();
}

describe('Filter menu', () => {
  it('is disabled for an adjustment layer, with a reason', async () => {
    await ready([layer(1, { kind: 'adjustment' })]);
    const button = filterButton() as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(button.title).toMatch(/pixel layer/);
  });

  it('is disabled for a layer with the pixels lock, and enabled for the mask', async () => {
    const { store } = await ready([layer(1, { hasMask: true, maskEnabled: true, locks: { ...NONE, pixels: true } })]);
    expect((filterButton() as HTMLButtonElement).disabled).toBe(true);
    store.selectMaskTarget(1);
    await flush();
    expect((filterButton() as HTMLButtonElement).disabled).toBe(false);
  });

  it('has shortcut actions without default keys', async () => {
    const { store } = await ready();
    const actions = editorActions(store);
    expect(actions.find((a) => a.id === EDITOR_ACTION.gaussianBlur)?.defaultBinding).toBeNull();
    actions.find((a) => a.id === EDITOR_ACTION.unsharpMask)?.run();
    await flush();
    expect(store.filterDialog?.kind).toBe('unsharpMask');
  });
});

describe('Gaussian Blur dialog', () => {
  it('previews with the layer, the target, and the last radius', async () => {
    const { api } = await ready();
    await openMenu('Gaussian Blur…');
    expect(screen.getByTestId('filter-dialog')).toBeTruthy();
    expect(api.filterPreview).toHaveBeenCalledWith(1, 1, 'pixels', { type: 'gaussianBlur', radius: 2 });
    const field = screen.getByLabelText('Radius value') as HTMLInputElement;
    await fireEvent.change(field, { target: { value: '7.5' } });
    await flush();
    expect(api.filterPreview).toHaveBeenLastCalledWith(1, 1, 'pixels', { type: 'gaussianBlur', radius: 7.5 });
  });

  it('uses the mask as the target when the mask chip is the paint target', async () => {
    const { api, store } = await ready([layer(1, { hasMask: true, maskEnabled: true })]);
    store.selectMaskTarget(1);
    await openMenu('Gaussian Blur…');
    expect(api.filterPreview).toHaveBeenCalledWith(1, 1, 'mask', { type: 'gaussianBlur', radius: 2 });
  });

  it('OK commits the previewed filter and closes the dialog', async () => {
    const { api, store } = await ready();
    await openMenu('Gaussian Blur…');
    await fireEvent.click(screen.getByRole('button', { name: 'OK' }));
    await flush();
    expect(api.filterPreview).toHaveBeenCalledTimes(1);
    expect(api.filterCommit).toHaveBeenCalledWith(1);
    expect(api.filterCancel).not.toHaveBeenCalled();
    expect(store.filterDialog).toBeNull();
    expect(screen.queryByTestId('filter-dialog')).toBeNull();
  });

  it('Escape cancels and closes the dialog', async () => {
    const { api, store } = await ready();
    await openMenu('Gaussian Blur…');
    await fireEvent.keyDown(screen.getByTestId('filter-dialog'), { key: 'Escape' });
    await flush();
    expect(api.filterCancel).toHaveBeenCalledWith(1);
    expect(api.filterCommit).not.toHaveBeenCalled();
    expect(store.filterDialog).toBeNull();
  });

  it('Enter in a field is OK', async () => {
    const { api } = await ready();
    await openMenu('Gaussian Blur…');
    await fireEvent.keyDown(screen.getByLabelText('Radius value'), { key: 'Enter' });
    await flush();
    expect(api.filterCommit).toHaveBeenCalledTimes(1);
  });

  it('turning the preview off cancels the session; OK then previews and commits', async () => {
    const { api } = await ready();
    await openMenu('Gaussian Blur…');
    await fireEvent.click(screen.getByLabelText('Preview'));
    await flush();
    expect(api.filterCancel).toHaveBeenCalledTimes(1);
    await fireEvent.click(screen.getByRole('button', { name: 'OK' }));
    await flush();
    expect(api.filterPreview).toHaveBeenCalledTimes(2);
    expect(api.filterCommit).toHaveBeenCalledTimes(1);
  });

  it('remembers the last radius', async () => {
    const { store } = await ready();
    await openMenu('Gaussian Blur…');
    await fireEvent.change(screen.getByLabelText('Radius value'), { target: { value: '12' } });
    await fireEvent.keyDown(screen.getByTestId('filter-dialog'), { key: 'Escape' });
    await flush();
    store.openFilterDialog('gaussianBlur');
    await flush();
    expect((screen.getByLabelText('Radius value') as HTMLInputElement).value).toBe('12.0');
  });

  it('blocks tool changes while open', async () => {
    const { store } = await ready();
    await openMenu('Gaussian Blur…');
    store.setTool('rectSelect');
    expect(store.tool).toBe('hand');
  });

  it('closes with a cancel when another layer is selected', async () => {
    const { api, store } = await ready([layer(1), layer(2)]);
    store.select(2);
    await openMenu('Gaussian Blur…');
    store.select(1);
    await flush();
    expect(store.filterDialog).toBeNull();
    expect(api.filterCancel).toHaveBeenCalledTimes(1);
  });

  it('shows a toast and closes the dialog when the preview fails', async () => {
    const { api, store, toasts } = await ready();
    api.filterPreview.mockRejectedValueOnce(new Error('boom'));
    await openMenu('Gaussian Blur…');
    await flush();
    expect(store.filterDialog).toBeNull();
    expect(toasts.items.length).toBeGreaterThan(0);
    expect(api.filterCancel).toHaveBeenCalledTimes(1);
  });
});

describe('Unsharp Mask dialog', () => {
  it('sends the amount as a fraction of the percent value', async () => {
    const { api } = await ready();
    await openMenu('Unsharp Mask…');
    expect(api.filterPreview).toHaveBeenCalledWith(1, 1, 'pixels', { type: 'unsharpMask', amount: 1, radius: 1, threshold: 0 });
    await fireEvent.change(screen.getByLabelText('Amount value'), { target: { value: '150' } });
    await fireEvent.change(screen.getByLabelText('Threshold value'), { target: { value: '12' } });
    await flush();
    expect(api.filterPreview).toHaveBeenLastCalledWith(1, 1, 'pixels', { type: 'unsharpMask', amount: 1.5, radius: 1, threshold: 12 });
  });

  it('OK commits the newest values', async () => {
    const { api } = await ready();
    await openMenu('Unsharp Mask…');
    await fireEvent.change(screen.getByLabelText('Amount value'), { target: { value: '200' } });
    await fireEvent.click(screen.getByRole('button', { name: 'OK' }));
    await flush();
    expect(api.filterPreview).toHaveBeenLastCalledWith(1, 1, 'pixels', { type: 'unsharpMask', amount: 2, radius: 1, threshold: 0 });
    expect(api.filterCommit).toHaveBeenCalledTimes(1);
  });
});
