// @vitest-environment happy-dom
import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, EditorApi } from '../../lib/editor/api';
import { strokeStubs } from '../../lib/editor/fakeStrokeApi';
import { EditorStore } from '../../lib/stores/editor.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import HistoryPanel from './HistoryPanel.svelte';

function doc(cursor: number): DocumentInfo {
  return {
    documentId: 1,
    name: 'doc',
    width: 100,
    height: 80,
    layers: [],
    canUndo: cursor > 0,
    canRedo: cursor < 3,
    history: { steps: ['Brush', 'Move', 'Rename'], cursor },
    revision: 0,
    dirty: false,
    savedPhotoId: null,
    sourcePhotoId: 1,
    hasSelection: false,
    selectionVersion: 0,
    canReselect: false,
  };
}

function makeStore(cursor: number) {
  const api = {
    open: vi.fn(async () => doc(cursor)),
    newDocument: vi.fn(async () => doc(cursor)),
    render: vi.fn(async () => new ArrayBuffer(0)),
    edit: vi.fn(async () => doc(cursor)),
    undo: vi.fn(async () => doc(cursor)),
    redo: vi.fn(async () => doc(cursor)),
    historyJump: vi.fn(async (_documentId: number, step: number) => doc(step)),
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
  return { store, api };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe('HistoryPanel', () => {
  it('shows a text when no document is open', () => {
    const { store } = makeStore(0);
    render(HistoryPanel, { editor: store });
    expect(screen.getByText('No document')).toBeTruthy();
  });

  it('shows the start row and the steps, marks the current row, and dims redo rows', async () => {
    const { store } = makeStore(2);
    await store.openPhoto(1);
    render(HistoryPanel, { editor: store });
    const rows = screen.getAllByRole('option');
    expect(rows.map((r) => r.textContent?.trim())).toEqual(['Start', 'Brush', 'Move', 'Rename']);
    expect(rows.map((r) => r.getAttribute('aria-selected'))).toEqual(['false', 'false', 'true', 'false']);
    expect(rows.map((r) => r.classList.contains('dim'))).toEqual([false, false, false, true]);
  });

  it('jumps on click and on Enter', async () => {
    const { store, api } = makeStore(2);
    await store.openPhoto(1);
    render(HistoryPanel, { editor: store });
    const rows = screen.getAllByRole('option');
    await fireEvent.click(rows[0] as HTMLElement);
    await flush();
    expect(api.historyJump).toHaveBeenCalledWith(1, 0);
    await fireEvent.keyDown(screen.getAllByRole('option')[3] as HTMLElement, { key: 'Enter' });
    await flush();
    expect(api.historyJump).toHaveBeenLastCalledWith(1, 3);
  });
});
