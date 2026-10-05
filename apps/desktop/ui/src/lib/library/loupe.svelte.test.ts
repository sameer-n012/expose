import { describe, expect, it, vi } from 'vitest';
import type { DevelopDocument } from '../develop/settings';
import { LoupeStore, type LoupeApi } from './loupe.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const DOC = { version: 1 } as unknown as DevelopDocument;

function setup() {
  const calls: string[] = [];
  const api = {
    getSettings: vi.fn(async (id: number) => {
      calls.push(`settings ${id}`);
      return DOC;
    }),
    openPreview: vi.fn(async (id: number) => {
      calls.push(`open ${id}`);
      return { session: id * 10, width: 4, height: 3 } as never;
    }),
    closePreview: vi.fn(async (session: number) => {
      calls.push(`close ${session}`);
    }),
  } satisfies LoupeApi;
  return { api, calls, store: new LoupeStore(api) };
}

describe('LoupeStore', () => {
  it('loads the settings and opens a session', async () => {
    const { store } = setup();
    await store.show(3);
    expect(store.status).toBe('ready');
    expect(store.session?.session).toBe(30);
    expect(store.settings).toBe(DOC);
  });

  it('closes the old session before it opens the next one', async () => {
    const { store, calls } = setup();
    await store.show(1);
    await store.show(2);
    expect(calls.indexOf('close 10')).toBeLessThan(calls.indexOf('open 2'));
    expect(store.session?.session).toBe(20);
  });

  it('closes the session when the loupe closes', async () => {
    const { store, api } = setup();
    await store.show(1);
    await store.show(null);
    expect(api.closePreview).toHaveBeenCalledWith(10);
    expect(store.status).toBe('empty');
    expect(store.session).toBeNull();
  });

  it('logs a failed close', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const { store, api } = setup();
    const failure = new Error('gone');
    api.closePreview.mockRejectedValueOnce(failure);
    await store.show(1);
    store.dispose();
    await vi.waitFor(() =>
      expect(warn).toHaveBeenCalledWith('loupe: preview session could not be closed', 10, failure),
    );
    warn.mockRestore();
  });

  it('closes the session on dispose', async () => {
    const { store, api } = setup();
    await store.show(1);
    store.dispose();
    expect(api.closePreview).toHaveBeenCalledWith(10);
  });

  it('ignores a late result for a photo that is no longer current', async () => {
    const { store, api } = setup();
    let release: (value: { session: number; width: number; height: number }) => void = () => undefined;
    api.openPreview.mockImplementationOnce(
      () => new Promise((resolve) => (release = resolve as unknown as typeof release)),
    );
    const first = store.show(1);
    const second = store.show(2);
    await second;
    release({ session: 10, width: 4, height: 3 });
    await first;
    expect(store.photoId).toBe(2);
    expect(store.session?.session).toBe(20);
    // The session of photo 1 was freed.
    expect(api.closePreview).toHaveBeenCalledWith(10);
  });

  it('leaves one session open after quick stepping', async () => {
    const { store, api } = setup();
    await Promise.all([store.show(1), store.show(2), store.show(3)]);
    const opened = api.openPreview.mock.calls.length;
    expect(opened).toBe(3);
    expect(api.closePreview).toHaveBeenCalledTimes(2);
    expect(store.session?.session).toBe(30);
  });

  it('reports an error and frees the session when the settings fail', async () => {
    const { store, api } = setup();
    api.getSettings.mockRejectedValueOnce({ kind: 'internal', message: 'bad settings' });
    await store.show(1);
    expect(store.status).toBe('error');
    expect(store.error).toBe('bad settings');
    expect(api.closePreview).toHaveBeenCalledWith(10);
  });

  it('reports an error when the preview does not open', async () => {
    const { store, api } = setup();
    api.openPreview.mockRejectedValueOnce(new Error('no decode'));
    await store.show(1);
    expect(store.status).toBe('error');
    expect(store.session).toBeNull();
  });
});
