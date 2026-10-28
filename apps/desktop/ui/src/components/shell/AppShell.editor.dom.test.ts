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
  width: 100,
  height: 80,
  layers: [{ id: 1, name: 'Photo', kind: 'pixel', visible: true, opacity: 1, fillOpacity: 1, blendMode: 'normal', hasMask: false, maskEnabled: false, adjustment: null, isolated: null, children: null, locks: { transparency: false, pixels: false, position: false, all: false } }],
  canUndo: false,
  canRedo: false,
  revision: 0,
  dirty: false,
  savedPhotoId: null,
  sourcePhotoId: 1,
};

beforeEach(() => {
  const catalog = new FakeCatalog(5);
  editorCalls = [];
  vi.mocked(invoke).mockImplementation((command: string, args?: unknown) => {
    if (command.startsWith('editor_')) {
      editorCalls.push({ command, args: (args ?? {}) as Record<string, unknown> });
      // The render result is not a frame: the test does not need pixels.
      if (command === 'editor_close') {
        return Promise.resolve(null);
      }
      if (command === 'editor_render') {
        return Promise.resolve(new ArrayBuffer(0));
      }
      if (command === 'editor_edit') {
        return Promise.resolve({ ...info, revision: 1, dirty: true });
      }
      if (command === 'editor_save') {
        const saved = { ...info, revision: 1, dirty: false, savedPhotoId: 99 };
        return Promise.resolve({ photoId: 99, documentPath: '/p/a-edit.exdoc', imagePath: '/p/a-edit.tif', created: true, info: saved });
      }
      return Promise.resolve(info);
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

  it('asks before another photo replaces a document with unsaved changes', async () => {
    await openEditedDocument();
    const opens = () => editorCalls.filter((c) => c.command === 'editor_open').length;
    expect(opens()).toBe(1);
    // Another photo: the question shows; Cancel keeps the document.
    app.library.click(1, { toggle: false, range: false });
    await fireEvent.keyDown(window, { key: 'e', metaKey: true });
    const dialog = await screen.findByRole('alertdialog');
    expect(dialog.textContent).toContain('Save the changes?');
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).toBeNull());
    expect(opens()).toBe(1);
    // Again: Don't save opens the other photo.
    await fireEvent.keyDown(window, { key: 'e', metaKey: true });
    await screen.findByRole('alertdialog');
    await fireEvent.click(screen.getByRole('button', { name: "Don't save" }));
    await waitFor(() => expect(opens()).toBe(2));
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
