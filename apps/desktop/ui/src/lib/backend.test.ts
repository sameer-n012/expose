import { describe, expect, it } from 'vitest';
import { isTauri } from './backend';

describe('isTauri', () => {
  it('returns false outside the Tauri shell', () => {
    expect(isTauri()).toBe(false);
  });
});
