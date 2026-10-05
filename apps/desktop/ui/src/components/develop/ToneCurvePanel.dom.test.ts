/** DOM tests of the Tone curve panel: channels, adding and moving points, keys, reset. */
import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { CurvePoint } from '../../lib/develop/curve';
import type { DevelopDocument } from '../../lib/develop/settings';
import { DevelopStore, type DevelopApi } from '../../lib/stores/develop.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import ToneCurvePanel from './ToneCurvePanel.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const ID: CurvePoint[] = [
  { x: 0, y: 0 },
  { x: 1, y: 1 },
];
const INITIAL: DevelopDocument = {
  version: 1,
  white_balance: { mode: 'as_shot', temperature: 5500, tint: 0 },
  tone: { exposure: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0 },
  tone_curve: {
    master: ID,
    red: [
      { x: 0, y: 0 },
      { x: 0.5, y: 0.6 },
      { x: 1, y: 1 },
    ],
    green: ID,
    blue: ID,
  },
};

async function makeStore(): Promise<DevelopStore> {
  const api: DevelopApi = {
    getSettings: async () => INITIAL,
    setSettings: vi.fn(async (_id, s) => s),
    openPreview: async () => ({ session: 1, width: 4, height: 3 }) as never,
    closePreview: async () => undefined,
  };
  const store = new DevelopStore({
    library: { focusIndex: null, photoAt: () => undefined, selectedIds: [] },
    modules: { active: 'develop' },
    toasts: new ToastStore(),
    api,
  });
  await store.activate(1);
  return store;
}

/** Gives the plot a 256 x 256 box at the origin: client (8 + 240x, 8 + 240(1 - y)) is curve (x, y). */
function sizePlot(): Element {
  const plot = screen.getByTestId('curve-plot');
  plot.getBoundingClientRect = () => ({ left: 0, top: 0, width: 256, height: 256 }) as DOMRect;
  return plot;
}

function client(x: number, y: number): { clientX: number; clientY: number; button: number } {
  return { clientX: 8 + 240 * x, clientY: 8 + 240 * (1 - y), button: 0 };
}

describe('ToneCurvePanel', () => {
  it('shows the curve of the chosen channel', async () => {
    const store = await makeStore();
    render(ToneCurvePanel, { develop: store });
    expect(screen.getAllByTestId('curve-point').length).toBe(2);
    await fireEvent.click(screen.getByRole('button', { name: 'Red' }));
    expect(screen.getAllByTestId('curve-point').length).toBe(3);
    expect(screen.getByRole('button', { name: 'Red' }).getAttribute('aria-pressed')).toBe('true');
  });

  it('adds a point on a click into the plot and updates the settings', async () => {
    const store = await makeStore();
    render(ToneCurvePanel, { develop: store });
    const plot = sizePlot();
    await fireEvent.pointerDown(plot, client(0.5, 0.7));
    await fireEvent.pointerUp(plot, client(0.5, 0.7));
    const master = store.settings?.tone_curve?.master ?? [];
    expect(master.length).toBe(3);
    expect(master[1]?.x).toBeCloseTo(0.5, 6);
    expect(master[1]?.y).toBeCloseTo(0.7, 6);
    // The other channels stay.
    expect(store.settings?.tone_curve?.red?.length).toBe(3);
    expect(screen.getAllByTestId('curve-point').length).toBe(3);
  });

  it('moves a point with a drag and removes it with a drag far outside', async () => {
    const store = await makeStore();
    render(ToneCurvePanel, { develop: store });
    const plot = sizePlot();
    await fireEvent.pointerDown(plot, client(0.5, 0.5));
    await fireEvent.pointerMove(plot, client(0.4, 0.3));
    await fireEvent.pointerUp(plot, client(0.4, 0.3));
    expect(store.settings?.tone_curve?.master?.[1]?.y).toBeCloseTo(0.3, 6);
    const point = screen.getAllByTestId('curve-point')[1] as Element;
    await fireEvent.pointerDown(point, client(0.4, 0.3));
    await fireEvent.pointerMove(plot, client(0.4, 1.6));
    await fireEvent.pointerUp(plot, client(0.4, 1.6));
    expect(store.settings?.tone_curve?.master?.length).toBe(2);
  });

  it('moves a point with the arrow keys and removes it with Delete', async () => {
    const store = await makeStore();
    render(ToneCurvePanel, { develop: store });
    await fireEvent.click(screen.getByRole('button', { name: 'Red' }));
    const point = screen.getAllByTestId('curve-point')[1] as Element;
    await fireEvent.keyDown(point, { key: 'ArrowUp' });
    expect(store.settings?.tone_curve?.red?.[1]?.y).toBeCloseTo(0.61, 6);
    await fireEvent.keyDown(point, { key: 'ArrowLeft', shiftKey: true });
    expect(store.settings?.tone_curve?.red?.[1]?.x).toBeCloseTo(0.4, 6);
    await fireEvent.keyDown(point, { key: 'Delete' });
    expect(store.settings?.tone_curve?.red?.length).toBe(2);
  });

  it('removes an inner point on a double-click, but not an end point', async () => {
    const store = await makeStore();
    render(ToneCurvePanel, { develop: store });
    await fireEvent.click(screen.getByRole('button', { name: 'Red' }));
    const points = screen.getAllByTestId('curve-point');
    await fireEvent.dblClick(points[0] as Element);
    expect(store.settings?.tone_curve?.red?.length).toBe(3);
    await fireEvent.dblClick(points[1] as Element);
    expect(store.settings?.tone_curve?.red?.length).toBe(2);
  });

  it('resets the current channel to the identity curve', async () => {
    const store = await makeStore();
    render(ToneCurvePanel, { develop: store });
    await fireEvent.click(screen.getByRole('button', { name: 'Red' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Reset red' }));
    expect(store.settings?.tone_curve?.red).toEqual(ID);
    expect(screen.getByRole<HTMLButtonElement>('button', { name: 'Reset red' }).disabled).toBe(true);
  });
});
