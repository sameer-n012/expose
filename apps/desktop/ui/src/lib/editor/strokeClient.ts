/**
 * The transport of one stroke to the shell.
 *
 * - `strokeBegin` runs at once. Points that come before it ends wait in a queue.
 * - At most one `strokePoints` call is in flight. The points that come
 *   while a call runs go in the next call. A call has at most 3000 points.
 * - `end` waits for all points, then calls `strokeEnd`.
 * - `cancel` drops the waiting points and calls `strokeCancel`.
 * - The shell gives `invalidInput` for a stroke id that is no longer
 *   current (another edit ended the stroke). This client ignores that error.
 *
 * Points are a flat list [x, y, pressure, ...] in canvas pixels.
 */
import type { BrushSettingsDto, DocumentInfo, EditorApi, StrokeTargetDto } from './api';

/** The most points in one `strokePoints` call. */
export const MAX_POINTS_PER_CALL = 3000;
const VALUES_PER_POINT = 3;

export interface StrokeClientOptions {
  readonly api: Pick<EditorApi, 'strokeBegin' | 'strokePoints' | 'strokeEnd' | 'strokeCancel'>;
  readonly documentId: number;
  readonly layerId: number;
  readonly target: StrokeTargetDto;
  readonly brush: BrushSettingsDto;
  /** Called after the shell took a batch of points. The view asks for a new frame. */
  readonly onProgress: () => void;
  /** Called when `strokeBegin` fails. The gesture is dropped. */
  readonly onBeginError: (error: unknown) => void;
  /** Called for an error of a points call, or of end and cancel. */
  readonly onError: (error: unknown) => void;
}

/** True if the error is the `invalidInput` error of the shell. */
export function isInvalidInput(error: unknown): boolean {
  return typeof error === 'object' && error !== null && 'kind' in error && (error as { kind: unknown }).kind === 'invalidInput';
}

export class StrokeClient {
  readonly #options: StrokeClientOptions;
  readonly #begin: Promise<void>;
  #id: number | null = null;
  #failed = false;
  #closed = false;
  #queue: number[] = [];
  #inFlight: Promise<void> | null = null;

  constructor(options: StrokeClientOptions) {
    this.#options = options;
    this.#begin = this.#start();
  }

  /** True if the stroke can take points. */
  get active(): boolean {
    return !this.#failed && !this.#closed;
  }

  async #start(): Promise<void> {
    const { api, documentId, layerId, target, brush } = this.#options;
    try {
      this.#id = await api.strokeBegin(documentId, layerId, target, brush);
    } catch (error: unknown) {
      this.#failed = true;
      this.#queue = [];
      this.#options.onBeginError(error);
      return;
    }
    this.#pump();
  }

  /** Adds points ([x, y, pressure, ...]). They go to the shell as soon as no call is in flight. */
  addPoints(points: readonly number[]): void {
    if (!this.active || points.length === 0) {
      return;
    }
    for (const value of points) {
      this.#queue.push(value);
    }
    this.#pump();
  }

  #pump(): void {
    const id = this.#id;
    if (id === null || this.#inFlight !== null || this.#queue.length === 0 || this.#failed) {
      return;
    }
    const batch = this.#queue.splice(0, MAX_POINTS_PER_CALL * VALUES_PER_POINT);
    this.#inFlight = this.#options.api
      .strokePoints(this.#options.documentId, id, batch)
      .then(() => {
        if (!this.#closed) {
          this.#options.onProgress();
        }
      })
      .catch((error: unknown) => {
        // The stroke is not current any more: the points are not needed.
        if (isInvalidInput(error)) {
          this.#queue = [];
        } else {
          this.#options.onError(error);
        }
      })
      .finally(() => {
        this.#inFlight = null;
        this.#pump();
      });
  }

  /** Waits until no point waits and no call is in flight. */
  async #drain(): Promise<void> {
    while (this.#inFlight !== null || this.#queue.length > 0) {
      if (this.#inFlight === null) {
        this.#pump();
        if (this.#inFlight === null) {
          // No stroke id: nothing can be sent.
          return;
        }
      }
      await this.#inFlight;
    }
  }

  /**
   * Sends the waiting points, then ends the stroke. Returns the document
   * info, or null if the begin failed or the stroke was not current.
   */
  async end(): Promise<DocumentInfo | null> {
    if (this.#closed) {
      return null;
    }
    await this.#begin;
    this.#closed = true;
    const id = this.#id;
    if (this.#failed || id === null) {
      return null;
    }
    await this.#drain();
    try {
      return await this.#options.api.strokeEnd(this.#options.documentId, id);
    } catch (error: unknown) {
      if (!isInvalidInput(error)) {
        this.#options.onError(error);
      }
      return null;
    }
  }

  /** Drops the waiting points and cancels the stroke. Returns the document info, or null. */
  async cancel(): Promise<DocumentInfo | null> {
    if (this.#closed) {
      return null;
    }
    this.#closed = true;
    this.#queue = [];
    await this.#begin;
    const id = this.#id;
    if (this.#failed || id === null) {
      return null;
    }
    await this.#inFlight;
    try {
      return await this.#options.api.strokeCancel(this.#options.documentId, id);
    } catch (error: unknown) {
      if (!isInvalidInput(error)) {
        this.#options.onError(error);
      }
      return null;
    }
  }
}
