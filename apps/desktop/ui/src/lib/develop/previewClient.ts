/**
 * The preview client: requests frames for the newest develop settings and
 * gives the parsed frames to a draw function.
 *
 * # Coalescing (latest wins)
 *
 * A slider gives many settings per second. The client keeps at most
 * `maxInFlight` requests (default 1) in flight. A `request()` call while
 * all slots are busy only replaces the waiting settings. When a request
 * completes, the client sends the newest waiting settings. Thus the shell
 * never gets a queue of stale frames, and the last settings always give a
 * frame.
 *
 * The shell coalesces too: a request that a newer one replaced fails with
 * the error kind `superseded`. The client counts it and does not report it.
 *
 * # Errors
 *
 * Other errors go to `onError`. The client continues: the next `request()`
 * tries again.
 */
import { parseFrame, type Frame } from './frame';
import type { DevelopSettingsJson, FrameTransport } from './transport';

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
}

export interface PreviewClientOptions {
  readonly transport: FrameTransport;
  readonly onFrame: (event: FrameEvent) => void;
  readonly onError: (error: unknown) => void;
  /** The maximum number of requests in flight (1 or more). Default 1. */
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
  readonly stats: PreviewClientStats = { requested: 0, sent: 0, delivered: 0, superseded: 0, dropped: 0, failed: 0 };

  readonly #transport: FrameTransport;
  readonly #onFrame: (event: FrameEvent) => void;
  readonly #onError: (error: unknown) => void;
  readonly #maxInFlight: number;
  readonly #now: () => number;

  #session: number | null = null;
  #waiting: DevelopSettingsJson | null = null;
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
    this.#maxInFlight = Math.max(1, Math.floor(options.maxInFlight ?? 1));
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
    this.#generation += 1;
    this.#lastSeq = -1;
    this.#settleIdle();
  }

  /** Requests a frame for `settings`. Replaces settings that wait to be sent. */
  request(settings: DevelopSettingsJson): void {
    if (this.#disposed) {
      return;
    }
    this.stats.requested += 1;
    this.#waiting = settings;
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
    try {
      const body = await this.#transport.render(session, settings);
      if (this.#disposed || generation !== this.#generation) {
        this.stats.dropped += 1;
        return;
      }
      const frame = parseFrame(body);
      if (frame.seq <= this.#lastSeq) {
        // An older frame arrived after a newer one (only with maxInFlight > 1).
        this.stats.dropped += 1;
        return;
      }
      this.#lastSeq = frame.seq;
      this.stats.delivered += 1;
      this.#onFrame({ frame, roundTripMs: this.#now() - started });
    } catch (error: unknown) {
      if (isSuperseded(error)) {
        this.stats.superseded += 1;
      } else if (this.#disposed || generation !== this.#generation) {
        this.stats.dropped += 1;
      } else {
        this.stats.failed += 1;
        this.#onError(error);
      }
    } finally {
      this.#inFlight -= 1;
      this.#pump();
    }
  }
}
