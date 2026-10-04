import { describe, expect, it, vi } from 'vitest';
import { makeFrameBody } from '../../test/frames';
import { DEFAULT_MAX_IN_FLIGHT, isSuperseded, PreviewClient, type FrameEvent } from './previewClient';
import type { DevelopSettingsJson, FrameTransport, RenderOptions } from './transport';

interface Call {
  readonly session: number;
  readonly settings: DevelopSettingsJson;
  readonly longEdge: number | null;
  resolve(body: ArrayBuffer): void;
  reject(error: unknown): void;
}

/** A transport whose requests the test completes by hand. */
class ManualTransport implements FrameTransport {
  readonly name = 'ipc' as const;
  readonly calls: Call[] = [];

  render(session: number, settings: DevelopSettingsJson, options: RenderOptions = {}): Promise<ArrayBuffer> {
    return new Promise((resolve, reject) => {
      this.calls.push({ session, settings, longEdge: options.longEdge ?? null, resolve, reject });
    });
  }

  call(index: number): Call {
    const call = this.calls[index];
    if (call === undefined) {
      throw new Error(`no call ${index}; ${this.calls.length} calls`);
    }
    return call;
  }
}

function setup(maxInFlight?: number) {
  const transport = new ManualTransport();
  const frames: FrameEvent[] = [];
  const errors: unknown[] = [];
  let clock = 0;
  const client = new PreviewClient({
    transport,
    onFrame: (event) => frames.push(event),
    onError: (error) => errors.push(error),
    now: () => clock,
    ...(maxInFlight === undefined ? {} : { maxInFlight }),
  });
  const advance = (ms: number): void => {
    clock += ms;
  };
  return { transport, client, frames, errors, advance };
}

const ev = (exposure: number): DevelopSettingsJson => ({ tone: { exposure } });

/** Lets pending promise callbacks run. */
const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

