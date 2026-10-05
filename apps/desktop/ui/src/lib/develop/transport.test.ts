import { invoke } from '@tauri-apps/api/core';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  DEFAULT_TRANSPORT,
  TRANSPORT_STORAGE_KEY,
  closePreview,
  ipcTransport,
  openPreview,
  previewSchemeBase,
  schemeErrorKind,
  schemeTransport,
  selectTransport,
} from './transport';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

afterEach(() => {
  vi.mocked(invoke).mockReset();
  vi.unstubAllGlobals();
});

const storage = (value: string | null) => ({ getItem: (key: string) => (key === TRANSPORT_STORAGE_KEY ? value : null) });

describe('selectTransport', () => {
  it('uses the default without an override', () => {
    expect(selectTransport(null).name).toBe(DEFAULT_TRANSPORT);
    expect(selectTransport(storage(null)).name).toBe(DEFAULT_TRANSPORT);
    expect(selectTransport(storage('carrier pigeon')).name).toBe(DEFAULT_TRANSPORT);
  });

  it('uses a valid override', () => {
    expect(selectTransport(storage('scheme'))).toBe(schemeTransport);
    expect(selectTransport(storage('ipc'))).toBe(ipcTransport);
  });
});

describe('commands', () => {
  it('ipc transport calls preview_render with the session and settings', async () => {
    const body = new ArrayBuffer(8);
    vi.mocked(invoke).mockResolvedValue(body);
    await expect(ipcTransport.render(3, { tone: { exposure: 1 } })).resolves.toBe(body);
    expect(invoke).toHaveBeenCalledWith('preview_render', {
      session: 3,
      settings: { tone: { exposure: 1 } },
      longEdge: null,
      applyGeometry: true,
    });
    await ipcTransport.render(3, {}, { longEdge: 1280 });
    expect(invoke).toHaveBeenLastCalledWith('preview_render', {
      session: 3,
      settings: {},
      longEdge: 1280,
      applyGeometry: true,
    });
    await ipcTransport.render(3, {}, { applyGeometry: false });
    expect(invoke).toHaveBeenLastCalledWith('preview_render', {
      session: 3,
      settings: {},
      longEdge: null,
      applyGeometry: false,
    });
  });

  it('open and close send the arguments in camelCase', async () => {
    vi.mocked(invoke).mockResolvedValue({ session: 1, width: 2560, height: 1707 });
    await openPreview(42, { displaySpace: 'displayP3' });
    expect(invoke).toHaveBeenCalledWith('preview_open', { photoId: 42, maxLongEdge: null, displaySpace: 'displayP3' });
    vi.mocked(invoke).mockResolvedValue(undefined);
    await closePreview(1);
    expect(invoke).toHaveBeenCalledWith('preview_close', { session: 1 });
  });
});

describe('scheme transport', () => {
  it('uses the URL form of the platform', () => {
    expect(previewSchemeBase(false)).toBe('preview://localhost');
    expect(previewSchemeBase(true)).toBe('http://preview.localhost');
  });

  it('posts the settings as text and returns the body', async () => {
    const body = new ArrayBuffer(4);
    const fetchMock = vi.fn().mockResolvedValue(new Response(body, { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);
    const result = await schemeTransport.render(7, { tone: { exposure: 0.5 } });
    expect(result.byteLength).toBe(4);
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toMatch(/\/7\?format=rgba8$/);
    expect(init.method).toBe('POST');
    expect(init.headers).toEqual({ 'Content-Type': 'text/plain' });
    expect(JSON.parse(String(init.body))).toEqual({ tone: { exposure: 0.5 } });

    fetchMock.mockResolvedValue(new Response(new ArrayBuffer(4), { status: 200 }));
    await schemeTransport.render(7, {}, { longEdge: 1280 });
    expect(fetchMock.mock.calls[1]?.[0]).toMatch(/\/7\?format=rgba8&long=1280$/);
  });

  it('turns an HTTP error into a command error', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('replaced', { status: 409 })));
    await expect(schemeTransport.render(1, {})).rejects.toEqual({ kind: 'superseded', message: 'replaced' });
  });

  it('maps status codes to error kinds', () => {
    expect(schemeErrorKind(409)).toBe('superseded');
    expect(schemeErrorKind(404)).toBe('notFound');
    expect(schemeErrorKind(400)).toBe('invalidInput');
    expect(schemeErrorKind(503)).toBe('unavailable');
    expect(schemeErrorKind(500)).toBe('internal');
  });
});
