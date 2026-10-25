// @vitest-environment happy-dom
import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, EditDto, EditorApi, LayerInfo } from '../../lib/editor/api';
import { EditorStore } from '../../lib/stores/editor.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import AdjustmentPanel from './AdjustmentPanel.svelte';
import LayersPanel from './LayersPanel.svelte';

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
    adjustment: null,
    isolated: null,
    children: null,
    ...extra,
  };
}

const bc = { type: 'brightnessContrast', brightness: 0, contrast: 0 } as const;

function doc(layers: LayerInfo[], revision = 0): DocumentInfo {
  return { documentId: 1, width: 100, height: 80, layers, canUndo: revision > 0, canRedo: false, revision, dirty: revision > 0, savedPhotoId: null };
}

async function makeStore(layers: LayerInfo[]) {
  let revision = 0;
  const api = {
    open: vi.fn(async () => doc(layers)),
    render: vi.fn(async () => new ArrayBuffer(0)),
    edit: vi.fn(async (_id: number, _edit: EditDto, _gesture: number | null) => doc(layers, ++revision)),
    undo: vi.fn(async () => doc(layers, ++revision)),
    redo: vi.fn(async () => doc(layers, ++revision)),
    save: vi.fn(async () => ({
      photoId: 9,
      documentPath: '/p/a-edit.exdoc',
      imagePath: '/p/a-edit.tif',
      created: true,
      info: { ...doc(layers, revision), dirty: false, savedPhotoId: 9 },
    })),
    close: vi.fn(async () => undefined),
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

describe('LayersPanel', () => {
  it('lists layers top first and selects on click', async () => {
    const { store } = await makeStore([layer(1), layer(2)]);
    render(LayersPanel, { editor: store });
    const names = screen.getAllByRole('button', { name: /^Layer \d$/ }).map((b) => b.textContent?.trim());
    expect(names).toEqual(['Layer 2', 'Layer 1']);
    await fireEvent.click(screen.getByRole('button', { name: 'Layer 1' }));
    expect(store.selectedId).toBe(1);
  });

  it('toggles visibility through an edit', async () => {
    const { store, api } = await makeStore([layer(1)]);
    render(LayersPanel, { editor: store });
    await fireEvent.click(screen.getByRole('button', { name: 'Hide Layer 1' }));
    await flush();
    expect(api.edit).toHaveBeenCalledWith(1, { type: 'setVisible', layerId: 1, visible: false }, null);
  });

  it('renames on double-click and Enter', async () => {
    const { store, api } = await makeStore([layer(1)]);
    render(LayersPanel, { editor: store });
    await fireEvent.dblClick(screen.getByRole('button', { name: 'Layer 1' }));
    const input = screen.getByRole<HTMLInputElement>('textbox', { name: 'Layer name' });
    input.value = 'Sky';
    await fireEvent.keyDown(input, { key: 'Enter' });
    await flush();
    expect(api.edit).toHaveBeenCalledWith(1, { type: 'rename', layerId: 1, name: 'Sky' }, null);
  });

  it('expands a group', async () => {
    const { store } = await makeStore([layer(1), layer(2, { kind: 'group', isolated: true, children: [layer(3)] })]);
    render(LayersPanel, { editor: store });
    expect(screen.queryByRole('button', { name: 'Layer 3' })).toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Expand Layer 2' }));
    expect(screen.getByRole('button', { name: 'Layer 3' })).toBeTruthy();
  });

  it('sends live opacity edits with a gesture and sets the blend mode', async () => {
    const { store, api } = await makeStore([layer(1)]);
    render(LayersPanel, { editor: store });
    const range = screen.getByRole<HTMLInputElement>('slider', { name: 'Opacity' });
    range.value = '40';
    await fireEvent.input(range);
    await flush();
    const call = api.edit.mock.calls[0];
    expect(call?.[1]).toEqual({ type: 'setOpacity', layerId: 1, opacity: 0.4 });
    expect(typeof call?.[2]).toBe('number');

    const select = screen.getByRole<HTMLSelectElement>('combobox', { name: 'Blend mode' });
    select.value = 'multiply';
    await fireEvent.change(select);
    await flush();
    expect(api.edit).toHaveBeenLastCalledWith(1, { type: 'setBlendMode', layerId: 1, mode: 'multiply' }, null);
  });

  it('adds an adjustment above the selection with default parameters', async () => {
    const { store, api } = await makeStore([layer(1), layer(2)]);
    render(LayersPanel, { editor: store });
    await fireEvent.click(screen.getByRole('button', { name: 'Add adjustment' }));
    await fireEvent.click(screen.getByRole('menuitem', { name: 'Hue/Saturation' }));
    await flush();
    expect(api.edit).toHaveBeenCalledWith(
      1,
      { type: 'addAdjustment', adjustment: { type: 'hueSaturation', hue: 0, saturation: 0, lightness: 0 }, aboveLayerId: 2 },
      null,
    );
  });

  it('disables move up for the top layer and sends duplicate, delete, and undo', async () => {
    const { store, api } = await makeStore([layer(1), layer(2)]);
    render(LayersPanel, { editor: store });
    expect(screen.getByRole<HTMLButtonElement>('button', { name: 'Move layer up' }).disabled).toBe(true);
    await fireEvent.click(screen.getByRole('button', { name: 'Move layer down' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Duplicate' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await flush();
    expect(api.edit.mock.calls.map((c) => c[1].type)).toEqual(['move', 'duplicate', 'delete']);
    await fireEvent.click(screen.getByRole('button', { name: 'Undo' }));
    await flush();
    expect(api.undo).toHaveBeenCalledWith(1);
  });
});

describe('LayersPanel save', () => {
  it('shows the unsaved marker after an edit and saves on the Save button', async () => {
    const { store, api } = await makeStore([layer(1)]);
    render(LayersPanel, { editor: store });
    expect(screen.getByTestId('editor-save-state').textContent?.trim()).toBe('Not saved');
    await fireEvent.click(screen.getByRole('button', { name: 'Hide Layer 1' }));
    await flush();
    const state = screen.getByTestId('editor-save-state');
    expect(state.textContent).toContain('•');
    expect(state.textContent).toContain('Unsaved changes');
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await flush();
    expect(api.save).toHaveBeenCalledWith(1, undefined);
    expect(screen.getByTestId('editor-save-state').textContent?.trim()).toBe('Saved');
  });
});

describe('AdjustmentPanel', () => {
  it('shows a hint without an adjustment layer', async () => {
    const { store } = await makeStore([layer(1)]);
    render(AdjustmentPanel, { editor: store });
    expect(screen.getByText('Select an adjustment layer to edit it.')).toBeTruthy();
  });

  it('edits brightness with a full adjustment and a gesture', async () => {
    const { store, api } = await makeStore([layer(1, { kind: 'adjustment', adjustment: { ...bc } })]);
    render(AdjustmentPanel, { editor: store });
    const range = screen.getByRole<HTMLInputElement>('slider', { name: 'Brightness' });
    range.value = '50';
    await fireEvent.input(range);
    await flush();
    const call = api.edit.mock.calls[0];
    expect(call?.[1]).toEqual({
      type: 'setAdjustment',
      layerId: 1,
      adjustment: { type: 'brightnessContrast', brightness: 0.5, contrast: 0 },
    });
    expect(typeof call?.[2]).toBe('number');
  });

  it('says that levels have no editor yet', async () => {
    const channel = { inputBlack: 0, inputWhite: 1, gamma: 1, outputBlack: 0, outputWhite: 1 };
    const { store } = await makeStore([
      layer(1, { kind: 'adjustment', adjustment: { type: 'levels', master: channel, red: channel, green: channel, blue: channel } }),
    ]);
    render(AdjustmentPanel, { editor: store });
    expect(screen.getByText('No editor yet.')).toBeTruthy();
  });
});
