// @vitest-environment happy-dom
import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, EditDto, EditorApi, LayerInfo } from '../../lib/editor/api';
import { EDITOR_ACTION, editorActions } from '../../lib/editor/editorActions';
import { EditorStore } from '../../lib/stores/editor.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import LayersPanel from './LayersPanel.svelte';

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

function doc(layers: LayerInfo[], revision = 0): DocumentInfo {
  return { documentId: 1, width: 100, height: 80, layers, canUndo: false, canRedo: false, revision, dirty: false, savedPhotoId: null, sourcePhotoId: 1, hasSelection: false, selectionVersion: 0 };
}

/** The layer list that the fake shell returns can change between calls. */
async function makeStore(initial: LayerInfo[]) {
  let layers = initial;
  let revision = 0;
  const api = {
    open: vi.fn(async () => doc(layers)),
    newDocument: vi.fn(async () => doc(layers)),
    render: vi.fn(async () => new ArrayBuffer(0)),
    edit: vi.fn(async (_id: number, _edit: EditDto, _gesture: number | null) => doc(layers, ++revision)),
    undo: vi.fn(async () => doc(layers, ++revision)),
    redo: vi.fn(async () => doc(layers, ++revision)),
    save: vi.fn(),
    close: vi.fn(async () => undefined),
    selectionOutline: vi.fn(async () => ({ selectionVersion: 0, step: 1, contours: [] })),
  } satisfies EditorApi;
  const store = new EditorStore({
    library: { focusIndex: null, photoAt: () => undefined, selectedIds: [] },
    modules: { active: 'editor', switchTo: vi.fn() } as never,
    toasts: new ToastStore(),
    api,
  });
  await store.openPhoto(1);
  return { store, api, setLayers: (next: LayerInfo[]) => (layers = next) };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe('show mask', () => {
  it('toggles for a selected layer with a mask', async () => {
    const { store } = await makeStore([layer(1, { hasMask: true, maskEnabled: true })]);
    expect(store.showMask).toBe(false);
    store.toggleShowMask();
    expect(store.showMaskLayerId).toBe(1);
    expect(store.showMask).toBe(true);
    store.toggleShowMask();
    expect(store.showMaskLayerId).toBeNull();
  });

  it('does nothing for a layer without a mask', async () => {
    const { store } = await makeStore([layer(1)]);
    store.toggleShowMask();
    expect(store.showMask).toBe(false);
  });

  it('turns off when the selection changes', async () => {
    const { store } = await makeStore([layer(1, { hasMask: true }), layer(2, { hasMask: true })]);
    store.select(1);
    store.toggleShowMask();
    expect(store.showMaskLayerId).toBe(1);
    store.select(2);
    expect(store.showMask).toBe(false);
    // Selecting layer 1 again does not bring the view back.
    store.select(1);
    expect(store.showMask).toBe(false);
  });

  it('turns off when the layer loses its mask', async () => {
    const { store, setLayers } = await makeStore([layer(1, { hasMask: true, maskEnabled: true })]);
    store.toggleShowMask();
    expect(store.showMask).toBe(true);
    setLayers([layer(1)]);
    store.deleteMask(1);
    await flush();
    expect(store.showMask).toBe(false);
  });

  it('turns off when the document closes', async () => {
    const { store } = await makeStore([layer(1, { hasMask: true })]);
    store.toggleShowMask();
    await store.dispose();
    expect(store.showMask).toBe(false);
  });

  it('the key action calls the store', async () => {
    const { store } = await makeStore([layer(1, { hasMask: true })]);
    const action = editorActions(store).find((a) => a.id === EDITOR_ACTION.showMask);
    expect(action?.defaultBinding).toBe('\\');
    action?.run();
    expect(store.showMask).toBe(true);
  });

  it('Alt+click on the chip selects the layer and shows its mask', async () => {
    const { store, api } = await makeStore([layer(1, { hasMask: true, maskEnabled: true }), layer(2, { hasMask: true, maskEnabled: true })]);
    render(LayersPanel, { editor: store });
    await fireEvent.click(screen.getByRole('button', { name: 'Mask of Layer 1 is on' }), { altKey: true });
    expect(store.selectedId).toBe(1);
    expect(store.showMaskLayerId).toBe(1);
    expect(api.edit).not.toHaveBeenCalled();
    await fireEvent.click(screen.getByRole('button', { name: 'Mask of Layer 1 is on' }), { altKey: true });
    expect(store.showMask).toBe(false);
  });

  it('the Mask menu has a checkable Show Mask item', async () => {
    const { store } = await makeStore([layer(1, { hasMask: true, maskEnabled: true })]);
    render(LayersPanel, { editor: store });
    await fireEvent.click(screen.getByRole('button', { name: 'Mask' }));
    await fireEvent.click(screen.getByRole('menuitemcheckbox', { name: 'Show Mask' }));
    expect(store.showMask).toBe(true);
    await fireEvent.click(screen.getByRole('button', { name: 'Mask' }));
    expect(screen.getByRole('menuitemcheckbox', { name: /Show Mask/ }).getAttribute('aria-checked')).toBe('true');
  });
});
