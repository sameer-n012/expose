/**
 * State of the Develop module: the active photo, its settings, the preview
 * session, and the saving of the settings.
 *
 * - The active photo is the focused photo of the library, or else the first
 *   selected photo. It is active only while the Develop module is shown
 *   (`target`). `activate` switches the photo.
 * - An edit changes `settings` at once. The preview shows it at once (the
 *   canvas sends each new value to the preview client). The catalog gets it
 *   after a quiet time (`SAVE_DELAY_MS`), and at once on `flush`.
 * - `flush` runs when the photo changes, when the module changes, and when
 *   the view is destroyed. Saves run one after the other.
 * - A late answer for an old photo is dropped (generation counter).
 */
import type { PhotoId } from '../catalog';
import { getDevelopSettings, isCommandError, setDevelopSettings } from '../catalog';
import { identityCurve, type CurveChannel, type CurvePoint } from '../develop/curve';
import type { Histogram } from '../develop/histogram';
import {
  RANGES,
  SHARPEN_RANGES,
  withCurve,
  resetBasic,
  resetSharpenKey,
  resetSharpening,
  resetSlider,
  sameDocument,
  withSharpening,
  withSlider,
  withWhiteBalanceMode,
  type DevelopDocument,
  type SharpenKey,
  type SliderKey,
} from '../develop/settings';
import { closePreview, openPreview, type SessionInfo } from '../develop/transport';
import { bumpThumbnailRevision } from '../library/thumbnailRevisions.svelte';
import type { LibraryStore } from './library.svelte';
import type { ModuleStore } from './modules.svelte';
import { describeError, type ToastStore } from './toasts.svelte';

/** The quiet time after the last edit before the settings are saved. */
export const SAVE_DELAY_MS = 300;

/** The backend calls of the store. Tests replace them. */
export interface DevelopApi {
  getSettings(photoId: PhotoId): Promise<DevelopDocument>;
  setSettings(photoId: PhotoId, settings: DevelopDocument): Promise<DevelopDocument>;
  openPreview(photoId: PhotoId): Promise<SessionInfo>;
  closePreview(session: number): Promise<void>;
}

const REAL_API: DevelopApi = {
  getSettings: getDevelopSettings,
  setSettings: setDevelopSettings,
  openPreview: (photoId) => openPreview(photoId),
  closePreview,
};

export interface DevelopStoreOptions {
  readonly library: Pick<LibraryStore, 'focusIndex' | 'photoAt' | 'selectedIds'>;
  readonly modules: Pick<ModuleStore, 'active'>;
  readonly toasts: ToastStore;
  readonly api?: DevelopApi;
  readonly saveDelayMs?: number;
}

/** What the Develop view shows. */
export type DevelopStatus = 'empty' | 'loading' | 'ready' | 'error';

export class DevelopStore {
  /** The photo that the view shows, or null. */
  photoId = $state<PhotoId | null>(null);
  /** The settings of the photo, or null while they load. */
  settings = $state.raw<DevelopDocument | null>(null);
  /** The preview session of the photo, or null. */
  session = $state.raw<SessionInfo | null>(null);
  /** The histogram of the newest displayed frame, or null (no frame yet). */
  histogram = $state.raw<Histogram | null>(null);
  status = $state<DevelopStatus>('empty');
  /** The reason of the `error` status. */
  loadError = $state<string | null>(null);

  readonly #library: DevelopStoreOptions['library'];
  readonly #modules: DevelopStoreOptions['modules'];
  readonly #toasts: ToastStore;
  readonly #api: DevelopApi;
  readonly #delay: number;

  #generation = 0;
  #timer: ReturnType<typeof setTimeout> | null = null;
  /** The settings that wait for a save. */
  #dirty: { photoId: PhotoId; settings: DevelopDocument } | null = null;
  /** The last save; the next save waits for it. */
  #saving: Promise<void> = Promise.resolve();

  constructor(options: DevelopStoreOptions) {
    this.#library = options.library;
    this.#modules = options.modules;
    this.#toasts = options.toasts;
    this.#api = options.api ?? REAL_API;
    this.#delay = options.saveDelayMs ?? SAVE_DELAY_MS;
  }

  /**
   * The photo that Develop must show: the focused or first selected photo
   * of the library while the Develop module is active, else null.
   */
  get target(): PhotoId | null {
    if (this.#modules.active !== 'develop') {
      return null;
    }
    const focus = this.#library.focusIndex;
    const focused = focus === null ? undefined : this.#library.photoAt(focus);
    return focused?.id ?? this.#library.selectedIds[0] ?? null;
  }

