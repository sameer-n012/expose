// @vitest-environment happy-dom
import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { DevelopDocument } from '../../lib/develop/settings';
import { DevelopStore } from '../../lib/stores/develop.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import ResetAllPanel from './ResetAllPanel.svelte';

async function makeStore(): Promise<DevelopStore> {
  const initial: DevelopDocument = {
    version: 1,
    white_balance: { mode: 'as_shot', temperature: 5500, tint: 0 },
    tone: { exposure: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0 },
  };
  const api = {
    getSettings: vi.fn(async (_id: number) => initial),
    setSettings: vi.fn(async (_id: number, s: DevelopDocument) => s),
    openPreview: vi.fn(async () => ({ session: 1, width: 4, height: 3 }) as never),
    closePreview: vi.fn(async (_s: number) => undefined),
  };
  const store = new DevelopStore({
    library: { focusIndex: null, photoAt: () => undefined, selectedIds: [] },
    modules: { active: 'develop' },
    toasts: new ToastStore(),
    api,
  });
  await store.activate(1);
  return store;
}

describe('ResetAllPanel', () => {
  it('is disabled at defaults and resets every panel on click', async () => {
    const store = await makeStore();
    render(ResetAllPanel, { develop: store });
    const button = screen.getByRole<HTMLButtonElement>('button', { name: 'Reset all' });
    expect(button.disabled).toBe(true);

    store.setSlider('exposure', 2);
    store.setCurve('master', [
      { x: 0, y: 0 },
      { x: 0.5, y: 0.8 },
      { x: 1, y: 1 },
    ]);
    store.setSharpening('amount', 60);
    store.setCropAngle(3);
    await Promise.resolve();
    expect(button.disabled).toBe(false);

    await fireEvent.click(button);
    expect(store.settings?.tone.exposure).toBe(0);
    expect(store.settings?.tone_curve?.master).toEqual([
      { x: 0, y: 0 },
      { x: 1, y: 1 },
    ]);
    expect(store.settings?.sharpening?.amount).toBe(0);
    expect(store.crop?.angle).toBe(0);
    expect(button.disabled).toBe(true);
  });
});
