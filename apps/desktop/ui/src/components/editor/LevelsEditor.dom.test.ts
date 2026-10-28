// @vitest-environment happy-dom
import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, EditDto, EditorApi, LayerInfo } from '../../lib/editor/api';
import { identityChannel, type LevelsAdjustment } from '../../lib/editor/levels';
import { EditorStore } from '../../lib/stores/editor.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import AdjustmentPanel from './AdjustmentPanel.svelte';

const levels: LevelsAdjustment = {
  type: 'levels',
  master: identityChannel(),
  red: identityChannel(),
  green: identityChannel(),
  blue: identityChannel(),
};

const layers: LayerInfo[] = [
  {
    id: 1,
    name: 'Levels',
    kind: 'adjustment',
    visible: true,
    opacity: 1,
    fillOpacity: 1,
    blendMode: 'normal',
    hasMask: false,
    adjustment: levels,
    isolated: null,
    children: null,
  },
];

function doc(revision = 0): DocumentInfo {
  return { documentId: 1, width: 100, height: 80, layers, canUndo: revision > 0, canRedo: false, revision, dirty: revision > 0, savedPhotoId: null };
}

async function makeStore() {
  let revision = 0;
  const api = {
    open: vi.fn(async () => doc()),
    render: vi.fn(async () => new ArrayBuffer(0)),
    edit: vi.fn(async (_id: number, edit: EditDto, _gesture: number | null) => {
      // Like the shell: the new document holds the edited adjustment.
      if (edit.type === 'setAdjustment' && layers[0] !== undefined) {
        layers[0] = { ...layers[0], adjustment: edit.adjustment };
      }
      return doc(++revision);
    }),
    undo: vi.fn(async () => doc(++revision)),
    redo: vi.fn(async () => doc(++revision)),
    save: vi.fn(),
    close: vi.fn(async () => undefined),
  } as unknown as EditorApi & { edit: ReturnType<typeof vi.fn> };
  const store = new EditorStore({
    library: { focusIndex: null, photoAt: () => undefined, selectedIds: [] },
    modules: { active: 'editor', switchTo: vi.fn() } as never,
    toasts: new ToastStore(),
    api,
  });
  await store.openPhoto(1);
  store.select(1);
  return { store, api };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

function lastAdjustment(api: { edit: ReturnType<typeof vi.fn> }): LevelsAdjustment {
  const call = api.edit.mock.calls.at(-1) as [number, EditDto, number | null];
  const edit = call[1];
  if (edit.type !== 'setAdjustment' || edit.adjustment.type !== 'levels') {
    throw new Error('not a levels edit');
  }
  return edit.adjustment;
}

describe('LevelsEditor', () => {
  it('sends a levels edit with a gesture for a slider move', async () => {
    const { store, api } = await makeStore();
    render(AdjustmentPanel, { editor: store });
    const slider = screen.getByRole<HTMLInputElement>('slider', { name: 'Gamma' });
    await fireEvent.input(slider, { target: { value: '2' } });
    await flush();
    expect(api.edit).toHaveBeenCalledTimes(1);
    expect(lastAdjustment(api).master.gamma).toBe(2);
    expect(typeof api.edit.mock.calls[0]?.[2]).toBe('number');
  });

  it('clamps the input black below the input white', async () => {
    const { store, api } = await makeStore();
    render(AdjustmentPanel, { editor: store });
    const white = screen.getByRole<HTMLInputElement>('slider', { name: 'Input white' });
    await fireEvent.input(white, { target: { value: '100' } });
    await flush();
    const field = screen.getByLabelText<HTMLInputElement>('Input black value');
    field.value = '200';
    await fireEvent.change(field);
    await flush();
    const adj = lastAdjustment(api);
    expect(adj.master.inputBlack).toBeLessThan(adj.master.inputWhite);
  });

  it('keeps the other channels when the channel changes', async () => {
    const { store, api } = await makeStore();
    render(AdjustmentPanel, { editor: store });
    await fireEvent.input(screen.getByRole('slider', { name: 'Gamma' }), { target: { value: '2' } });
    await flush();
    await fireEvent.click(screen.getByRole('button', { name: 'Red' }));
    await fireEvent.input(screen.getByRole('slider', { name: 'Output white' }), { target: { value: '128' } });
    await flush();
    const adj = lastAdjustment(api);
    expect(adj.master.gamma).toBe(2);
    expect(adj.red.outputWhite).toBeCloseTo(128 / 255);
    expect(adj.green).toEqual(identityChannel());
  });

  it('resets one channel', async () => {
    const { store, api } = await makeStore();
    render(AdjustmentPanel, { editor: store });
    await fireEvent.input(screen.getByRole('slider', { name: 'Gamma' }), { target: { value: '2' } });
    await flush();
    await fireEvent.click(screen.getByRole('button', { name: 'Reset master' }));
    await flush();
    expect(lastAdjustment(api).master).toEqual(identityChannel());
  });
});
