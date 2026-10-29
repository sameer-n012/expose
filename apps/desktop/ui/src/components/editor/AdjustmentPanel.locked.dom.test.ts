// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AdjustmentDto, DocumentInfo, EditorApi, LayerInfo } from '../../lib/editor/api';
import { identityCurve } from '../../lib/editor/curves';
import { identityChannel } from '../../lib/editor/levels';
import { EditorStore } from '../../lib/stores/editor.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import AdjustmentPanel from './AdjustmentPanel.svelte';

const levels = { type: 'levels', master: identityChannel(), red: identityChannel(), green: identityChannel(), blue: identityChannel() } as const;
const curves = { type: 'curves', master: identityCurve(), red: identityCurve(), green: identityCurve(), blue: identityCurve() } as const;
const brightness = { type: 'brightnessContrast', brightness: 0.2, contrast: 0 } as const;
const hueSat = { type: 'hueSaturation', hue: 10, saturation: 0, lightness: 0 } as const;

function layer(adjustment: AdjustmentDto, all: boolean): LayerInfo {
  return {
    id: 1,
    name: 'Adjustment',
    kind: 'adjustment',
    visible: true,
    opacity: 1,
    fillOpacity: 1,
    blendMode: 'normal',
    hasMask: false,
    maskEnabled: false,
    maskVersion: null,
    locks: { transparency: false, pixels: false, position: false, all },
    adjustment,
    isolated: null,
    children: null,
  };
}

async function makeStore(adjustment: AdjustmentDto, all: boolean) {
  const layers = [layer(adjustment, all)];
  const doc: DocumentInfo = { documentId: 1, name: 'doc', width: 100, height: 80, layers, canUndo: false, canRedo: false, revision: 0, dirty: false, savedPhotoId: null, sourcePhotoId: 1, hasSelection: false, selectionVersion: 0, canReselect: false };
  const api = {
    open: vi.fn(async () => doc),
    render: vi.fn(async () => new ArrayBuffer(0)),
    edit: vi.fn(async () => doc),
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

afterEach(cleanup);

/** Every native control of the panel (sliders, number fields, buttons). */
function controls(): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>('input, button, select'));
}

describe('AdjustmentPanel with the "all" lock', () => {
  it.each([
    ['brightness and contrast', brightness],
    ['hue and saturation', hueSat],
    ['levels', levels],
    ['curves', curves],
  ] as const)('disables the controls of %s', async (_name, adjustment) => {
    const { store } = await makeStore(adjustment, true);
    render(AdjustmentPanel, { editor: store });
    const all = controls();
    expect(all.length).toBeGreaterThan(0);
    // The channel buttons only change the view of the editor; the rest must be disabled.
    const enabled = all.filter((el) => !(el as HTMLInputElement | HTMLButtonElement).disabled && el.getAttribute('aria-pressed') === null);
    expect(enabled).toEqual([]);
  });

  it('enables the controls without the lock', async () => {
    const { store } = await makeStore(brightness, false);
    render(AdjustmentPanel, { editor: store });
    expect(controls().every((el) => !(el as HTMLInputElement).disabled)).toBe(true);
  });

  it('does not edit curve points while locked', async () => {
    const { store, api } = await makeStore(curves, true);
    render(AdjustmentPanel, { editor: store });
    const plot = screen.getByTestId('curve-plot');
    plot.getBoundingClientRect = () => ({ left: 0, top: 0, width: 256, height: 256 }) as DOMRect;
    await fireEvent.pointerDown(plot, { clientX: 128, clientY: 60, button: 0 });
    const point = screen.getAllByTestId('curve-point')[0];
    if (point === undefined) {
      throw new Error('no curve point');
    }
    await fireEvent.keyDown(point, { key: 'ArrowUp' });
    await fireEvent.dblClick(point);
    await new Promise((r) => setTimeout(r, 0));
    expect(api.edit).not.toHaveBeenCalled();
  });
});
