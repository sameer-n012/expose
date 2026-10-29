// @vitest-environment happy-dom
import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, EditDto, EditorApi, LayerInfo } from '../../lib/editor/api';
import { identityCurve, type CurvesAdjustment } from '../../lib/editor/curves';
import { EditorStore } from '../../lib/stores/editor.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import AdjustmentPanel from './AdjustmentPanel.svelte';

function makeLayers(): LayerInfo[] {
  const curves: CurvesAdjustment = {
    type: 'curves',
    master: identityCurve(),
    red: [
      { x: 0, y: 0 },
      { x: 0.5, y: 0.6 },
      { x: 1, y: 1 },
    ],
    green: identityCurve(),
    blue: identityCurve(),
  };
  return [
    {
      id: 1,
      name: 'Curves',
      kind: 'adjustment',
      visible: true,
      opacity: 1,
      fillOpacity: 1,
      blendMode: 'normal',
      hasMask: false,
      maskEnabled: false,
      locks: { transparency: false, pixels: false, position: false, all: false },
      adjustment: curves,
      isolated: null,
      children: null,
    },
  ];
}

async function makeStore() {
  const layers = makeLayers();
  let revision = 0;
  const doc = (): DocumentInfo => ({
    documentId: 1,
    name: 'doc',
    width: 100,
    height: 80,
    layers,
    canUndo: revision > 0,
    canRedo: false,
    revision,
    dirty: revision > 0,
    savedPhotoId: null,
    sourcePhotoId: 1,
    hasSelection: false,
    selectionVersion: 0,
    canReselect: false,
  });
  const api = {
    open: vi.fn(async () => doc()),
    render: vi.fn(async () => new ArrayBuffer(0)),
    edit: vi.fn(async (_id: number, edit: EditDto, _gesture: number | null) => {
      // Like the shell: the new document holds the edited adjustment.
      if (edit.type === 'setAdjustment' && layers[0] !== undefined) {
        layers[0] = { ...layers[0], adjustment: edit.adjustment };
      }
      revision += 1;
      return doc();
    }),
    undo: vi.fn(),
    redo: vi.fn(),
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

function sizePlot(): Element {
  const plot = screen.getByTestId('curve-plot');
  plot.getBoundingClientRect = () => ({ left: 0, top: 0, width: 256, height: 256 }) as DOMRect;
  return plot;
}

/** Client position of curve point (x, y) for a 256 px plot. */
function client(x: number, y: number): { clientX: number; clientY: number; button: number } {
  return { clientX: 8 + 240 * x, clientY: 8 + 240 * (1 - y), button: 0 };
}

function lastCurves(api: { edit: ReturnType<typeof vi.fn> }): CurvesAdjustment {
  const call = api.edit.mock.calls.at(-1) as [number, EditDto, number | null];
  const edit = call[1];
  if (edit.type !== 'setAdjustment' || edit.adjustment.type !== 'curves') {
    throw new Error('not a curves edit');
  }
  return edit.adjustment;
}

describe('CurvesEditor', () => {
  it('adds a valid point on a click and keeps the other channels', async () => {
    const { store, api } = await makeStore();
    render(AdjustmentPanel, { editor: store });
    const plot = sizePlot();
    await fireEvent.pointerDown(plot, client(0.5, 0.7));
    await fireEvent.pointerUp(plot, client(0.5, 0.7));
    await flush();
    const adj = lastCurves(api);
    expect(adj.master).toHaveLength(3);
    expect(adj.master[1]?.x).toBeCloseTo(0.5, 6);
    expect(adj.master[1]?.y).toBeCloseTo(0.7, 6);
    expect(adj.red).toHaveLength(3);
    expect(adj.green).toEqual(identityCurve());
    // An add is a discrete edit: it has no gesture.
    expect(api.edit.mock.calls[0]?.[2]).toBeNull();
  });

  it('removes a middle point with Delete but never an end point', async () => {
    const { store, api } = await makeStore();
    render(AdjustmentPanel, { editor: store });
    await fireEvent.click(screen.getByRole('button', { name: 'Red' }));
    const points = screen.getAllByTestId('curve-point');
    expect(points).toHaveLength(3);
    await fireEvent.keyDown(points[1] as Element, { key: 'Delete' });
    await flush();
    expect(lastCurves(api).red).toEqual(identityCurve());
    const count = api.edit.mock.calls.length;
    await fireEvent.keyDown(screen.getAllByTestId('curve-point')[0] as Element, { key: 'Delete' });
    await flush();
    expect(lastCurves(api).red).toEqual(identityCurve());
    expect(api.edit.mock.calls.length).toBe(count + 1);
  });

  it('moves a point with one gesture for a drag', async () => {
    const { store, api } = await makeStore();
    render(AdjustmentPanel, { editor: store });
    await fireEvent.click(screen.getByRole('button', { name: 'Red' }));
    const plot = sizePlot();
    const point = screen.getAllByTestId('curve-point')[1] as Element;
    await fireEvent.pointerDown(point, { ...client(0.5, 0.6), pointerId: 1 });
    await fireEvent.pointerMove(plot, { ...client(0.5, 0.65), pointerId: 1 });
    await fireEvent.pointerMove(plot, { ...client(0.5, 0.7), pointerId: 1 });
    await fireEvent.pointerUp(plot, { ...client(0.5, 0.7), pointerId: 1 });
    await flush();
    const gestures = new Set(api.edit.mock.calls.map((c) => c[2]));
    expect(gestures.size).toBe(1);
    expect(typeof [...gestures][0]).toBe('number');
    expect(lastCurves(api).red[1]?.y).toBeCloseTo(0.7, 6);
  });

  it('switches the channel without changing the others', async () => {
    const { store } = await makeStore();
    render(AdjustmentPanel, { editor: store });
    expect(screen.getAllByTestId('curve-point')).toHaveLength(2);
    await fireEvent.click(screen.getByRole('button', { name: 'Red' }));
    expect(screen.getAllByTestId('curve-point')).toHaveLength(3);
  });
});
