import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createAppState, type AppState } from '../../lib/appState';
import type { DocumentInfo } from '../../lib/editor/api';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import { FakeCatalog, mockElementSize } from '../../test/fakeCatalog';
import AppShell from './AppShell.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

let app: AppState;
let toasts: ToastStore;
let restoreSize: () => void;
let editorCalls: Array<{ command: string; args: Record<string, unknown> }>;

const info: DocumentInfo = {
  documentId: 11,
  name: 'photo',
  width: 100,
  height: 80,
  layers: [{ id: 1, name: 'Photo', kind: 'pixel', visible: true, opacity: 1, fillOpacity: 1, blendMode: 'normal', hasMask: false, maskEnabled: false, maskVersion: null, adjustment: null, isolated: null, children: null, locks: { transparency: false, pixels: false, position: false, all: false } }],
  canUndo: false,
  canRedo: false,
  revision: 0,
  dirty: false,
  savedPhotoId: null,
  sourcePhotoId: 1,
  hasSelection: false,
  selectionVersion: 0,
  canReselect: false,
};

beforeEach(() => {
  const catalog = new FakeCatalog(5);
  editorCalls = [];
  // Each open gives a new document: 11 "photo", 12 "photo-12", ...
  let nextDocument = 11;
  vi.mocked(invoke).mockImplementation((command: string, args?: unknown) => {
    if (command.startsWith('editor_')) {
      const params = (args ?? {}) as Record<string, unknown>;
      editorCalls.push({ command, args: params });
      const documentId = typeof params['documentId'] === 'number' ? params['documentId'] : 11;
      const name = documentId === 11 ? 'photo' : `photo-${documentId}`;
      // The render result is not a frame: the test does not need pixels.
      if (command === 'editor_close') {
        return Promise.resolve(null);
      }
      if (command === 'editor_render') {
        return Promise.resolve(new ArrayBuffer(0));
      }
      if (command === 'editor_edit') {
        return Promise.resolve({ ...info, documentId, name, revision: 1, dirty: true });
      }
      if (command === 'editor_save') {
        const saved = { ...info, documentId, name, revision: 1, dirty: false, savedPhotoId: 99 };
        return Promise.resolve({ photoId: 99, documentPath: '/p/a-edit.exdoc', imagePath: '/p/a-edit.tif', created: true, info: saved });
      }
      const opened = nextDocument++;
      return Promise.resolve({ ...info, documentId: opened, name: opened === 11 ? 'photo' : `photo-${opened}` });
    }
    return catalog.invoke(command, args as never);
  });
  restoreSize = mockElementSize(1000, 600);
  toasts = new ToastStore();
  app = createAppState({ platform: 'mac', toasts });
});

afterEach(() => {
  app.dispose();
  toasts.clear();
  restoreSize();
});

describe('Edit in Editor', () => {
  it('Mod+E opens the focused photo in the Editor module', async () => {
    render(AppShell, { modules: app.modules, layout: app.layout, library: app.library, registry: app.registry, toasts });
    await screen.findByRole('toolbar', { name: 'Library toolbar' });
    await waitFor(() => expect(app.library.photoAt(0)).toBeDefined());
    app.library.click(0, { toggle: false, range: false });
    await fireEvent.keyDown(window, { key: 'e', metaKey: true });
    expect(app.modules.active).toBe('editor');
    await waitFor(() => expect(editorCalls.some((c) => c.command === 'editor_open')).toBe(true));
    const open = editorCalls.find((c) => c.command === 'editor_open');
    expect(open?.args['photoId']).toBe(app.library.photoAt(0)?.id);
    expect(await screen.findByRole('button', { name: 'Photo' })).toBeTruthy();
  });

  async function openEditedDocument(): Promise<void> {
    render(AppShell, { modules: app.modules, layout: app.layout, library: app.library, registry: app.registry, toasts });
    await screen.findByRole('toolbar', { name: 'Library toolbar' });
    await waitFor(() => expect(app.library.photoAt(0)).toBeDefined());
    app.library.click(0, { toggle: false, range: false });
    await fireEvent.keyDown(window, { key: 'e', metaKey: true });
    await fireEvent.click(await screen.findByRole('button', { name: 'Hide Photo' }));
    await waitFor(() => expect(screen.getByTestId('editor-save-state').textContent).toContain('Unsaved changes'));
  }

  it('Mod+S saves the document in the Editor module', async () => {
    await openEditedDocument();
    await fireEvent.keyDown(window, { key: 's', metaKey: true });
    await waitFor(() => expect(editorCalls.some((c) => c.command === 'editor_save')).toBe(true));
    expect(editorCalls.find((c) => c.command === 'editor_save')?.args['documentId']).toBe(11);
    await waitFor(() => expect(screen.getByTestId('editor-save-state').textContent?.trim()).toBe('Saved'));
    expect(toasts.items.some((t) => t.message === 'Saved to library: a-edit.tif')).toBe(true);
  });

  it('another photo opens in a second document, and the dirty document stays open', async () => {
    await openEditedDocument();
    const opens = () => editorCalls.filter((c) => c.command === 'editor_open').length;
    expect(opens()).toBe(1);
    app.library.click(1, { toggle: false, range: false });
    await fireEvent.keyDown(window, { key: 'e', metaKey: true });
    await waitFor(() => expect(opens()).toBe(2));
    expect(screen.queryByRole('alertdialog')).toBeNull();
    expect(editorCalls.some((c) => c.command === 'editor_close')).toBe(false);
    // The tab strip shows both documents; the new one is active.
    await waitFor(() => expect(screen.getAllByRole('tab').length).toBe(2));
    expect(screen.getByRole('tab', { name: /^photo-12/ }).getAttribute('aria-selected')).toBe('true');
    // Close the dirty tab: the question shows; Cancel keeps the tab.
    await fireEvent.click(screen.getByRole('button', { name: 'Close photo' }));
    const dialog = await screen.findByRole('alertdialog');
    expect(dialog.textContent).toContain('Save the changes?');
    // The tab that asks is active (the open dialog hides the strip from the role queries).
    expect(document.querySelector('[role="tab"][aria-selected="true"] .title')?.textContent).toBe('photo');
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).toBeNull());
    expect(screen.getAllByRole('tab').length).toBe(2);
    // Again with Mod+W: Don't save closes the document.
    await fireEvent.keyDown(window, { key: 'w', metaKey: true });
    await screen.findByRole('alertdialog');
    await fireEvent.click(screen.getByRole('button', { name: "Don't save" }));
    await waitFor(() => expect(editorCalls.some((c) => c.command === 'editor_close' && c.args['documentId'] === 11)).toBe(true));
    expect(screen.getAllByRole('tab').map((t) => t.textContent?.trim())).toEqual(['photo-12']);
    expect(editorCalls.some((c) => c.command === 'editor_save')).toBe(false);
  });

  it('the Editor module opens the selected photo when it has no document', async () => {
    render(AppShell, { modules: app.modules, layout: app.layout, library: app.library, registry: app.registry, toasts });
    await screen.findByRole('toolbar', { name: 'Library toolbar' });
    await waitFor(() => expect(app.library.photoAt(0)).toBeDefined());
    app.library.click(1, { toggle: false, range: false });
    await fireEvent.keyDown(window, { key: 'e' });
    await waitFor(() => expect(editorCalls.some((c) => c.command === 'editor_open')).toBe(true));
  });
});
