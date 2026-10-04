/**
 * Frame transports: how the UI gets a rendered preview frame from the shell.
 *
 * Two transports give the same frame body (header + RGBA8 pixels):
 *
 * - `ipc` (default): the `preview_render` command returns a raw binary IPC
 *   response, which `invoke` gives as an `ArrayBuffer`.
 * - `scheme`: `fetch()` POST to the `preview` URI scheme of the shell.
 *
 * Both use the custom-protocol path of the web view. The transport
 * benchmark (`?bench=transport`) measures them in the real web view. To
 * change the default after a measurement, change `DEFAULT_TRANSPORT`, or
 * set `localStorage['expose.previewTransport']` to `ipc` or `scheme`.
 */
import { invoke } from '@tauri-apps/api/core';
import { usesHttpSchemeForm } from '../library/thumbnail';

/** Develop settings as the shell reads them (snake_case JSON; missing fields are defaults). */
export type DevelopSettingsJson = { readonly [key: string]: unknown };

/** The display color space of the frames of a session. */
export type DisplaySpace = 'srgb' | 'displayP3';

/** The result of a session open. */
export interface SessionInfo {
  readonly session: number;
  readonly width: number;
  readonly height: number;
}

/** The names of the transports. */
export type TransportName = 'ipc' | 'scheme';

/** The transport that the application uses when nothing else is set. */
export const DEFAULT_TRANSPORT: TransportName = 'ipc';

/** The localStorage key that overrides the default transport. */
export const TRANSPORT_STORAGE_KEY = 'expose.previewTransport';

/** Options of one frame request. */
export interface RenderOptions {
  /**
   * The target long edge of the frame in pixels. The shell clamps it to
   * the session size and to a minimum; the frame header has the real
   * size. `null` or absent: the session size.
   */
  readonly longEdge?: number | null;
}

/** Gets frame bodies for a session. */
export interface FrameTransport {
  readonly name: TransportName;
  /**
   * Requests one frame. Resolves with the frame body. Rejects with a
   * `CommandError` (for example kind `superseded` when a newer request
   * replaced this one in the shell) or another error.
   */
  render(session: number, settings: DevelopSettingsJson, options?: RenderOptions): Promise<ArrayBuffer>;
}

/** Opens a preview session for a photo (decode + GPU upload in the shell). */
export function openPreview(
  photoId: number,
  options: { maxLongEdge?: number; displaySpace?: DisplaySpace } = {},
): Promise<SessionInfo> {
  return invoke<SessionInfo>('preview_open', {
    photoId,
    maxLongEdge: options.maxLongEdge ?? null,
    displaySpace: options.displaySpace ?? null,
  });
}

/** Closes a preview session and frees its GPU memory. */
export function closePreview(session: number): Promise<void> {
  return invoke<void>('preview_close', { session });
}

/** The `ipc` transport: the `preview_render` command. */
export const ipcTransport: FrameTransport = {
  name: 'ipc',
  render(session, settings, options = {}) {
    return invoke<ArrayBuffer>('preview_render', { session, settings, longEdge: options.longEdge ?? null });
  },
};

/** The `preview` scheme base URL of this web view (the form depends on the platform). */
export function previewSchemeBase(httpForm: boolean): string {
  return httpForm ? 'http://preview.localhost' : 'preview://localhost';
}

/** The URL form of this web view. Computed once. */
const HTTP_FORM: boolean = typeof navigator !== 'undefined' && usesHttpSchemeForm(navigator.userAgent);

/** The URL of a frame of `session` in the given format, with an optional target long edge. */
export function previewFrameUrl(
  session: number,
  format: 'rgba8' | 'jpeg' | 'png' = 'rgba8',
  longEdge: number | null = null,
): string {
  const long = longEdge === null ? '' : `&long=${Math.round(longEdge)}`;
  return `${previewSchemeBase(HTTP_FORM)}/${encodeURIComponent(String(session))}?format=${format}${long}`;
}

/** A failed scheme request, with the HTTP status. */
export class SchemeError extends Error {
  override readonly name = 'SchemeError';
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

/**
 * Converts a scheme response status to the command error form, so that the
 * client handles both transports in one way.
 */
export function schemeErrorKind(status: number): 'superseded' | 'notFound' | 'invalidInput' | 'unavailable' | 'internal' {
  switch (status) {
    case 409:
      return 'superseded';
    case 404:
      return 'notFound';
    case 400:
      return 'invalidInput';
    case 503:
      return 'unavailable';
    default:
      return 'internal';
  }
}

/** The `scheme` transport: `fetch()` POST to the `preview` URI scheme. */
export const schemeTransport: FrameTransport = {
  name: 'scheme',
  async render(session, settings, options = {}) {
    const response = await fetch(previewFrameUrl(session, 'rgba8', options.longEdge ?? null), {
      method: 'POST',
      // `text/plain` keeps the request a CORS "simple request" (no preflight).
      headers: { 'Content-Type': 'text/plain' },
      body: JSON.stringify(settings),
    });
    if (!response.ok) {
      const message = await response.text();
      throw { kind: schemeErrorKind(response.status), message };
    }
    return response.arrayBuffer();
  },
};

/** Returns the transport with this name. */
export function transportByName(name: TransportName): FrameTransport {
  return name === 'scheme' ? schemeTransport : ipcTransport;
}

/**
 * Returns the transport to use: the localStorage override if it is valid,
 * else `DEFAULT_TRANSPORT`.
 */
export function selectTransport(storage: Pick<Storage, 'getItem'> | null = safeLocalStorage()): FrameTransport {
  const stored = storage?.getItem(TRANSPORT_STORAGE_KEY);
  const name: TransportName = stored === 'ipc' || stored === 'scheme' ? stored : DEFAULT_TRANSPORT;
  return transportByName(name);
}

/** localStorage, or null where it does not exist or access fails. */
function safeLocalStorage(): Storage | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    // Access can throw in a sandboxed context; then no override applies.
    return null;
  }
}
