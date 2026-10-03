import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createAppState, type AppState } from '../../lib/appState';
import { DEFAULT_PANEL_WIDTH, PANEL_LIMITS, RESIZE_STEP } from '../../lib/stores/layout.svelte';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import { FakeCatalog, mockElementSize } from '../../test/fakeCatalog';
import AppShell from './AppShell.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

let app: AppState;
let toasts: ToastStore;
let restoreSize: () => void;

/** Renders the shell and waits until the Library toolbar shows (the catalog is ready). */
async function renderShell(): Promise<void> {
  render(AppShell, { modules: app.modules, layout: app.layout, library: app.library, registry: app.registry, toasts });
  await screen.findByRole('toolbar', { name: 'Library toolbar' });
}

function moduleView(id: string): HTMLElement | null {
  return screen.queryByTestId(`module-${id}`);
}

async function press(key: string, init: KeyboardEventInit = {}, target: Window | Element = window): Promise<void> {
  await fireEvent.keyDown(target, { key, ...init });
}

beforeEach(() => {
  vi.mocked(invoke).mockImplementation(new FakeCatalog(5).invoke);
  restoreSize = mockElementSize(1000, 600);
  toasts = new ToastStore();
  app = createAppState({ platform: 'mac', toasts });
});

afterEach(() => {
  app.dispose();
  toasts.clear();
  restoreSize();
});

describe('module switching', () => {
  it('shows the Library module first and mounts only that module', async () => {
    await renderShell();
    expect(moduleView('library')?.hidden).toBe(false);
    expect(moduleView('develop')).toBeNull();
    expect(moduleView('editor')).toBeNull();
  });

  it('switches modules with the D, E, and G keys', async () => {
    await renderShell();
    await press('d');
    expect(app.modules.active).toBe('develop');
    expect(moduleView('develop')?.hidden).toBe(false);
    expect(moduleView('library')?.hidden).toBe(true);

    await press('e');
    expect(moduleView('editor')?.hidden).toBe(false);
    expect(moduleView('develop')?.hidden).toBe(true);

    await press('g');
    expect(moduleView('library')?.hidden).toBe(false);
    expect(moduleView('editor')?.hidden).toBe(true);
  });

  it('switches modules with the top bar buttons', async () => {
    await renderShell();
    await fireEvent.click(screen.getByRole('button', { name: 'Editor' }));
    expect(moduleView('editor')?.hidden).toBe(false);
    expect(screen.getByRole('button', { name: 'Editor' }).getAttribute('aria-current')).toBe('page');
    expect(screen.getByRole('button', { name: 'Library' }).getAttribute('aria-current')).toBeNull();
  });

  it('shows the shortcut in the module button tooltip', async () => {
    await renderShell();
    expect(screen.getByRole('button', { name: 'Develop' }).title).toBe('Develop (D)');
  });

  it('keeps the module view mounted, so that its state stays', async () => {
    await renderShell();
    const search = screen.getByRole('searchbox', { name: 'Search photos' }) as HTMLInputElement;
    await fireEvent.input(search, { target: { value: 'sunset' } });

    await press('d');
    await press('g');

    const again = screen.getByRole('searchbox', { name: 'Search photos' }) as HTMLInputElement;
    expect(again).toBe(search);
    expect(again.value).toBe('sunset');
  });

  it('does not switch while the user types in a text field', async () => {
    await renderShell();
    const search = screen.getByRole('searchbox', { name: 'Search photos' });
    search.focus();
    await press('d', {}, search);
    await press('e', {}, search);
    expect(app.modules.active).toBe('library');
  });

  it('G opens the grid view from the loupe view', async () => {
    await renderShell();
    const cell = await within(screen.getByTestId('library-grid')).findByRole('button', { name: 'photo-3.jpg' });
    await fireEvent.dblClick(cell);
    expect(screen.getByTestId('library-loupe')).toBeTruthy();
    await press('e');
    await press('g');
    expect(screen.getByTestId('library-grid')).toBeTruthy();
  });
});

