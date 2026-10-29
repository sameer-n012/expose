// @vitest-environment happy-dom
import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, EditDto, EditorApi } from '../../lib/editor/api';
import { strokeStubs } from '../../lib/editor/fakeStrokeApi';
import { ShortcutRegistry } from '../../lib/shortcuts/registry';
import { editorActions } from '../../lib/editor/editorActions';
import { EditorStore } from '../../lib/stores/editor.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import DocumentTabs from './DocumentTabs.svelte';

const NONE = { transparency: false, pixels: false, position: false, all: false };

function doc(documentId: number, extra: Partial<DocumentInfo> = {}): DocumentInfo {
  return {
    documentId,
    name: `photo-${documentId}`,
    width: 100,
    height: 80,
    layers: [{ id: 1, name: 'Photo', kind: 'pixel', visible: true, opacity: 1, fillOpacity: 1, blendMode: 'normal', hasMask: false, maskEnabled: false, adjustment: null, isolated: null, children: null, locks: NONE }],
    canUndo: false,
    canRedo: false,
    revision: 0,
    dirty: false,
    savedPhotoId: null,
    sourcePhotoId: documentId,
    hasSelection: false,
    selectionVersion: 0,
    canReselect: false,
    ...extra,
  };
}

/** A store with two open documents: "photo-1" and "photo-2" (active). */
async function makeStore() {
  const api = {
    open: vi.fn(async (photoId: number) => doc(photoId)),
    newDocument: vi.fn(async () => doc(99)),
    render: vi.fn(async () => new ArrayBuffer(0)),
    edit: vi.fn(async (documentId: number, _edit: EditDto, _gesture: number | null) => doc(documentId, { revision: 1, dirty: true })),
    undo: vi.fn(async (documentId: number) => doc(documentId)),
    redo: vi.fn(async (documentId: number) => doc(documentId)),
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
  await store.openPhoto(2);
  return { store, api };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe('DocumentTabs', () => {
  it('shows a tab for each document with its name, and marks the active tab', async () => {
    const { store } = await makeStore();
    render(DocumentTabs, { editor: store });
    const tabs = screen.getAllByRole('tab');
    expect(tabs.map((t) => t.textContent?.trim())).toEqual(['photo-1', 'photo-2']);
    expect(tabs[0]?.getAttribute('aria-selected')).toBe('false');
    expect(tabs[1]?.getAttribute('aria-selected')).toBe('true');
    expect(screen.getByRole('tablist', { name: 'Open documents' })).toBeTruthy();
  });

  it('shows a dirty marker after an edit', async () => {
    const { store } = await makeStore();
    render(DocumentTabs, { editor: store });
    expect(screen.queryByTestId('tab-dirty')).toBeNull();
    store.setVisible(1, false);
    await flush();
    const marker = await screen.findByTestId('tab-dirty');
    expect(marker.closest('[data-tab-id]')?.textContent).toContain('photo-2');
    expect(screen.getByRole('tab', { name: /photo-2/ }).textContent).toContain('unsaved changes');
  });

  it('a click activates a tab', async () => {
    const { store } = await makeStore();
    render(DocumentTabs, { editor: store });
    await fireEvent.click(screen.getByRole('tab', { name: 'photo-1' }));
    expect(store.info?.documentId).toBe(1);
    expect(screen.getByRole('tab', { name: 'photo-1' }).getAttribute('aria-selected')).toBe('true');
  });

  it('the close button closes the tab and its document', async () => {
    const { store, api } = await makeStore();
    render(DocumentTabs, { editor: store });
    await fireEvent.click(screen.getByRole('button', { name: 'Close photo-1' }));
    await flush();
    expect(api.close).toHaveBeenCalledWith(1);
    expect(screen.getAllByRole('tab').map((t) => t.textContent?.trim())).toEqual(['photo-2']);
  });

  it('a middle click closes the tab', async () => {
    const { store, api } = await makeStore();
    render(DocumentTabs, { editor: store });
    await fireEvent(screen.getByRole('tab', { name: 'photo-2' }), new MouseEvent('auxclick', { button: 1, bubbles: true }));
    await flush();
    expect(api.close).toHaveBeenCalledWith(2);
    expect(store.info?.documentId).toBe(1);
  });

  it('the arrow keys move to the next and the previous tab', async () => {
    const { store } = await makeStore();
    render(DocumentTabs, { editor: store });
    await fireEvent.keyDown(screen.getByRole('tab', { name: 'photo-2' }), { key: 'ArrowRight' });
    expect(store.info?.documentId).toBe(1);
    await fireEvent.keyDown(screen.getByRole('tab', { name: 'photo-1' }), { key: 'ArrowLeft' });
    expect(store.info?.documentId).toBe(2);
  });

  it('Ctrl+Tab and Mod+W run through the editor shortcuts', async () => {
    const { store, api } = await makeStore();
    const registry = new ShortcutRegistry('mac');
    for (const action of editorActions(store)) {
      registry.register(action);
    }
    expect(registry.handleKeyDown(new KeyboardEvent('keydown', { key: 'Tab', ctrlKey: true }), 'editor')).toBe(true);
    expect(store.info?.documentId).toBe(1);
    expect(registry.handleKeyDown(new KeyboardEvent('keydown', { key: 'Tab', ctrlKey: true, shiftKey: true }), 'editor')).toBe(true);
    expect(store.info?.documentId).toBe(2);
    expect(registry.handleKeyDown(new KeyboardEvent('keydown', { key: 'w', metaKey: true }), 'editor')).toBe(true);
    await flush();
    expect(api.close).toHaveBeenCalledWith(2);
    expect(store.tabs.length).toBe(1);
  });
});
