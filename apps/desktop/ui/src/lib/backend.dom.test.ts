import { afterEach, describe, expect, it, vi } from 'vitest';
import { callBackend } from './backend';
import { toasts } from './stores/toasts.svelte';

afterEach(() => {
  toasts.clear();
  vi.restoreAllMocks();
});

describe('callBackend', () => {
  it('returns the result of a successful call', async () => {
    await expect(callBackend('Read the version', () => Promise.resolve('1.0.0'))).resolves.toBe('1.0.0');
    expect(toasts.items).toHaveLength(0);
  });

  it('shows an error toast for a failed call and returns undefined', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const result = await callBackend('Read the version', () => Promise.reject(new Error('no backend')));
    expect(result).toBeUndefined();
    expect(toasts.items).toMatchObject([{ kind: 'error', message: 'Read the version failed.', detail: 'no backend' }]);
  });
});
