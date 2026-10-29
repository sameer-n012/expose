// @vitest-environment happy-dom
import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, EditDto, EditorApi, LayerInfo } from '../../lib/editor/api';
import { strokeStubs } from '../../lib/editor/fakeStrokeApi';
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
    maskVersion: null,
    adjustment: null,
    isolated: null,
    children: null,
    locks: NONE,
    ...extra,
  };
}

function doc(layers: LayerInfo[], revision = 0): DocumentInfo {
  return { documentId: 1, name: 'doc', width: 100, height: 80, layers, canUndo: false, canRedo: false, revision, dirty: false, savedPhotoId: null, sourcePhotoId: 1, hasSelection: false, selectionVersion: 0, canReselect: false };
}

async function makeStore(layers: LayerInfo[]) {
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
    ...strokeStubs(),
  } satisfies EditorApi;
  const store = new EditorStore({
    library: { focusIndex: null, photoAt: () => undefined, selectedIds: [] },
    modules: { active: 'editor', switchTo: vi.fn() } as never,
    toasts: new ToastStore(),
    api,
  });
  await store.openPhoto(1);
  return { store, api };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe('LayersPanel masks', () => {
  it('adds a reveal mask on click and a hide mask on Alt+click', async () => {
    const { store, api } = await makeStore([layer(1)]);
    render(LayersPanel, { editor: store });
    const button = screen.getByRole('button', { name: 'Add mask' });
    await fireEvent.click(button);
    await flush();
    expect(api.edit.mock.calls[0]?.[1]).toEqual({ type: 'addMask', layerId: 1, fill: 'reveal' });
    await fireEvent.click(button, { altKey: true });
    await flush();
    expect(api.edit.mock.calls[1]?.[1]).toEqual({ type: 'addMask', layerId: 1, fill: 'hide' });
  });

  it('disables Add mask when the layer has a mask or the pixels lock is set', async () => {
    const masked = await makeStore([layer(1, { hasMask: true, maskEnabled: true })]);
    const first = render(LayersPanel, { editor: masked.store });
    expect(screen.getByRole<HTMLButtonElement>('button', { name: 'Add mask' }).disabled).toBe(true);
    first.unmount();
    const locked = await makeStore([layer(1, { locks: { ...NONE, pixels: true } })]);
    render(LayersPanel, { editor: locked.store });
    expect(screen.getByRole<HTMLButtonElement>('button', { name: 'Add mask' }).disabled).toBe(true);
  });

  it('shows a chip that toggles the mask on Shift+click', async () => {
    const { store, api } = await makeStore([layer(1, { hasMask: true, maskEnabled: true })]);
    render(LayersPanel, { editor: store });
    const chip = screen.getByRole('button', { name: 'Mask of Layer 1 is on' });
    await fireEvent.click(chip);
    await flush();
    expect(api.edit).not.toHaveBeenCalled();
    await fireEvent.click(chip, { shiftKey: true });
    await flush();
    expect(api.edit.mock.calls[0]?.[1]).toEqual({ type: 'setMaskEnabled', layerId: 1, enabled: false });
  });

  it('shows a disabled mask as off and the menu offers Enable and Delete', async () => {
    const { store, api } = await makeStore([layer(1, { hasMask: true, maskEnabled: false })]);
    render(LayersPanel, { editor: store });
    expect(screen.getByRole('button', { name: 'Mask of Layer 1 is off' })).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Mask' }));
    expect(screen.getByRole<HTMLButtonElement>('menuitem', { name: 'Add Mask (Reveal All)' }).disabled).toBe(true);
    await fireEvent.click(screen.getByRole('menuitem', { name: 'Enable Mask' }));
    await flush();
    expect(api.edit.mock.calls[0]?.[1]).toEqual({ type: 'setMaskEnabled', layerId: 1, enabled: true });
    await fireEvent.click(screen.getByRole('button', { name: 'Mask' }));
    await fireEvent.click(screen.getByRole('menuitem', { name: 'Delete Mask' }));
    await flush();
    expect(api.edit.mock.calls[1]?.[1]).toEqual({ type: 'deleteMask', layerId: 1 });
  });
});
