/** Tests of the export form: defaults, request, validation, and storage. */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { buildRequest, defaultForm, loadForm, saveForm, STORAGE_KEY, type ExportForm } from './options';
import { isAbsolutePath, validateForm } from './validate';

function form(over: Partial<ExportForm> = {}): ExportForm {
  return { ...defaultForm(), destination: '/out', ...over };
}

afterEach(() => vi.restoreAllMocks());

describe('buildRequest', () => {
  it('builds a JPEG request from the defaults', () => {
    expect(buildRequest(form(), [3, 4])).toEqual({
      photoIds: [3, 4],
      destination: '/out',
      format: { kind: 'jpeg', quality: 85, subsampling: '420' },
      colorSpace: 'srgb',
      size: { mode: 'original' },
      allowUpscale: false,
      naming: { mode: 'original' },
      onConflict: 'uniqueName',
      dither: true,
    });
  });

  it('maps size modes and suffix naming', () => {
    const r = buildRequest(form({ sizeMode: 'megapixels', sizeValue: 12, namingMode: 'suffix', suffix: '_x' }), [1]);
    expect(r.size).toEqual({ mode: 'megapixels', megapixels: 12 });
    expect(r.naming).toEqual({ mode: 'suffix', suffix: '_x' });
    expect(buildRequest(form({ sizeMode: 'width', sizeValue: 800 }), [1]).size).toEqual({ mode: 'width', pixels: 800 });
    expect(buildRequest(form({ sizeMode: 'percent', sizeValue: 25 }), [1]).size).toEqual({ mode: 'percent', percent: 25 });
  });

  it('sends dither false for 16-bit output', () => {
    expect(buildRequest(form({ formatKind: 'tiff', tiffBitDepth: 16 }), [1]).dither).toBe(false);
    expect(buildRequest(form({ formatKind: 'png', pngBitDepth: 8 }), [1]).dither).toBe(true);
  });
});

describe('validateForm', () => {
  it('accepts the defaults with a destination', () => {
    expect(validateForm(form(), 1)).toEqual({});
  });

  it('checks the photo count', () => {
    expect(validateForm(form(), 0).photos).toBeDefined();
    expect(validateForm(form(), 10_001).photos).toBeDefined();
    expect(validateForm(form(), 10_000).photos).toBeUndefined();
  });

  it('needs an absolute destination', () => {
    expect(validateForm(form({ destination: '' }), 1).destination).toBeDefined();
    expect(validateForm(form({ destination: 'rel/dir' }), 1).destination).toBeDefined();
    expect(isAbsolutePath('C:\\photos')).toBe(true);
    expect(isAbsolutePath('\\\\server\\share')).toBe(true);
  });

  it('checks the JPEG quality only for JPEG', () => {
    expect(validateForm(form({ jpegQuality: 0 }), 1).quality).toBeDefined();
    expect(validateForm(form({ jpegQuality: 101 }), 1).quality).toBeDefined();
    expect(validateForm(form({ jpegQuality: 50.5 }), 1).quality).toBeDefined();
    expect(validateForm(form({ jpegQuality: Number.NaN }), 1).quality).toBeDefined();
    expect(validateForm(form({ jpegQuality: 0, formatKind: 'png' }), 1).quality).toBeUndefined();
  });

  it('checks the size limits per mode', () => {
    const size = (sizeMode: ExportForm['sizeMode'], sizeValue: number, allowUpscale = false) =>
      validateForm(form({ sizeMode, sizeValue, allowUpscale }), 1).size;
    expect(size('longEdge', 0)).toBeDefined();
    expect(size('longEdge', 65_536)).toBeDefined();
    expect(size('longEdge', 65_535)).toBeUndefined();
    expect(size('height', 1.5)).toBeDefined();
    expect(size('megapixels', 0)).toBeDefined();
    expect(size('megapixels', 1001)).toBeDefined();
    expect(size('megapixels', 0.5)).toBeUndefined();
    expect(size('percent', 0)).toBeDefined();
    expect(size('percent', 401, true)).toBeDefined();
    expect(size('percent', 150)).toBeDefined();
    expect(size('percent', 150, true)).toBeUndefined();
    expect(size('percent', 100)).toBeUndefined();
    expect(size('original', Number.NaN)).toBeUndefined();
  });

  it('checks the suffix only in suffix mode', () => {
    const suffix = (s: string, namingMode: ExportForm['namingMode'] = 'suffix') =>
      validateForm(form({ namingMode, suffix: s }), 1).suffix;
    expect(suffix('a/b')).toBeDefined();
    expect(suffix('a\\b')).toBeDefined();
    expect(suffix('a\nb')).toBeDefined();
    expect(suffix('x'.repeat(65))).toBeDefined();
    expect(suffix('x'.repeat(64))).toBeUndefined();
    expect(suffix('a/b', 'original')).toBeUndefined();
  });
});

describe('storage', () => {
  it('round-trips the form', () => {
    const data = new Map<string, string>();
    const storage = {
      getItem: (k: string) => data.get(k) ?? null,
      setItem: (k: string, v: string) => void data.set(k, v),
    };
    const saved = form({ formatKind: 'tiff', tiffCompression: 'deflate', jpegQuality: 70 });
    saveForm(saved, storage);
    expect(loadForm(storage)).toEqual(saved);
  });

  it('returns the defaults when nothing is stored or storage is missing', () => {
    expect(loadForm({ getItem: () => null })).toEqual(defaultForm());
    expect(loadForm(null)).toEqual(defaultForm());
    expect(() => saveForm(form(), null)).not.toThrow();
  });

  it('warns and returns the defaults for bad JSON', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    expect(loadForm({ getItem: () => '{not json' })).toEqual(defaultForm());
    expect(loadForm({ getItem: () => '[1,2]' })).toEqual(defaultForm());
    expect(warn).toHaveBeenCalledTimes(2);
  });

  it('replaces only the invalid fields', () => {
    const stored = JSON.stringify({ formatKind: 'bmp', jpegQuality: 60, colorSpace: 'rec2020', dither: 'yes' });
    const loaded = loadForm({ getItem: (k) => (k === STORAGE_KEY ? stored : null) });
    expect(loaded.formatKind).toBe('jpeg');
    expect(loaded.jpegQuality).toBe(60);
    expect(loaded.colorSpace).toBe('rec2020');
    expect(loaded.dither).toBe(true);
  });

  it('survives a storage that throws', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const broken = {
      getItem: (): string => {
        throw new Error('denied');
      },
      setItem: (): void => {
        throw new Error('full');
      },
    };
    expect(loadForm(broken)).toEqual(defaultForm());
    expect(() => saveForm(form(), broken)).not.toThrow();
    expect(warn).toHaveBeenCalledTimes(2);
  });
});
