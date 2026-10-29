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
    adjustment: null,
    isolated: null,
    children: null,
    locks: NONE,
    ...extra,
  };
}

function doc(layers: LayerInfo[], revision = 0): DocumentInfo {
  return { documentId: 1, name: 'doc', width: 100, height: 80, layers, canUndo: false, canRedo: false, revision, dirty: false, savedPhotoId: null, sourcePhotoId: 1, hasSelection: false, selectionVersion: 0 };
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

describe('LayersPanel new layer and locks', () => {
  it('sends addPixelLayer above the selected layer', async () => {
    const { store, api } = await makeStore([layer(1), layer(2)]);
    render(LayersPanel, { editor: store });
    await fireEvent.click(screen.getByRole('button', { name: 'New layer' }));
    await flush();
    expect(api.edit.mock.calls[0]?.[1]).toEqual({ type: 'addPixelLayer', aboveLayerId: 2 });
  });

  it('toggles a lock with the full locks value', async () => {
    const { store, api } = await makeStore([layer(1, { locks: { ...NONE, position: true } })]);
    render(LayersPanel, { editor: store });
    const pixels = screen.getByRole('button', { name: 'Pixels' });
    expect(pixels.getAttribute('aria-pressed')).toBe('false');
    expect(screen.getByRole('button', { name: 'Position' }).getAttribute('aria-pressed')).toBe('true');
    await fireEvent.click(pixels);
    await flush();
    expect(api.edit.mock.calls[0]?.[1]).toEqual({
      type: 'setLocks',
      layerId: 1,
      locks: { transparency: false, pixels: true, position: true, all: false },
    });
  });

  it('shows a lock mark on a locked row', async () => {
    const { store } = await makeStore([layer(1, { locks: { ...NONE, all: true } }), layer(2)]);
    render(LayersPanel, { editor: store });
    expect(screen.getAllByRole('img', { name: 'Locked' })).toHaveLength(1);
  });

  it('disables opacity and blend mode when a parent group has the all lock', async () => {
    const group = layer(3, { kind: 'group', isolated: true, locks: { ...NONE, all: true }, children: [layer(4)] });
    const { store } = await makeStore([group]);
    store.toggleExpanded(3);
    store.select(4);
    render(LayersPanel, { editor: store });
    expect(screen.getByRole<HTMLSelectElement>('combobox', { name: 'Blend mode' }).disabled).toBe(true);
    expect(screen.getByRole<HTMLInputElement>('slider', { name: 'Opacity' }).disabled).toBe(true);
  });
});
