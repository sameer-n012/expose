import { describe, expect, it } from 'vitest';
import { DOCUMENT_PRESETS, MAX_PIXELS, defaultSpec, untitledName, validateSpec } from './newDocument';

describe('validateSpec', () => {
  const ok = defaultSpec('Untitled-1');

  it('accepts the default spec and all presets', () => {
    expect(validateSpec(ok)).toBeNull();
    for (const p of DOCUMENT_PRESETS) {
      expect(validateSpec({ ...ok, width: p.width, height: p.height, resolutionPpi: p.resolutionPpi })).toBeNull();
    }
  });

  it('refuses bad names', () => {
    expect(validateSpec({ ...ok, name: '  ' })).not.toBeNull();
    expect(validateSpec({ ...ok, name: 'a'.repeat(121) })).not.toBeNull();
    expect(validateSpec({ ...ok, name: 'a'.repeat(120) })).toBeNull();
    for (const bad of ['a/b', 'a\\b', 'a:b', 'a\nb', 'a\u0000b']) {
      expect(validateSpec({ ...ok, name: bad })).not.toBeNull();
    }
  });

  it('refuses bad sizes', () => {
    expect(validateSpec({ ...ok, width: 0 })).not.toBeNull();
    expect(validateSpec({ ...ok, height: 30001 })).not.toBeNull();
    expect(validateSpec({ ...ok, width: 10.5 })).not.toBeNull();
    expect(validateSpec({ ...ok, width: Number.NaN })).not.toBeNull();
    expect(validateSpec({ ...ok, width: 30000, height: 30000 })).not.toBeNull();
  });

  it('checks the pixel count at the limit', () => {
    expect(validateSpec({ ...ok, width: 16384, height: 16384 })).toBeNull();
    expect(MAX_PIXELS).toBe(16384 * 16384);
    expect(validateSpec({ ...ok, width: 16385, height: 16384 })).not.toBeNull();
  });

  it('refuses bad resolution', () => {
    expect(validateSpec({ ...ok, resolutionPpi: 0 })).not.toBeNull();
    expect(validateSpec({ ...ok, resolutionPpi: 10001 })).not.toBeNull();
    expect(validateSpec({ ...ok, resolutionPpi: Number.NaN })).not.toBeNull();
  });

  it('names untitled documents', () => {
    expect(untitledName(3)).toBe('Untitled-3');
  });
});
