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
  layers: [{ id: 1, name: 'Photo', kind: 'pixel', visible: true, opacity: 1, fillOpacity: 1, blendMode: 'normal', hasMask: false, adjustment: null, isolated: null, children: null }],
  canUndo: false,
  canRedo: false,
  revision: 0,
};

beforeEach(() => {
  const catalog = new FakeCatalog(5);
  editorCalls = [];
  vi.mocked(invoke).mockImplementation((command: string, args?: unknown) => {
    if (command.startsWith('editor_')) {
      editorCalls.push({ command, args: (args ?? {}) as Record<string, unknown> });
      // The render result is not a frame: the test does not need pixels.
      return Promise.resolve(command === 'editor_close' ? null : command === 'editor_render' ? new ArrayBuffer(0) : info);
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

  it('the Editor module opens the selected photo when it has no document', async () => {
    render(AppShell, { modules: app.modules, layout: app.layout, library: app.library, registry: app.registry, toasts });
    await screen.findByRole('toolbar', { name: 'Library toolbar' });
    await waitFor(() => expect(app.library.photoAt(0)).toBeDefined());
    app.library.click(1, { toggle: false, range: false });
    await fireEvent.keyDown(window, { key: 'e' });
    await waitFor(() => expect(editorCalls.some((c) => c.command === 'editor_open')).toBe(true));
  });
});
