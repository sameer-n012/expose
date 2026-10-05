/**
 * State of the Library loupe: the preview session and the saved develop
 * settings of one photo. The loupe only reads. It never saves settings.
 *
 * - `show` switches the photo. It closes the old session first, then loads
 *   the settings and opens the session of the new photo.
 * - A late answer for an old photo is dropped (generation counter), and the
 *   session that came with it is closed. So fast stepping through photos
 *   leaves no session open.
 * - `show(null)` (or `dispose`) closes the session. Call it when the loupe
 *   closes or the module changes.
 *
 * `DevelopStore` is not reused: it also owns saving and the active-module
 * rule, and the loupe needs neither.
 */
import type { PhotoId } from '../catalog';
import { getDevelopSettings, isCommandError } from '../catalog';
import type { DevelopDocument } from '../develop/settings';
import { closePreview, openPreview, type SessionInfo } from '../develop/transport';
import { describeError } from '../stores/toasts.svelte';

/** The backend calls of the loupe. Tests replace them. */
export interface LoupeApi {
  getSettings(photoId: PhotoId): Promise<DevelopDocument>;
  openPreview(photoId: PhotoId): Promise<SessionInfo>;
  closePreview(session: number): Promise<void>;
}

const REAL_API: LoupeApi = {
  getSettings: getDevelopSettings,
  openPreview: (photoId) => openPreview(photoId),
  closePreview,
};

export type LoupeStatus = 'empty' | 'loading' | 'ready' | 'error';

export class LoupeStore {
  /** The photo of the loupe, or null. */
  photoId = $state<PhotoId | null>(null);
  /** The saved settings, or null until they load. */
  settings = $state.raw<DevelopDocument | null>(null);
  /** The preview session, or null. */
  session = $state.raw<SessionInfo | null>(null);
  status = $state<LoupeStatus>('empty');
  /** The reason of the `error` status. */
  error = $state<string | null>(null);

  readonly #api: LoupeApi;
  #generation = 0;

  constructor(api: LoupeApi = REAL_API) {
    this.#api = api;
  }

  /** Shows another photo (or none). Resolves when the photo is ready or failed. */
  async show(photoId: PhotoId | null): Promise<void> {
    if (photoId === this.photoId) {
      return;
    }
    const generation = ++this.#generation;
    this.#closeSession();
    this.photoId = photoId;
    this.settings = null;
    this.error = null;
    if (photoId === null) {
      this.status = 'empty';
      return;
    }
    this.status = 'loading';
    const [settings, session] = await Promise.allSettled([
      // `async` wrappers: a synchronous throw of an API call becomes a rejection.
      (async () => this.#api.getSettings(photoId))(),
      (async () => this.#api.openPreview(photoId))(),
    ]);
    if (generation !== this.#generation) {
      if (session.status === 'fulfilled') {
        this.#closeQuietly(session.value.session);
      }
      return;
    }
    if (settings.status === 'rejected' || session.status === 'rejected') {
      if (session.status === 'fulfilled') {
        this.#closeQuietly(session.value.session);
      }
      const reason = settings.status === 'rejected' ? settings.reason : (session as PromiseRejectedResult).reason;
      this.status = 'error';
      this.error = isCommandError(reason) ? reason.message : describeError(reason);
      return;
    }
    this.settings = settings.value;
    this.session = session.value;
    this.status = 'ready';
  }

  /** Frees the session. Call when the view is destroyed. */
  dispose(): void {
    this.#generation++;
    this.#closeSession();
    this.photoId = null;
    this.settings = null;
    this.status = 'empty';
  }

  #closeSession(): void {
    const session = this.session;
    this.session = null;
    if (session !== null) {
      this.#closeQuietly(session.session);
    }
  }

  /** Closes a session. A failed close has no effect for the user; the shell frees old sessions. */
  #closeQuietly(session: number): void {
    void (async () => this.#api.closePreview(session))().catch(() => undefined);
  }
}
