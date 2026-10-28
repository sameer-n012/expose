import { describe, expect, it, vi } from 'vitest';
import type { DocumentInfo } from './api';
import { defaultBrush } from './brush';
import { MAX_POINTS_PER_CALL, StrokeClient } from './strokeClient';

const INFO = { documentId: 1 } as unknown as DocumentInfo;

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

function pts(n: number, from = 0): number[] {
  const out: number[] = [];
  for (let i = 0; i < n; i += 1) {
    out.push(from + i, 0, 1);
  }
  return out;
}

function setup() {
  const begin = deferred<number>();
  const calls: { resolve: () => void; reject: (e: unknown) => void; points: number[] }[] = [];
  const api = {
    strokeBegin: vi.fn(() => begin.promise),
    strokePoints: vi.fn((_d: number, _s: number, points: number[]) => {
      const d = deferred<null>();
      calls.push({ resolve: () => d.resolve(null), reject: d.reject, points });
      return d.promise;
    }),
    strokeEnd: vi.fn(async () => INFO),
    strokeCancel: vi.fn(async () => INFO),
  };
  const onProgress = vi.fn();
  const onBeginError = vi.fn();
  const onError = vi.fn();
  const client = new StrokeClient({
    api,
    documentId: 1,
    layerId: 7,
    target: { type: 'erase' },
    brush: defaultBrush(),
    onProgress,
    onBeginError,
    onError,
  });
  return { client, api, begin, calls, onProgress, onBeginError, onError };
}

describe('StrokeClient', () => {
  it('begins at once with the arguments', () => {
    const { api } = setup();
    expect(api.strokeBegin).toHaveBeenCalledWith(1, 7, { type: 'erase' }, defaultBrush());
  });

  it('holds the points until begin resolves', async () => {
    const { client, api, begin, calls } = setup();
    client.addPoints(pts(2));
    await tick();
    expect(api.strokePoints).not.toHaveBeenCalled();
    begin.resolve(42);
    await tick();
    expect(api.strokePoints).toHaveBeenCalledTimes(1);
    expect(api.strokePoints.mock.calls[0]?.[1]).toBe(42);
    expect(calls[0]?.points).toEqual(pts(2));
  });

  it('keeps one call in flight and batches the points that come meanwhile', async () => {
    const { client, begin, calls, onProgress } = setup();
    begin.resolve(1);
    await tick();
    client.addPoints(pts(1));
    client.addPoints(pts(1, 10));
    client.addPoints(pts(1, 20));
    await tick();
    expect(calls).toHaveLength(1);
    calls[0]?.resolve();
    await tick();
    expect(onProgress).toHaveBeenCalledTimes(1);
    expect(calls).toHaveLength(2);
    expect(calls[1]?.points).toEqual([...pts(1, 10), ...pts(1, 20)]);
  });

  it('splits at 3000 points', async () => {
    const { client, begin, calls } = setup();
    begin.resolve(1);
    await tick();
    client.addPoints(pts(MAX_POINTS_PER_CALL * 2 + 5));
    await tick();
    expect(calls[0]?.points.length).toBe(MAX_POINTS_PER_CALL * 3);
    calls[0]?.resolve();
    await tick();
    expect(calls[1]?.points.length).toBe(MAX_POINTS_PER_CALL * 3);
    calls[1]?.resolve();
    await tick();
    expect(calls[2]?.points.length).toBe(15);
  });

  it('ends after it sent all points', async () => {
    const { client, api, begin, calls } = setup();
    begin.resolve(1);
    await tick();
    client.addPoints(pts(1));
    client.addPoints(pts(1, 5));
    const ended = client.end();
    await tick();
    expect(api.strokeEnd).not.toHaveBeenCalled();
    calls[0]?.resolve();
    await tick();
    expect(api.strokeEnd).not.toHaveBeenCalled();
    calls[1]?.resolve();
    expect(await ended).toBe(INFO);
    expect(api.strokeEnd).toHaveBeenCalledWith(1, 1);
    expect(calls.map((c) => c.points.length)).toEqual([3, 3]);
  });

  it('ends after begin when no point came', async () => {
    const { client, api, begin } = setup();
    const ended = client.end();
    begin.resolve(9);
    expect(await ended).toBe(INFO);
    expect(api.strokePoints).not.toHaveBeenCalled();
    expect(api.strokeEnd).toHaveBeenCalledWith(1, 9);
  });

  it('cancels and drops the waiting points', async () => {
    const { client, api, begin, calls } = setup();
    begin.resolve(3);
    await tick();
    client.addPoints(pts(1));
    client.addPoints(pts(1, 5));
    const cancelled = client.cancel();
    calls[0]?.resolve();
    expect(await cancelled).toBe(INFO);
    expect(calls).toHaveLength(1);
    expect(api.strokeCancel).toHaveBeenCalledWith(1, 3);
    expect(api.strokeEnd).not.toHaveBeenCalled();
    client.addPoints(pts(1));
    await tick();
    expect(calls).toHaveLength(1);
  });

  it('drops the gesture when begin fails', async () => {
    const { client, api, begin, calls, onBeginError } = setup();
    client.addPoints(pts(1));
    begin.reject({ kind: 'invalidInput', message: 'no pixel layer' });
    await tick();
    expect(onBeginError).toHaveBeenCalledTimes(1);
    expect(client.active).toBe(false);
    expect(await client.end()).toBeNull();
    expect(calls).toHaveLength(0);
    expect(api.strokeEnd).not.toHaveBeenCalled();
  });

  it('ignores the invalidInput error of an old stroke id', async () => {
    const { client, api, begin, calls, onError } = setup();
    begin.resolve(1);
    await tick();
    client.addPoints(pts(1));
    await tick();
    calls[0]?.reject({ kind: 'invalidInput', message: 'old stroke' });
    api.strokeEnd.mockRejectedValueOnce({ kind: 'invalidInput', message: 'old stroke' });
    expect(await client.end()).toBeNull();
    expect(onError).not.toHaveBeenCalled();
  });

  it('reports other errors of the points call', async () => {
    const { client, begin, calls, onError } = setup();
    begin.resolve(1);
    await tick();
    client.addPoints(pts(1));
    await tick();
    calls[0]?.reject({ kind: 'io', message: 'broken' });
    await tick();
    expect(onError).toHaveBeenCalledTimes(1);
  });
});
