import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import tokensCss from '../../styles/tokens.css?raw';
import { DEFAULT_PANEL_WIDTH, LayoutStore, PANEL_LIMITS, clampPanelWidth } from './layout.svelte';
import { ModuleStore } from './modules.svelte';
import { MAX_TOASTS, ToastStore, describeError } from './toasts.svelte';

describe('ModuleStore', () => {
  it('starts in the given module and mounts only that module', () => {
    const store = new ModuleStore('develop');
    expect(store.active).toBe('develop');
    expect(store.mounted).toEqual(['develop']);
  });

  it('mounts a module on the first visit and keeps it mounted', () => {
    const store = new ModuleStore();
    store.switchTo('editor');
    store.switchTo('library');
    expect(store.active).toBe('library');
    expect(store.mounted).toEqual(['library', 'editor']);
    expect(store.isMounted('develop')).toBe(false);
  });

  it('opens the library grid', () => {
    const store = new ModuleStore('editor');
    store.libraryView = 'loupe';
    store.showLibraryGrid();
    expect(store.active).toBe('library');
    expect(store.libraryView).toBe('grid');
  });
});

describe('LayoutStore', () => {
  it('toggles the side panels together', () => {
    const layout = new LayoutStore();
    layout.toggleSides();
    expect(layout.visible).toMatchObject({ left: false, right: false, topbar: true, filmstrip: true });
    layout.toggleSides();
    expect(layout.visible).toMatchObject({ left: true, right: true });
  });

  it('hides both side panels if only one is visible', () => {
    const layout = new LayoutStore();
    layout.toggle('left');
    layout.toggleSides();
    expect(layout.visible).toMatchObject({ left: false, right: false });
  });

  it('toggles all panels together', () => {
    const layout = new LayoutStore();
    layout.toggle('right');
    layout.toggleAll();
    expect(Object.values(layout.visible).every((v) => !v)).toBe(true);
    layout.toggleAll();
    expect(Object.values(layout.visible).every((v) => v)).toBe(true);
  });

  it('limits panel widths', () => {
    const layout = new LayoutStore();
    layout.setWidth('left', 10);
    expect(layout.width.left).toBe(PANEL_LIMITS.min);
    layout.setWidth('left', 10_000);
    expect(layout.width.left).toBe(PANEL_LIMITS.max);
    layout.setWidth('right', 321.6);
    expect(layout.width.right).toBe(322);
    layout.resetWidth('right');
    expect(layout.width.right).toBe(DEFAULT_PANEL_WIDTH.right);
    expect(clampPanelWidth(Number.NaN)).toBe(PANEL_LIMITS.min);
  });

  it('uses the same sizes as the theme tokens', () => {
    const px = (name: string): number => {
      const match = new RegExp(`--${name}:\\s*(\\d+)px`).exec(tokensCss);
      if (match?.[1] === undefined) {
        throw new Error(`Token --${name} is missing in tokens.css.`);
      }
      return Number(match[1]);
    };
    expect(px('panel-min-width')).toBe(PANEL_LIMITS.min);
    expect(px('panel-max-width')).toBe(PANEL_LIMITS.max);
    expect(px('panel-left-width')).toBe(DEFAULT_PANEL_WIDTH.left);
    expect(px('panel-right-width')).toBe(DEFAULT_PANEL_WIDTH.right);
  });
});

describe('ToastStore', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('closes info toasts after the timeout', () => {
    const store = new ToastStore();
    store.info('Saved');
    expect(store.items).toHaveLength(1);
    vi.advanceTimersByTime(4000);
    expect(store.items).toHaveLength(0);
  });

  it('keeps error toasts until the user closes them', () => {
    const store = new ToastStore();
    const id = store.error('Import failed.', new Error('disk full'));
    vi.advanceTimersByTime(60_000);
    expect(store.items).toEqual([{ id, kind: 'error', message: 'Import failed.', detail: 'disk full' }]);
    expect(console.error).toHaveBeenCalled();
    store.dismiss(id);
    expect(store.items).toHaveLength(0);
  });

  it('closes the oldest toast when there are too many', () => {
    const store = new ToastStore();
    const first = store.error('first');
    for (let i = 0; i < MAX_TOASTS; i++) {
      store.error(`more ${i}`);
    }
    expect(store.items).toHaveLength(MAX_TOASTS);
    expect(store.items.some((t) => t.id === first)).toBe(false);
  });

  it('clears all toasts and their timers', () => {
    const store = new ToastStore();
    store.info('a');
    store.error('b');
    store.clear();
    expect(store.items).toHaveLength(0);
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe('describeError', () => {
  it('describes errors, strings, and other values', () => {
    expect(describeError(new Error('x'))).toBe('x');
    expect(describeError('plain')).toBe('plain');
    expect(describeError({ code: 3 })).toBe('{"code":3}');
    expect(describeError(undefined)).toBe('undefined');
    const circular: Record<string, unknown> = {};
    circular['self'] = circular;
    expect(describeError(circular)).toContain('[object Object]');
  });
});