  /**
   * Shows another photo (or none). It saves the edits of the old photo,
   * closes its session, then loads the settings and opens the session of
   * the new photo.
   */
  async activate(photoId: PhotoId | null): Promise<void> {
    if (photoId === this.photoId) {
      return;
    }
    const generation = ++this.#generation;
    const flushed = this.flush();
    this.#closeSession();
    this.photoId = photoId;
    this.settings = null;
    this.loadError = null;
    if (photoId === null) {
      this.status = 'empty';
      await flushed;
      return;
    }
    this.status = 'loading';
    const [settings, session] = await Promise.allSettled([
      // `async` wrappers: a synchronous throw of an API call becomes a rejection.
      (async () => this.#api.getSettings(photoId))(),
      (async () => this.#api.openPreview(photoId))(),
    ]);
    if (generation !== this.#generation) {
      // The user chose another photo. Free the session of this one.
      if (session.status === 'fulfilled') {
        void this.#closeQuietly(session.value.session);
      }
      return;
    }
    if (settings.status === 'rejected') {
      if (session.status === 'fulfilled') {
        void this.#closeQuietly(session.value.session);
      }
      this.#fail(settings.reason);
      return;
    }
    this.settings = settings.value;
    if (session.status === 'fulfilled') {
      this.session = session.value;
    } else {
      // The preview is not available (for example the file does not decode).
      // The settings still load, so the sliders work; the user sees the reason.
      this.#toasts.error('The preview could not be opened.', session.reason);
    }
    this.status = 'ready';
    await flushed;
  }

  /** Sets one slider. A white balance slider switches the mode to custom. */
  setSlider(key: SliderKey, value: number): void {
    const current = this.settings;
    if (current === null || !Number.isFinite(value)) {
      return;
    }
    const range = key === 'temperature' || key === 'tint' || key === 'exposure' ? RANGES[key] : RANGES.tone;
    const clamped = Math.min(range.max, Math.max(range.min, value));
    this.#edit(withSlider(current, key, clamped));
  }

  /** Sets the white balance mode. */
  setWhiteBalanceMode(mode: 'as_shot' | 'custom'): void {
    if (this.settings !== null) {
      this.#edit(withWhiteBalanceMode(this.settings, mode));
    }
  }

  /** Sets one slider to its identity value. */
  resetSlider(key: SliderKey): void {
    if (this.settings !== null) {
      this.#edit(resetSlider(this.settings, key));
    }
  }

  /** Sets the points of one tone curve channel. */
  setCurve(channel: CurveChannel, points: readonly CurvePoint[]): void {
    if (this.settings !== null) {
      this.#edit(withCurve(this.settings, channel, points));
    }
  }

  /** Sets one tone curve channel to the identity curve. */
  resetCurve(channel: CurveChannel): void {
    this.setCurve(channel, identityCurve());
  }

  /** Sets one sharpening value (clamped to its range). */
  setSharpening(key: SharpenKey, value: number): void {
    if (this.settings === null || !Number.isFinite(value)) {
      return;
    }
    const range = SHARPEN_RANGES[key];
    this.#edit(withSharpening(this.settings, key, Math.min(range.max, Math.max(range.min, value))));
  }

  /** Sets one sharpening value to its default. */
  resetSharpenKey(key: SharpenKey): void {
    if (this.settings !== null) {
      this.#edit(resetSharpenKey(this.settings, key));
    }
  }

  /** Sets all four sharpening values to their defaults. */
  resetSharpening(): void {
    if (this.settings !== null) {
      this.#edit(resetSharpening(this.settings));
    }
  }

  /** Sets all Basic panel values to identity. */
  resetAll(): void {
    if (this.settings !== null) {
      this.#edit(resetBasic(this.settings));
    }
  }

  /** Saves the waiting edits now. Resolves when all saves are done. */
  flush(): Promise<void> {
    if (this.#timer !== null) {
      clearTimeout(this.#timer);
      this.#timer = null;
    }
    const dirty = this.#dirty;
    this.#dirty = null;
    if (dirty !== null) {
      this.#saving = this.#saving.then(() => this.#save(dirty.photoId, dirty.settings));
    }
    return this.#saving;
  }

  /** Saves the edits and frees the session. Call when the view is destroyed. */
  async dispose(): Promise<void> {
    this.#generation++;
    const flushed = this.flush();
    this.#closeSession();
    this.photoId = null;
    this.settings = null;
    this.status = 'empty';
    await flushed;
  }

  #edit(next: DevelopDocument): void {
    const photoId = this.photoId;
    if (photoId === null || this.settings === null || sameDocument(this.settings, next)) {
      return;
    }
    this.settings = next;
    this.#dirty = { photoId, settings: next };
    if (this.#timer !== null) {
      clearTimeout(this.#timer);
    }
    this.#timer = setTimeout(() => {
      this.#timer = null;
      void this.flush();
    }, this.#delay);
  }

  async #save(photoId: PhotoId, settings: DevelopDocument): Promise<void> {
    try {
      await this.#api.setSettings(photoId, settings);
      bumpThumbnailRevision(photoId);
    } catch (error: unknown) {
      this.#toasts.error('The develop settings could not be saved.', error);
    }
  }

  #closeSession(): void {
    const session = this.session;
    this.session = null;
    this.histogram = null;
    if (session !== null) {
      void this.#closeQuietly(session.session);
    }
  }

  async #closeQuietly(session: number): Promise<void> {
    try {
      await this.#api.closePreview(session);
    } catch (error: unknown) {
      this.#toasts.error('The preview could not be closed.', error);
    }
  }

  #fail(reason: unknown): void {
    this.status = 'error';
    this.loadError = isCommandError(reason) ? reason.message : describeError(reason);
  }
}
