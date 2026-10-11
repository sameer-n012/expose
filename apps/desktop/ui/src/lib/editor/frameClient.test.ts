import { describe, expect, it, vi } from 'vitest';
import { makeFrameBody } from '../../test/frames';
import type { RenderArea } from './api';
import { EditorFrameClient, type EditorFrame } from './frameClient';

function area(n: number): RenderArea {
  return { originX: n, originY: 0, scale: 1, width: 2, height: 2 };
}

function deferred() {
  let resolve!: (b: ArrayBuffer) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<ArrayBuffer>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

function setup(maxInFlight?: number) {
  const calls: Array<{ documentId: number; area: RenderArea; d: ReturnType<typeof deferred> }> = [];
  const frames: EditorFrame[] = [];
  const onError = vi.fn();
  const client = new EditorFrameClient({
    render: (documentId, a) => {
      const d = deferred();
      calls.push({ documentId, area: a, d });
      return d.promise;
    },
    onFrame: (f) => frames.push(f),
    onError,
    ...(maxInFlight === undefined ? {} : { maxInFlight }),
  });
  return { client, calls, frames, onError };
}

describe('EditorFrameClient', () => {
  it('does nothing without a document', () => {
    const { client, calls } = setup();
    client.request(area(1));
    expect(calls).toHaveLength(0);
  });

  it('keeps at most 2 requests in flight and sends the newest waiting area', async () => {
    const { client, calls } = setup();
    client.setDocument(7);
    client.request(area(1));
    client.request(area(2));
    client.request(area(3));
    client.request(area(4));
    expect(calls.map((c) => c.area.originX)).toEqual([1, 2]);
    expect(client.inFlight).toBe(2);
    calls[0]?.d.resolve(makeFrameBody(2, 2, 0));
    await tick();
    expect(calls.map((c) => c.area.originX)).toEqual([1, 2, 4]);
    expect(calls[0]?.documentId).toBe(7);
  });

  it('drops a frame that arrives after a newer one', async () => {
    const { client, calls, frames } = setup();
    client.setDocument(1);
    client.request(area(1));
    client.request(area(2));
    calls[1]?.d.resolve(makeFrameBody(2, 2, 0));
    await tick();
    calls[0]?.d.resolve(makeFrameBody(2, 2, 0));
    await tick();
    expect(frames.map((f) => f.area.originX)).toEqual([2]);
  });

  it('drops results of an old document', async () => {
    const { client, calls, frames } = setup();
    client.setDocument(1);
    client.request(area(1));
    client.setDocument(2);
    calls[0]?.d.resolve(makeFrameBody(2, 2, 0));
    await tick();
    expect(frames).toHaveLength(0);
  });

  it('ignores superseded and reports other errors', async () => {
    const { client, calls, onError } = setup(1);
    client.setDocument(1);
    client.request(area(1));
    calls[0]?.d.reject({ kind: 'superseded', message: 'x' });
    await tick();
    expect(onError).not.toHaveBeenCalled();
    client.request(area(2));
    calls[1]?.d.reject({ kind: 'internal', message: 'boom' });
    await tick();
    expect(onError).toHaveBeenCalledTimes(1);
  });

  it('ignores notFound of a closed document, but not of the open one', async () => {
    let open: number | null = 1;
    const calls: Array<ReturnType<typeof deferred>> = [];
    const onError = vi.fn();
    const client = new EditorFrameClient({
      render: () => {
        const d = deferred();
        calls.push(d);
        return d.promise;
      },
      onFrame: vi.fn(),
      onError,
      isCurrent: (id) => id === open,
      maxInFlight: 1,
    });
    client.setDocument(1);
    client.request(area(1));
    // The store dropped the document and the shell closed it; the client
    // still has the old document.
    open = null;
    calls[0]?.reject({ kind: 'notFound', message: 'editor document 1 is not open' });
    await tick();
    expect(onError).not.toHaveBeenCalled();
    open = 1;
    client.request(area(2));
    calls[1]?.reject({ kind: 'notFound', message: 'editor document 1 is not open' });
    await tick();
    expect(onError).toHaveBeenCalledTimes(1);
  });

  it('reports a bad frame body', async () => {
    const { client, calls, onError, frames } = setup();
    client.setDocument(1);
    client.request(area(1));
    calls[0]?.d.resolve(new ArrayBuffer(4));
    await tick();
    expect(onError).toHaveBeenCalledTimes(1);
    expect(frames).toHaveLength(0);
  });
});
