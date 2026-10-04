import { describe, expect, it, vi } from 'vitest';
import { benchSettings, encodedNote, formatResults, shellMsFromHeader, summarize } from './bench';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), Channel: vi.fn() }));

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
