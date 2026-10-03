import { describe, expect, it } from 'vitest';
import { isCommandError } from './catalog';

describe('isCommandError', () => {
  it('accepts a structured error', () => {
    expect(isCommandError({ kind: 'notFound', message: 'No photo 3.' })).toBe(true);
  });

  it('rejects a plain string error from Tauri', () => {
    expect(isCommandError('invalid args `query` for command `list_photos`')).toBe(false);
  });

  it('rejects null and objects without the fields', () => {
    expect(isCommandError(null)).toBe(false);
    expect(isCommandError({ kind: 'io' })).toBe(false);
  });
});