describe('panel toggles', () => {
  it('Tab toggles the side panels', async () => {
    await renderShell();
    const left = screen.getByTestId('panel-left');
    const right = screen.getByTestId('panel-right');
    expect(left.hidden).toBe(false);

    await press('Tab');
    expect(left.hidden).toBe(true);
    expect(right.hidden).toBe(true);
    expect(screen.getByTestId('filmstrip').hidden).toBe(false);

    await press('Tab');
    expect(left.hidden).toBe(false);
    expect(right.hidden).toBe(false);
  });

  it('Shift+Tab toggles all panels', async () => {
    await renderShell();
    await press('Tab', { shiftKey: true });
    expect(screen.getByTestId('panel-left').hidden).toBe(true);
    expect(screen.getByTestId('panel-right').hidden).toBe(true);
    expect(screen.getByTestId('filmstrip').hidden).toBe(true);
    expect(screen.getByTestId('topbar').parentElement?.hidden).toBe(true);

    await press('Tab', { shiftKey: true });
    expect(screen.getByTestId('filmstrip').hidden).toBe(false);
    expect(screen.getByTestId('topbar').parentElement?.hidden).toBe(false);
  });

  it('prevents the default Tab focus move when it toggles panels', async () => {
    await renderShell();
    const event = new KeyboardEvent('keydown', { key: 'Tab', cancelable: true });
    window.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
  });

  it('shows a hidden panel again with its edge strip', async () => {
    await renderShell();
    await press('Tab');
    await fireEvent.click(screen.getByRole('button', { name: 'Show left panel' }));
    expect(screen.getByTestId('panel-left').hidden).toBe(false);
    expect(screen.getByTestId('panel-right').hidden).toBe(true);
    expect(screen.queryByRole('button', { name: 'Show left panel' })).toBeNull();
  });

  it('toggles a panel with the top bar button', async () => {
    await renderShell();
    const button = screen.getByRole('button', { name: 'Filmstrip' });
    await fireEvent.click(button);
    expect(screen.getByTestId('filmstrip').hidden).toBe(true);
    expect(button.getAttribute('aria-pressed')).toBe('false');
  });
});

describe('panel resize', () => {
  it('resizes with the arrow keys and keeps the width in the limits', async () => {
    await renderShell();
    const handle = screen.getByRole('separator', { name: 'Resize left panel' });
    const panel = screen.getByTestId('panel-left');

    await fireEvent.keyDown(handle, { key: 'ArrowRight' });
    expect(app.layout.width.left).toBe(DEFAULT_PANEL_WIDTH.left + RESIZE_STEP);
    expect(panel.style.width).toBe(`${DEFAULT_PANEL_WIDTH.left + RESIZE_STEP}px`);

    await fireEvent.keyDown(handle, { key: 'End' });
    expect(app.layout.width.left).toBe(PANEL_LIMITS.max);
    await fireEvent.keyDown(handle, { key: 'ArrowRight' });
    expect(app.layout.width.left).toBe(PANEL_LIMITS.max);
    expect(handle.getAttribute('aria-valuenow')).toBe(String(PANEL_LIMITS.max));
  });

  it('resizes the right panel in the other direction', async () => {
    await renderShell();
    const handle = screen.getByRole('separator', { name: 'Resize right panel' });
    await fireEvent.keyDown(handle, { key: 'ArrowLeft' });
    expect(app.layout.width.right).toBe(DEFAULT_PANEL_WIDTH.right + RESIZE_STEP);
  });

  it('resizes with a pointer drag', async () => {
    await renderShell();
    const left = screen.getByRole('separator', { name: 'Resize left panel' });
    await fireEvent.pointerDown(left, { pointerId: 1, button: 0, clientX: 300 });
    await fireEvent.pointerMove(left, { pointerId: 1, clientX: 340 });
    expect(app.layout.width.left).toBe(DEFAULT_PANEL_WIDTH.left + 40);
    await fireEvent.pointerUp(left, { pointerId: 1, clientX: 340 });
    // After the drag ends, pointer moves do not resize.
    await fireEvent.pointerMove(left, { pointerId: 1, clientX: 500 });
    expect(app.layout.width.left).toBe(DEFAULT_PANEL_WIDTH.left + 40);

    // The right panel gets wider when the pointer moves to the left.
    const right = screen.getByRole('separator', { name: 'Resize right panel' });
    await fireEvent.pointerDown(right, { pointerId: 2, button: 0, clientX: 900 });
    await fireEvent.pointerMove(right, { pointerId: 2, clientX: 2000 });
    expect(app.layout.width.right).toBe(PANEL_LIMITS.min);
    await fireEvent.pointerUp(right, { pointerId: 2 });
  });

  it('resets the width with a double-click', async () => {
    await renderShell();
    app.layout.setWidth('left', 400);
    await fireEvent.dblClick(screen.getByRole('separator', { name: 'Resize left panel' }));
    expect(app.layout.width.left).toBe(DEFAULT_PANEL_WIDTH.left);
  });
});

describe('toasts', () => {
  it('shows an error toast for a failed shortcut action, and closes it', async () => {
    app.registry.register({
      id: 'test.fail',
      label: 'Fail',
      scope: 'global',
      defaultBinding: 'F',
      run: () => {
        throw new Error('backend unavailable');
      },
    });
    const originalError = console.error;
    console.error = (): void => undefined;
    try {
      await renderShell();
      await press('f');
    } finally {
      console.error = originalError;
    }
    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('The action "Fail" failed.');
    expect(alert.textContent).toContain('backend unavailable');

    await fireEvent.click(screen.getByRole('button', { name: 'Close notification' }));
    expect(screen.queryByRole('alert')).toBeNull();
  });
});
