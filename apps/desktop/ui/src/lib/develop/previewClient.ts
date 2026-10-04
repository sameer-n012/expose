/**
 * The preview client: requests frames for the newest develop settings and
 * gives the parsed frames to a frame function (normally a
 * `FramePresenter`, see `presenter.ts`).
 *
 * # Pipelining
 *
 * The client keeps up to `maxInFlight` requests in flight (default
 * `DEFAULT_MAX_IN_FLIGHT` = 2). With two requests, the shell renders the
 * next frame while the web view receives the current frame, so render and
 * transfer overlap. A slot becomes free when the frame body arrives, not
 * when the frame is drawn, so the draw does not hold back the next
 * request.
 *
 * # Coalescing (latest wins)
 *
 * A slider gives many settings per second. A `request()` call while all
 * slots are busy only replaces the waiting settings. When a slot becomes
 * free, the client sends the newest waiting settings. Thus the shell never
 * gets a queue of stale frames, and the last settings always give a frame.
 *
 * The shell coalesces too: a request that a newer one replaced fails with
 * the error kind `superseded`. The client counts it and does not report it.
 *
 * # Order
 *
 * The shell numbers the frames of a session in render order (`seq`). With
 * two requests in flight, a frame can arrive after a newer frame. The
 * client drops a frame with a `seq` that is not larger than the last
 * delivered `seq`, so a stale frame is never delivered after a newer one.
 *
 * # Errors
 *
 * Other errors go to `onError`. The client continues: the next `request()`
 * tries again.
 */
import { parseFrame, type Frame } from './frame';
import type { DevelopSettingsJson, FrameTransport } from './transport';

/** The default number of requests in flight. */
export const DEFAULT_MAX_IN_FLIGHT = 2;

/** A frame and the client-side timing of its request. */
export interface FrameEvent {
  readonly frame: Frame;
  /** Time from the send of the request to the parsed frame, in milliseconds. */
  readonly roundTripMs: number;
}

/** Counters of the client (for diagnostics and the benchmark). */
export interface PreviewClientStats {
  /** Calls of `request()`. */
  requested: number;
  /** Requests sent to the transport. */
  sent: number;
  /** Frames given to `onFrame`. */
  delivered: number;
  /** Requests that the shell replaced with a newer one. */
  superseded: number;
  /** Frames dropped because the session changed, the client was disposed, or a newer frame came first. */
  dropped: number;
  /** Errors given to `onError`. */
  failed: number;
  /** The largest number of requests in flight at one time. */
  peakInFlight: number;
}

export interface PreviewClientOptions {
  readonly transport: FrameTransport;
  readonly onFrame: (event: FrameEvent) => void;
  readonly onError: (error: unknown) => void;
  /** The maximum number of requests in flight (1 or more). Default `DEFAULT_MAX_IN_FLIGHT`. */
  readonly maxInFlight?: number;
  /** The clock in milliseconds. Default `performance.now`. For tests. */
  readonly now?: () => number;
}

/** Returns true if the error is a command error of kind `superseded`. */
export function isSuperseded(error: unknown): boolean {
  return (
    typeof error === 'object' && error !== null && 'kind' in error && (error as { kind: unknown }).kind === 'superseded'
  );
}

export class PreviewClient {
  readonly stats: PreviewClientStats = {
    requested: 0,
    sent: 0,
    delivered: 0,
    superseded: 0,
    dropped: 0,
    failed: 0,
    peakInFlight: 0,
  };

  readonly #transport: FrameTransport;
  readonly #onFrame: (event: FrameEvent) => void;
  readonly #onError: (error: unknown) => void;
  readonly #maxInFlight: number;
  readonly #now: () => number;

  #session: number | null = null;
  #waiting: DevelopSettingsJson | null = null;
  /** The settings of the last `request()` (for a new frame at a size change). */
  #latest: DevelopSettingsJson | null = null;
  #longEdge: number | null = null;
  #inFlight = 0;
  #disposed = false;
  /** Increases at each session change, so that old results are dropped. */
  #generation = 0;
  /** The highest frame sequence number delivered in this generation. */
  #lastSeq = -1;
  #idleWaiters: Array<() => void> = [];

  constructor(options: PreviewClientOptions) {
    this.#transport = options.transport;
    this.#onFrame = options.onFrame;
    this.#onError = options.onError;
    this.#maxInFlight = Math.max(1, Math.floor(options.maxInFlight ?? DEFAULT_MAX_IN_FLIGHT));
    this.#now = options.now ?? (() => performance.now());
  }

