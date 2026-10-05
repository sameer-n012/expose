/**
 * DOM tests of the navigation between grid and loupe: view toggle, Esc, G,
 * L, arrow keys, module tab, and the shortcuts that act on the loupe photo.
 */
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createAppState, type AppState } from '../../lib/appState';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import { FakeCatalog, mockElementSize } from '../../test/fakeCatalog';
import AppShell from '../shell/AppShell.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

let app: AppState;
let toasts: ToastStore;
let fake: FakeCatalog;
let restoreSize: () => void;

async function renderLibrary(): Promise<void> {
  render(AppShell, { modules: app.modules, layout: app.layout, library: app.library, registry: app.registry, toasts });
  await screen.findByRole('button', { name: 'photo-1.jpg' });
}

const press = (key: string): Promise<boolean> => fireEvent.keyDown(window, { key });
const toggle = (name: 'Grid' | 'Loupe'): HTMLButtonElement =>
  within(screen.getByRole('group', { name: 'View mode' })).getByRole('button', { name }) as HTMLButtonElement;

beforeEach(() => {
  vi.spyOn(console, 'error').mockImplementation(() => undefined);
  fake = new FakeCatalog(30);
  vi.mocked(invoke).mockImplementation(fake.invoke);
  restoreSize = mockElementSize(1000, 600);
  toasts = new ToastStore();
  app = createAppState({ platform: 'mac', toasts });
});

afterEach(() => {
  app.dispose();
  toasts.clear();
  restoreSize();
  vi.restoreAllMocks();
});

describe('view toggle', () => {
  it('marks the grid as pressed and disables the loupe without a selection', async () => {
    await renderLibrary();
    expect(toggle('Grid').getAttribute('aria-pressed')).toBe('true');
    expect(toggle('Loupe').getAttribute('aria-pressed')).toBe('false');
    expect(toggle('Loupe').disabled).toBe(true);
    expect(toggle('Grid').title).toContain('G');
    expect(toggle('Loupe').title).toContain('L');
  });

  it('opens the loupe on the selected photo and goes back to the grid', async () => {
    await renderLibrary();
    await fireEvent.click(screen.getByRole('button', { name: 'photo-3.jpg' }));
    expect(toggle('Loupe').disabled).toBe(false);
    await fireEvent.click(toggle('Loupe'));
    expect(screen.getByTestId('library-loupe').textContent).toContain('photo-3.jpg');
    expect(toggle('Loupe').getAttribute('aria-pressed')).toBe('true');
    await fireEvent.click(toggle('Grid'));
    expect(screen.getByTestId('library-grid')).toBeTruthy();
  });
});

describe('returning to the grid', () => {
  async function openLoupeOn(name: string): Promise<void> {
    await fireEvent.dblClick(screen.getByRole('button', { name }));
    expect(screen.getByTestId('library-loupe')).toBeTruthy();
  }

  it('Esc, G, and the Library tab return to the grid', async () => {
    await renderLibrary();
    for (const back of [
      () => press('Escape'),
      () => press('g'),
      () => fireEvent.click(within(screen.getByRole('navigation', { name: 'Modules' })).getByRole('button', { name: 'Library' })),
    ]) {
      await openLoupeOn('photo-2.jpg');
      await back();
      expect(screen.getByTestId('library-grid')).toBeTruthy();
      expect(screen.queryByTestId('library-loupe')).toBeNull();
    }
  });

  it('L opens the loupe on the selection', async () => {
    await renderLibrary();
    await press('l');
    expect(screen.queryByTestId('library-loupe')).toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'photo-5.jpg' }));
    await press('l');
    expect(screen.getByTestId('library-loupe').textContent).toContain('photo-5.jpg');
  });
});

describe('loupe navigation', () => {
  it('steps with the arrow keys, clamps at the ends, and keeps the selection', async () => {
    await renderLibrary();
    await fireEvent.dblClick(screen.getByRole('button', { name: 'photo-1.jpg' }));
    await press('ArrowLeft');
    expect(screen.getByTestId('library-loupe').textContent).toContain('photo-1.jpg');
    await press('ArrowRight');
    await press('ArrowRight');
    expect(screen.getByTestId('library-loupe').textContent).toContain('photo-3.jpg');
    // Up and down do nothing in the loupe.
    await press('ArrowDown');
    expect(screen.getByTestId('library-loupe').textContent).toContain('photo-3.jpg');
    await press('ArrowLeft');
    await press('Escape');
    const selected = app.library.photoAt(app.library.focusIndex ?? -1);
    expect(selected?.fileName).toBe('photo-2.jpg');
    expect(app.library.selectedIds).toEqual([selected?.id]);
  });

  it('keeps the arrow keys of the grid', async () => {
    await renderLibrary();
    await fireEvent.click(screen.getByRole('button', { name: 'photo-1.jpg' }));
    await press('ArrowDown');
    expect(app.library.focusIndex).toBe(5);
  });

  it('rating and flag shortcuts act on the loupe photo', async () => {
    await renderLibrary();
    await fireEvent.dblClick(screen.getByRole('button', { name: 'photo-1.jpg' }));
    await press('ArrowRight');
    await press('4');
    await press('p');
    await waitFor(() => expect(fake.callsOf('set_rating')).toHaveLength(1));
    expect(fake.callsOf('set_rating')[0]?.args).toEqual({ ids: [2], rating: 4 });
    await waitFor(() => expect(fake.callsOf('set_flag')).toHaveLength(1));
    expect(fake.callsOf('set_flag')[0]?.args).toMatchObject({ ids: [2], flag: 'pick' });
  });
});
