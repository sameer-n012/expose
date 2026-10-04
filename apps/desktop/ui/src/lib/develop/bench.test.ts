import { describe, expect, it, vi } from 'vitest';
import {
  benchSettings,
  DEFAULT_SETTLE_MS,
  encodedNote,
  formatResults,
  parseBenchQuery,
  selectCandidates,
  shellMsFromHeader,
  summarize,
} from './bench';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), Channel: vi.fn() }));

describe('bench selection', () => {
  const candidates = [
    ['a', 1],
    ['b', 2],
    ['c', 3],
  ] as const;

  it('reads no options as all candidates, in order, with the default pause', () => {
    expect(parseBenchQuery('?bench=transport')).toEqual({ only: null, reverse: false, settleMs: DEFAULT_SETTLE_MS });
  });

  it('reads only, reverse, and settle', () => {
    expect(parseBenchQuery('?bench=transport&only=slider-ipc, ipc-rgba8-bitmap,&reverse=1&settle=0')).toEqual({
      only: ['slider-ipc', 'ipc-rgba8-bitmap'],
      reverse: true,
      settleMs: 0,
    });
    expect(parseBenchQuery('?reverse=0&settle=-5&only=').reverse).toBe(false);
    expect(parseBenchQuery('?settle=abc').settleMs).toBe(DEFAULT_SETTLE_MS);
    expect(parseBenchQuery('?only=').only).toBeNull();
  });

  it('keeps the list order for a subset and can reverse it', () => {
    const names = (list: ReadonlyArray<readonly [string, number]>): string[] => list.map(([name]) => name);
    expect(names(selectCandidates(candidates, { only: null, reverse: false }))).toEqual(['a', 'b', 'c']);
    expect(names(selectCandidates(candidates, { only: ['c', 'a'], reverse: false }))).toEqual(['a', 'c']);
    expect(names(selectCandidates(candidates, { only: ['c', 'a'], reverse: true }))).toEqual(['c', 'a']);
    expect(names(selectCandidates(candidates, { only: null, reverse: true }))).toEqual(['c', 'b', 'a']);
  });

  it('rejects an unknown candidate name and lists the valid names', () => {
    expect(() => selectCandidates(candidates, { only: ['a', 'x'], reverse: false })).toThrow(
      'Unknown bench candidate(s): x. Valid names: a, b, c.',
    );
  });
});

describe('bench helpers', () => {
  it('summarizes means and frames per second', () => {
    const result = summarize(
      'ipc-rgba8-put',
      [
        { transferMs: 10, drawMs: 2, shellMs: 4, bytes: 100 },
        { transferMs: 20, drawMs: 4, shellMs: 6, bytes: 300 },
      ],
      50,
    );
    expect(result).toMatchObject({ frames: 2, fps: 40, transferMs: 15, drawMs: 3, shellMs: 5, bytes: 200 });
    expect(summarize('empty', [], 0)).toMatchObject({ frames: 0, fps: 0, transferMs: 0 });
  });

  it('formats an aligned table', () => {
    const text = formatResults([summarize('a', [{ transferMs: 1, drawMs: 1, shellMs: 1, bytes: 2e6 }], 10, 'x')]);
    const lines = text.split('\n');
    expect(lines).toHaveLength(3);
    expect(lines[0]).toMatch(/^candidate\s+frames\s+fps/);
    expect(lines[2]).toMatch(/^a\s+1\s+100\.0\s+1\.00\s+1\.00\s+1\.00\s+2\.00\s+x$/);
  });

  it('reads the shell time from the frame header value', () => {
    expect(shellMsFromHeader('2560,1707,3,10,4500')).toBe(4.5);
    expect(shellMsFromHeader(null)).toBe(0);
    expect(shellMsFromHeader('garbage')).toBe(0);
  });

  it('describes PNG frames as lossless and JPEG frames as lossy', () => {
    expect(encodedNote('jpeg')).toMatch(/lossy at q85/);
    expect(encodedNote('png')).toMatch(/lossless/);
    expect(encodedNote('png')).not.toMatch(/lossy|q85/);
  });

  it('gives each frame new settings', () => {
    expect(benchSettings(1)).not.toEqual(benchSettings(2));
    expect(benchSettings(0)).toEqual({ tone: { exposure: 0 } });
  });
});
