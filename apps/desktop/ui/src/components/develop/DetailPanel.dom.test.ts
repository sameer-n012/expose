/** DOM tests of the Detail panel: labels, edits, dimming, and reset. */
import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { DevelopDocument } from '../../lib/develop/settings';
import { DevelopStore, type DevelopApi } from '../../lib/stores/develop.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import DetailPanel from './DetailPanel.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

// No `sharpening` field: an old document.
const INITIAL: DevelopDocument = {
  version: 1,
  white_balance: { mode: 'as_shot', temperature: 5500, tint: 0 },
  tone: { exposure: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0 },
};

async function makeStore(): Promise<DevelopStore> {
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
  return store;
}

describe('DetailPanel', () => {
  it('shows the defaults for an old document', async () => {
    render(DetailPanel, { develop: await makeStore() });
    expect(screen.getByRole<HTMLInputElement>('spinbutton', { name: 'Amount value' }).value).toBe('0');
    expect(screen.getByRole<HTMLInputElement>('spinbutton', { name: 'Radius value' }).value).toBe('1.0');
    expect(screen.getByRole<HTMLInputElement>('spinbutton', { name: 'Detail value' }).value).toBe('25');
    expect(screen.getByRole<HTMLInputElement>('spinbutton', { name: 'Masking value' }).value).toBe('0');
  });

  it('dims but does not disable radius, detail, and masking at amount 0', async () => {
    const { container } = render(DetailPanel, { develop: await makeStore() });
    const radius = screen.getByRole<HTMLInputElement>('slider', { name: 'Radius' });
    expect(radius.disabled).toBe(false);
    expect(radius.closest('.row')?.classList.contains('dimmed')).toBe(true);
    expect(screen.getByRole('slider', { name: 'Amount' }).closest('.row')?.classList.contains('dimmed')).toBe(false);
    await fireEvent.input(screen.getByRole('slider', { name: 'Amount' }), { target: { value: '50' } });
    expect(container.querySelectorAll('.dimmed').length).toBe(0);
  });

  it('updates the store from the sliders', async () => {
    const store = await makeStore();
    render(DetailPanel, { develop: store });
    await fireEvent.input(screen.getByRole('slider', { name: 'Amount' }), { target: { value: '80' } });
    await fireEvent.input(screen.getByRole('slider', { name: 'Radius' }), { target: { value: '1.5' } });
    expect(store.settings?.sharpening).toEqual({ amount: 80, radius: 1.5, detail: 25, masking: 0 });
  });

  it('resets one slider and all four values', async () => {
    const store = await makeStore();
    render(DetailPanel, { develop: store });
    await fireEvent.input(screen.getByRole('slider', { name: 'Amount' }), { target: { value: '80' } });
    await fireEvent.input(screen.getByRole('slider', { name: 'Masking' }), { target: { value: '40' } });
    await fireEvent.dblClick(screen.getByRole('slider', { name: 'Amount' }));
    expect(store.settings?.sharpening?.amount).toBe(0);
    expect(store.settings?.sharpening?.masking).toBe(40);
    await fireEvent.input(screen.getByRole('slider', { name: 'Detail' }), { target: { value: '70' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Reset' }));
    expect(store.settings?.sharpening).toEqual({ amount: 0, radius: 1, detail: 25, masking: 0 });
  });
});
