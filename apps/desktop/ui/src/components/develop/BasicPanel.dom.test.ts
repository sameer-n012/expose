/** DOM tests of the Basic panel: labels, values, edits, white balance mode, and reset. */
import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { DevelopDocument } from '../../lib/develop/settings';
import { DevelopStore, type DevelopApi } from '../../lib/stores/develop.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import BasicPanel from './BasicPanel.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const INITIAL: DevelopDocument = {
  version: 1,
  white_balance: { mode: 'as_shot', temperature: 5500, tint: 0 },
  tone: { exposure: 0.35, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0 },
};

async function makeStore(): Promise<{ store: DevelopStore; api: DevelopApi }> {
  const api: DevelopApi = {
    getSettings: async () => INITIAL,
    setSettings: vi.fn(async (_id, s) => s),
    openPreview: async () => ({ session: 1, width: 4, height: 3 }) as never,
    closePreview: async () => undefined,
  };
  const store = new DevelopStore({
    library: { focusIndex: null, photoAt: () => undefined, selectedIds: [] },
    modules: { active: 'develop' },
    toasts: new ToastStore(),
    api,
  });
  await store.activate(1);
  return { store, api };
}

describe('BasicPanel', () => {
  it('has a labelled range and number input for each slider', async () => {
    const { store } = await makeStore();
    render(BasicPanel, { develop: store });
    for (const name of ['Temperature', 'Tint', 'Exposure', 'Contrast', 'Highlights', 'Shadows', 'Whites', 'Blacks']) {
      expect(screen.getByRole('slider', { name })).toBeTruthy();
      expect(screen.getByRole('spinbutton', { name: `${name} value` })).toBeTruthy();
    }
    const exposure = screen.getByRole<HTMLInputElement>('spinbutton', { name: 'Exposure value' });
    expect(exposure.value).toBe('0.35');
    expect(screen.getByText('EV')).toBeTruthy();
  });

  it('disables the white balance sliders as shot and enables them in custom', async () => {
    const { store } = await makeStore();
    render(BasicPanel, { develop: store });
    expect(screen.getByRole<HTMLInputElement>('slider', { name: 'Temperature' }).disabled).toBe(true);
    await fireEvent.change(screen.getByLabelText('White balance'), { target: { value: 'custom' } });
    expect(screen.getByRole<HTMLInputElement>('slider', { name: 'Temperature' }).disabled).toBe(false);
  });

  it('edits a slider from the range and from the number input', async () => {
    const { store } = await makeStore();
    render(BasicPanel, { develop: store });
    await fireEvent.input(screen.getByRole('slider', { name: 'Contrast' }), { target: { value: '25' } });
    expect(store.settings?.tone.contrast).toBe(25);
    await fireEvent.change(screen.getByRole('spinbutton', { name: 'Exposure value' }), { target: { value: '9' } });
    expect(store.settings?.tone.exposure).toBe(5);
    expect(screen.getByRole<HTMLInputElement>('spinbutton', { name: 'Exposure value' }).value).toBe('5.00');
  });

  it('resets a slider on double-click and all sliders with the button', async () => {
    const { store } = await makeStore();
    render(BasicPanel, { develop: store });
    await fireEvent.dblClick(screen.getByRole('slider', { name: 'Exposure' }));
    expect(store.settings?.tone.exposure).toBe(0);
    await fireEvent.input(screen.getByRole('slider', { name: 'Shadows' }), { target: { value: '40' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Reset' }));
    expect(store.settings?.tone.shadows).toBe(0);
  });
});
