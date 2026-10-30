// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { EditorStore } from '../../lib/stores/editor.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import GradientEditorDialog from './GradientEditorDialog.svelte';

afterEach(cleanup);

function makeStore(): EditorStore {
  return new EditorStore({
    library: { focusIndex: null, photoAt: () => undefined, selectedIds: [] },
    modules: { active: 'editor', switchTo: vi.fn() } as never,
    toasts: new ToastStore(),
    api: {} as never,
  });
}

function open() {
  const editor = makeStore();
  editor.gradientDialogOpen = true;
  render(GradientEditorDialog, { editor });
  const track = screen.getByTestId('gradient-track');
  // happy-dom has no layout: the bar is 200 px wide at x = 0.
  track.getBoundingClientRect = () => ({ left: 0, right: 200, top: 0, bottom: 28, width: 200, height: 28, x: 0, y: 0, toJSON: () => ({}) });
  return { editor, track };
}

const stops = () => screen.getAllByTestId('gradient-stop');

describe('gradient editor dialog', () => {
  it('starts with the two stops of the preset', () => {
    open();
    expect(stops()).toHaveLength(2);
  });

  it('a click on the bar adds a stop and selects it', async () => {
    const { track } = open();
    await fireEvent.pointerDown(track, { clientX: 100, clientY: 10, button: 0, pointerId: 1 });
    await fireEvent.pointerUp(track, { clientX: 100, clientY: 10, pointerId: 1 });
    expect(stops()).toHaveLength(3);
    expect(stops()[1]?.getAttribute('aria-pressed')).toBe('true');
    expect((screen.getByLabelText('Position (%)') as HTMLInputElement).value).toBe('50');
  });

  it('removing never goes below 2 stops', async () => {
    const { track } = open();
    await fireEvent.pointerDown(track, { clientX: 100, clientY: 10, button: 0, pointerId: 1 });
    await fireEvent.pointerUp(track, { clientX: 100, clientY: 10, pointerId: 1 });
    await fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Delete' });
    expect(stops()).toHaveLength(2);
    await fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Delete' });
    expect(stops()).toHaveLength(2);
  });

  it('a drag far below the bar removes the stop', async () => {
    const { track } = open();
    await fireEvent.pointerDown(track, { clientX: 100, clientY: 10, button: 0, pointerId: 1 });
    await fireEvent.pointerMove(track, { clientX: 100, clientY: 120, pointerId: 1 });
    await fireEvent.pointerUp(track, { clientX: 100, clientY: 120, pointerId: 1 });
    expect(stops()).toHaveLength(2);
  });

  it('OK applies the stops and sets the preset to custom; Cancel discards', async () => {
    const { editor, track } = open();
    await fireEvent.pointerDown(track, { clientX: 100, clientY: 10, button: 0, pointerId: 1 });
    await fireEvent.pointerUp(track, { clientX: 100, clientY: 10, pointerId: 1 });
    await fireEvent.click(screen.getByText('OK'));
    expect(editor.gradient.preset).toBe('custom');
    expect(editor.gradient.stops).toHaveLength(3);
    expect(editor.gradientDialogOpen).toBe(false);

    const second = makeStore();
    second.gradientDialogOpen = true;
    cleanup();
    render(GradientEditorDialog, { editor: second });
    await fireEvent.click(screen.getByText('Cancel'));
    expect(second.gradient.preset).toBe('foregroundToBackground');
    expect(second.gradientDialogOpen).toBe(false);
  });
});