describe('PreviewClient', () => {
  it('sends nothing without a session', async () => {
    const { transport, client } = setup();
    client.request(ev(1));
    expect(transport.calls).toHaveLength(0);
    await expect(client.idle()).resolves.toBeUndefined();
  });

  it('with one slot, keeps one request in flight and sends only the newest waiting settings', async () => {
    const { transport, client, frames } = setup(1);
    client.setSession(5);
    client.request(ev(1));
    client.request(ev(2));
    client.request(ev(3));
    client.request(ev(4));
    expect(transport.calls).toHaveLength(1);
    expect(transport.call(0)).toMatchObject({ session: 5, settings: ev(1) });

    transport.call(0).resolve(makeFrameBody(2, 2, 0));
    await flush();
    expect(transport.calls).toHaveLength(2);
    expect(transport.call(1).settings).toEqual(ev(4));

    transport.call(1).resolve(makeFrameBody(2, 2, 1));
    await client.idle();
    expect(frames.map((f) => f.frame.seq)).toEqual([0, 1]);
    expect(client.stats).toMatchObject({ requested: 4, sent: 2, delivered: 2, superseded: 0, failed: 0 });
  });

  it('reports the round-trip time of each frame', async () => {
    const { transport, client, frames, advance } = setup();
    client.setSession(1);
    client.request(ev(0));
    advance(12.5);
    transport.call(0).resolve(makeFrameBody(1, 1, 0));
    await client.idle();
    expect(frames[0]?.roundTripMs).toBe(12.5);
  });

  it('counts superseded requests and does not report them', async () => {
    const { transport, client, errors, frames } = setup();
    client.setSession(1);
    client.request(ev(1));
    transport.call(0).reject({ kind: 'superseded', message: 'replaced' });
    await client.idle();
    expect(errors).toHaveLength(0);
    expect(frames).toHaveLength(0);
    expect(client.stats.superseded).toBe(1);
  });

  it('reports other errors and continues with the next request', async () => {
    const { transport, client, errors, frames } = setup();
    client.setSession(1);
    client.request(ev(1));
    client.request(ev(2));
    const failure = { kind: 'internal', message: 'GPU lost' };
    transport.call(0).reject(failure);
    await flush();
    expect(errors).toEqual([failure]);
    expect(transport.calls).toHaveLength(2);
    transport.call(1).resolve(makeFrameBody(1, 1, 3));
    await client.idle();
    expect(frames.map((f) => f.frame.seq)).toEqual([3]);
    expect(client.stats.failed).toBe(1);
  });

  it('reports a body that is not a frame as an error', async () => {
    const { transport, client, errors } = setup();
    client.setSession(1);
    client.request(ev(1));
    transport.call(0).resolve(new ArrayBuffer(4));
    await client.idle();
    expect(errors).toHaveLength(1);
    expect(String(errors[0])).toMatch(/FrameFormatError/);
  });

  it('drops results of an old session and waiting settings at a session change', async () => {
    const { transport, client, frames, errors } = setup(1);
    client.setSession(1);
    client.request(ev(1));
    client.request(ev(2));
    client.setSession(2);
    transport.call(0).resolve(makeFrameBody(1, 1, 9));
    await flush();
    // The waiting settings of session 1 are gone; nothing new is sent.
    expect(transport.calls).toHaveLength(1);
    expect(frames).toHaveLength(0);

    client.request(ev(3));
    expect(transport.call(1)).toMatchObject({ session: 2, settings: ev(3) });
    // An error of the new session is reported; the sequence restarts at 0.
    transport.call(1).resolve(makeFrameBody(1, 1, 0));
    await client.idle();
    expect(frames.map((f) => f.frame.seq)).toEqual([0]);
    expect(errors).toHaveLength(0);
    expect(client.stats.dropped).toBe(1);
  });

  it('drops errors of an old session', async () => {
    const { transport, client, errors } = setup();
    client.setSession(1);
    client.request(ev(1));
    client.setSession(null);
    transport.call(0).reject(new Error('session closed'));
    await client.idle();
    expect(errors).toHaveLength(0);
    expect(client.stats.dropped).toBe(1);
  });

  it('drops everything after dispose', async () => {
    const { transport, client, frames } = setup();
    client.setSession(1);
    client.request(ev(1));
    client.dispose();
    client.request(ev(2));
    transport.call(0).resolve(makeFrameBody(1, 1, 0));
    await flush();
    expect(frames).toHaveLength(0);
    expect(transport.calls).toHaveLength(1);
    await expect(client.idle()).resolves.toBeUndefined();
  });

  it('with two slots, drops a frame that arrives after a newer one', async () => {
    const { transport, client, frames } = setup(2);
    client.setSession(1);
    client.request(ev(1));
    client.request(ev(2));
    expect(transport.calls).toHaveLength(2);
    transport.call(1).resolve(makeFrameBody(1, 1, 1));
    await flush();
    transport.call(0).resolve(makeFrameBody(1, 1, 0));
    await client.idle();
    expect(frames.map((f) => f.frame.seq)).toEqual([1]);
    expect(client.stats.dropped).toBe(1);
  });

  it('keeps two requests in flight by default and never more', async () => {
    expect(DEFAULT_MAX_IN_FLIGHT).toBe(2);
    const { transport, client, frames } = setup();
    client.setSession(1);
    for (let i = 1; i <= 5; i += 1) {
      client.request(ev(i));
    }
    expect(transport.calls.map((c) => c.settings)).toEqual([ev(1), ev(2)]);
    expect(client.inFlight).toBe(2);

    // The first frame frees one slot: the newest waiting settings go out.
    transport.call(0).resolve(makeFrameBody(1, 1, 0));
    await flush();
    expect(transport.calls.map((c) => c.settings)).toEqual([ev(1), ev(2), ev(5)]);
    expect(client.inFlight).toBe(2);

    transport.call(1).resolve(makeFrameBody(1, 1, 1));
    transport.call(2).resolve(makeFrameBody(1, 1, 2));
    await client.idle();
    expect(frames.map((f) => f.frame.seq)).toEqual([0, 1, 2]);
    expect(client.stats).toMatchObject({ requested: 5, sent: 3, delivered: 3, peakInFlight: 2 });
  });

  it('frees the slot before the frame handler runs', async () => {
    const transport = new ManualTransport();
    const callsAtFrame: number[] = [];
    const client = new PreviewClient({
      transport,
      onFrame: () => callsAtFrame.push(transport.calls.length),
      onError: (error) => {
        throw error;
      },
      maxInFlight: 1,
    });
    client.setSession(1);
    client.request(ev(1));
    client.request(ev(2));
    transport.call(0).resolve(makeFrameBody(1, 1, 0));
    await flush();
    // The second request was sent before the first frame was handled.
    expect(callsAtFrame).toEqual([2]);
  });

  it('with two slots, never delivers a stale frame after a superseded one', async () => {
    const { transport, client, frames } = setup(2);
    client.setSession(1);
    client.request(ev(1));
    client.request(ev(2));
    client.request(ev(3));
    // Request 2 is superseded in the shell. Request 3 renders as frame 1
    // and arrives before frame 0 (request 1).
    transport.call(1).reject({ kind: 'superseded', message: 'replaced' });
    await flush();
    expect(transport.call(2).settings).toEqual(ev(3));
    transport.call(2).resolve(makeFrameBody(1, 1, 1));
    await flush();
    transport.call(0).resolve(makeFrameBody(1, 1, 0));
    await client.idle();
    expect(frames.map((f) => f.frame.seq)).toEqual([1]);
    expect(client.stats).toMatchObject({ superseded: 1, dropped: 1, delivered: 1 });
  });

  it('sends the target long edge and requests a new frame when it changes', async () => {
    const { transport, client } = setup(1);
    client.setSession(1);
    // No settings yet: a size change sends nothing.
    client.setTargetLongEdge(1280);
    expect(transport.calls).toHaveLength(0);
    expect(client.targetLongEdge).toBe(1280);

    client.request(ev(1));
    expect(transport.call(0).longEdge).toBe(1280);
    transport.call(0).resolve(makeFrameBody(1, 1, 0));
    await client.idle();

    client.setTargetLongEdge(1280);
    expect(transport.calls).toHaveLength(1);
    client.setTargetLongEdge(640);
    expect(transport.call(1)).toMatchObject({ settings: ev(1), longEdge: 640 });
    transport.call(1).resolve(makeFrameBody(1, 1, 1));
    await client.idle();

    // Invalid values mean "no target".
    client.setTargetLongEdge(Number.NaN);
    expect(client.targetLongEdge).toBeNull();
    expect(transport.call(2).longEdge).toBeNull();
  });

  it('a throwing frame handler is reported as an error', async () => {
    const transport = new ManualTransport();
    const onError = vi.fn();
    const client = new PreviewClient({
      transport,
      onFrame: () => {
        throw new Error('draw failed');
      },
      onError,
    });
    client.setSession(1);
    client.request(ev(1));
    transport.call(0).resolve(makeFrameBody(1, 1, 0));
    await client.idle();
    expect(onError).toHaveBeenCalledOnce();
  });
});

describe('isSuperseded', () => {
  it('detects only the superseded kind', () => {
    expect(isSuperseded({ kind: 'superseded', message: '' })).toBe(true);
    expect(isSuperseded({ kind: 'internal', message: '' })).toBe(false);
    expect(isSuperseded(null)).toBe(false);
    expect(isSuperseded('superseded')).toBe(false);
  });
});