  /** The current session, or null. */
  get session(): number | null {
    return this.#session;
  }

  /** The number of requests in flight. */
  get inFlight(): number {
    return this.#inFlight;
  }

  /** The maximum number of requests in flight. */
  get maxInFlight(): number {
    return this.#maxInFlight;
  }

  /** The target long edge sent with each request, or null (the session size). */
  get targetLongEdge(): number | null {
    return this.#longEdge;
  }

  /**
   * Sets the session. Waiting settings are dropped, and results of requests
   * for the old session are not delivered.
   */
  setSession(session: number | null): void {
    if (session === this.#session) {
      return;
    }
    this.#session = session;
    this.#waiting = null;
    this.#latest = null;
    this.#generation += 1;
    this.#lastSeq = -1;
    this.#settleIdle();
  }

  /**
   * Sets the target long edge of the frames (see `targetLongEdge` in
   * `size.ts`). A change requests a new frame for the last settings, so
   * that the canvas gets the new size.
   */
  setTargetLongEdge(longEdge: number | null): void {
    const value = longEdge === null || !Number.isFinite(longEdge) || longEdge <= 0 ? null : Math.round(longEdge);
    if (value === this.#longEdge) {
      return;
    }
    this.#longEdge = value;
    if (!this.#disposed && this.#waiting === null && this.#latest !== null) {
      this.#waiting = this.#latest;
      this.#pump();
    }
  }

  /** Requests a frame for `settings`. Replaces settings that wait to be sent. */
  request(settings: DevelopSettingsJson): void {
    if (this.#disposed) {
      return;
    }
    this.stats.requested += 1;
    this.#waiting = settings;
    this.#latest = settings;
    this.#pump();
  }

  /** Stops the client. Results that arrive later are dropped. */
  dispose(): void {
    this.#disposed = true;
    this.#waiting = null;
    this.#settleIdle();
  }

  /** Resolves when no request is in flight and no settings wait (or the client is disposed). */
  idle(): Promise<void> {
    if (this.#isIdle()) {
      return Promise.resolve();
    }
    return new Promise((resolve) => this.#idleWaiters.push(resolve));
  }

  #isIdle(): boolean {
    return this.#disposed || (this.#inFlight === 0 && (this.#waiting === null || this.#session === null));
  }

  #settleIdle(): void {
    if (!this.#isIdle()) {
      return;
    }
    const waiters = this.#idleWaiters;
    this.#idleWaiters = [];
    for (const resolve of waiters) {
      resolve();
    }
  }

  #pump(): void {
    while (!this.#disposed && this.#session !== null && this.#waiting !== null && this.#inFlight < this.#maxInFlight) {
      const settings = this.#waiting;
      this.#waiting = null;
      void this.#send(this.#session, settings);
    }
    this.#settleIdle();
  }

  async #send(session: number, settings: DevelopSettingsJson): Promise<void> {
    const generation = this.#generation;
    const started = this.#now();
    this.#inFlight += 1;
    this.stats.sent += 1;
    this.stats.peakInFlight = Math.max(this.stats.peakInFlight, this.#inFlight);
    let body: ArrayBuffer;
    try {
      body = await this.#transport.render(session, settings, { longEdge: this.#longEdge });
    } catch (error: unknown) {
      this.#release();
      if (isSuperseded(error)) {
        this.stats.superseded += 1;
      } else if (this.#disposed || generation !== this.#generation) {
        this.stats.dropped += 1;
      } else {
        this.stats.failed += 1;
        this.#onError(error);
      }
      return;
    }
    // Free the slot before the frame is handled, so that the next request
    // goes out while this frame is parsed and drawn.
    this.#release();
    if (this.#disposed || generation !== this.#generation) {
      this.stats.dropped += 1;
      return;
    }
    try {
      const frame = parseFrame(body);
      if (frame.seq <= this.#lastSeq) {
        // An older frame arrived after a newer one (two requests in flight).
        this.stats.dropped += 1;
        return;
      }
      this.#lastSeq = frame.seq;
      this.stats.delivered += 1;
      this.#onFrame({ frame, roundTripMs: this.#now() - started });
    } catch (error: unknown) {
      this.stats.failed += 1;
      this.#onError(error);
    }
  }

  /** Frees one slot and sends the waiting settings, if any. */
  #release(): void {
    this.#inFlight -= 1;
    this.#pump();
  }
}
