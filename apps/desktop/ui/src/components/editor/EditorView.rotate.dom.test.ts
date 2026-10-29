// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, EditDto, EditorApi, LayerInfo } from '../../lib/editor/api';
import { strokeStubs } from '../../lib/editor/fakeStrokeApi';
import { EditorStore } from '../../lib/stores/editor.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import EditorView from './EditorView.svelte';

const NONE = { transparency: false, pixels: false, position: false, all: false };

const layer: LayerInfo = {
  id: 1,
  name: 'Layer 1',
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
};

const flush = () => new Promise((r) => setTimeout(r, 0));

const doc = (): DocumentInfo => ({
  documentId: 1,
  name: 'doc',
  width: 100,
  height: 80,
  layers: [layer],
  canUndo: false,
  canRedo: false,
  revision: 0,
  dirty: false,
  savedPhotoId: null,
  sourcePhotoId: 1,
  hasSelection: false,
  selectionVersion: 0,
  canReselect: false,
});

/** The viewport of the tests: happy-dom has no layout, so the store gets this size by hand. Center (50, 40). */
const VIEWPORT = { width: 100, height: 80 };

async function ready() {
  const api = {
    open: vi.fn(async () => doc()),
    newDocument: vi.fn(async () => doc()),
    render: vi.fn(async () => new ArrayBuffer(0)),
    edit: vi.fn(async (_id: number, _edit: EditDto, _gesture: number | null) => doc()),
    undo: vi.fn(async () => doc()),
    redo: vi.fn(async () => doc()),
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
  render(EditorView, { editor: store, toasts: new ToastStore() });
  await flush();
  store.setViewport(VIEWPORT);
  store.view = { zoom: 1, originX: 0, originY: 0, rotation: 0 };
  await flush();
  return { store, api, viewport: screen.getByTestId('editor-viewport') };
}

const point = (x: number, y: number, extra: object = {}) => ({ clientX: x, clientY: y, button: 0, pointerId: 1, ...extra });

afterEach(cleanup);

describe('EditorView rotate view', () => {
  it('has no transform at 0° and rotates the canvas layers', async () => {
    const { store } = await ready();
    const rotator = screen.getByTestId('editor-rotator');
    expect(rotator.style.transform).toBe('');
    expect(screen.queryByTestId('rotation-readout')).toBeNull();
    store.setRotation(30);
    await flush();
    expect(rotator.style.transform).toBe('rotate(30deg)');
    // The badges and readouts stay outside the rotated container.
    const readout = screen.getByTestId('rotation-readout');
    expect(rotator.contains(readout)).toBe(false);
    expect(readout.textContent).toContain('Rotation 30°');
  });

  it('the reset button sets the rotation to 0', async () => {
    const { store } = await ready();
    store.setRotation(-60);
    await flush();
    await fireEvent.click(screen.getByRole('button', { name: 'Reset' }));
    expect(store.rotation).toBe(0);
    expect(screen.queryByTestId('rotation-readout')).toBeNull();
  });

  it('a drag turns the view about the center; Shift snaps', async () => {
    const { store, viewport } = await ready();
    store.setTool('rotate');
    await flush();
    // From the right of the center (angle 0) to below it (angle 90).
    await fireEvent.pointerDown(viewport, point(80, 40));
    await fireEvent.pointerMove(viewport, point(50, 70));
    expect(store.rotation).toBeCloseTo(90);
    // 20° from the start snaps to 15°.
    const rad = (20 * Math.PI) / 180;
    await fireEvent.pointerMove(viewport, point(50 + 30 * Math.cos(rad), 40 + 30 * Math.sin(rad), { shiftKey: true }));
    expect(store.rotation).toBe(15);
    await fireEvent.pointerUp(viewport, point(50, 70));
    expect(store.rotation).toBe(15);
  });

  it('Escape during a drag gives back the start angle', async () => {
    const { store, viewport } = await ready();
    store.setRotation(10);
    store.setTool('rotate');
    await flush();
    await fireEvent.pointerDown(viewport, point(80, 40));
    await fireEvent.pointerMove(viewport, point(50, 70));
    expect(store.rotation).toBeCloseTo(100);
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(store.rotation).toBe(10);
    // The rest of the gesture does nothing.
    await fireEvent.pointerMove(viewport, point(20, 40));
    await fireEvent.pointerUp(viewport, point(20, 40));
    expect(store.rotation).toBe(10);
  });

  it('a cancelled pointer gives back the start angle', async () => {
    const { store, viewport } = await ready();
    store.setTool('rotate');
    await flush();
    await fireEvent.pointerDown(viewport, point(80, 40));
    await fireEvent.pointerMove(viewport, point(50, 70));
    await fireEvent.pointerCancel(viewport, point(50, 70));
    expect(store.rotation).toBe(0);
  });

  it('a marquee drag on a rotated view selects in canvas pixels', async () => {
    const { store, api, viewport } = await ready();
    store.setRotation(90);
    store.setTool('rectSelect');
    await flush();
    // The center stays; screen (70, 60) is (70, 20) in content space at 90°.
    await fireEvent.pointerDown(viewport, point(50, 40));
    await fireEvent.pointerMove(viewport, point(70, 60));
    await fireEvent.pointerUp(viewport, point(70, 60));
    await flush();
    expect(api.edit.mock.calls[0]?.[1]).toMatchObject({
      type: 'selectShape',
      shape: { type: 'rect', x: 50, y: 20, width: 20, height: 20 },
    });
  });

  it('the hand tool pans with the content under the pointer', async () => {
    const { store, viewport } = await ready();
    // Zoom in so that the document can move (a small document stays centered).
    store.view = { zoom: 4, originX: 20, originY: 20, rotation: 90 };
    store.setTool('hand');
    await flush();
    await fireEvent.pointerDown(viewport, point(50, 40));
    await fireEvent.pointerMove(viewport, point(60, 40));
    await fireEvent.pointerUp(viewport, point(60, 40));
    // A screen move of +10 x at 90° is a content move of -10 y: the origin moves by +10 / 4 in y.
    expect(store.view?.originX).toBeCloseTo(20);
    expect(store.view?.originY).toBeCloseTo(22.5);
  });
});
