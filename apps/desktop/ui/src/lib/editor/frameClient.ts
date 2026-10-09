/**
 * The Editor frame client: requests frames for the newest view of the open
 * document.
 *
 * - At most `maxInFlight` requests (default 2) are in flight.
 * - A `request()` while all slots are busy replaces the area that waits
 *   (latest wins).
 * - Requests are numbered. A result is delivered only if it is newer than
 *   the last delivered result, so a stale frame never draws over a newer
 *   one. A document change drops all results of the old document.
 * - The error kind `superseded` is not an error (a newer request replaced
 *   the request in the shell).
 */
import { parseFrame, type Frame } from '../develop/frame';
import { isSuperseded } from '../develop/previewClient';
import type { RenderArea } from './api';

export const EDITOR_MAX_IN_FLIGHT = 2;

/** A frame with the area it was rendered for. */
export interface EditorFrame {
  readonly frame: Frame;
  readonly area: RenderArea;
}

export interface EditorFrameClientOptions {
  /** Renders one area of a document. */
  readonly render: (documentId: number, area: RenderArea) => Promise<ArrayBuffer>;
  readonly onFrame: (frame: EditorFrame) => void;
  readonly onError: (error: unknown) => void;
  readonly maxInFlight?: number;
}

export class EditorFrameClient {
  readonly #render: EditorFrameClientOptions['render'];
  readonly #onFrame: EditorFrameClientOptions['onFrame'];
  readonly #onError: EditorFrameClientOptions['onError'];
  readonly #maxInFlight: number;

  #documentId: number | null = null;
  #waiting: RenderArea | null = null;
  #inFlight = 0;
  #nextRequest = 0;
  #lastDelivered = -1;
  /** Increases at each document change. */
  #generation = 0;
  #disposed = false;

  constructor(options: EditorFrameClientOptions) {
    this.#render = options.render;
    this.#onFrame = options.onFrame;
    this.#onError = options.onError;
    this.#maxInFlight = Math.max(1, Math.floor(options.maxInFlight ?? EDITOR_MAX_IN_FLIGHT));
  }

  get inFlight(): number {
    return this.#inFlight;
  }

  /** Sets the document (or none). Waiting areas and old results are dropped. */
  setDocument(documentId: number | null): void {
    if (documentId === this.#documentId) {
      return;
    }
    this.#documentId = documentId;
    this.#waiting = null;
    this.#generation += 1;
    this.#lastDelivered = -1;
    this.#nextRequest = 0;
  }

  /** Requests a frame of `area`. */
  request(area: RenderArea): void {
    if (this.#disposed) {
      return;
    }
    this.#waiting = area;
    this.#pump();
  }

  dispose(): void {
    this.#disposed = true;
    this.#waiting = null;
  }

  #pump(): void {
    while (!this.#disposed && this.#documentId !== null && this.#waiting !== null && this.#inFlight < this.#maxInFlight) {
      const area = this.#waiting;
      this.#waiting = null;
      void this.#send(this.#documentId, area);
    }
  }

  async #send(documentId: number, area: RenderArea): Promise<void> {
    const generation = this.#generation;
    const id = this.#nextRequest;
    this.#nextRequest += 1;
    this.#inFlight += 1;
    let body: ArrayBuffer;
    try {
      body = await this.#render(documentId, area);
    } catch (error: unknown) {
      this.#release();
      if (!isSuperseded(error) && !this.#disposed && generation === this.#generation) {
        this.#onError(error);
      }
      return;
    }
    this.#release();
    if (this.#disposed || generation !== this.#generation || id <= this.#lastDelivered) {
      return;
    }
    try {
      const frame = parseFrame(body);
      this.#lastDelivered = id;
      this.#onFrame({ frame, area });
    } catch (error: unknown) {
      this.#onError(error);
    }
  }

  #release(): void {
    this.#inFlight -= 1;
    this.#pump();
  }
}
