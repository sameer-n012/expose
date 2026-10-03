/**
 * DOM tests of the Library module: folder tree, grid, toolbar, selection,
 * shortcuts, and import. The `invoke` function of Tauri is replaced by a
 * fake catalog.
 */
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createAppState, type AppState } from '../../lib/appState';
import { formatCaptureTime } from '../../lib/library/dateFormat';
import { ToastStore } from '../../lib/stores/toasts.svelte';
import { FakeCatalog, makeFolder, makePhoto, mockElementSize } from '../../test/fakeCatalog';
import AppShell from '../shell/AppShell.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

let app: AppState;
let toasts: ToastStore;
let fake: FakeCatalog;
let restoreSize: () => void;

/** Renders the whole shell (it handles the keys) and waits for the first photos. */
async function renderLibrary(waitForPhotos = true): Promise<void> {
  render(AppShell, { modules: app.modules, layout: app.layout, library: app.library, registry: app.registry, toasts });
  await screen.findByRole('toolbar', { name: 'Library toolbar' });
  if (waitForPhotos) {
    await screen.findByRole('button', { name: 'photo-1.jpg' });
  }
}

function useFake(next: FakeCatalog): void {
  fake = next;
  vi.mocked(invoke).mockImplementation(fake.invoke);
}

function cell(name: string): HTMLElement {
  return screen.getByRole('button', { name });
}

function cellCount(): number {
  return within(screen.getByTestId('library-grid')).queryAllByRole('button').length;
}

async function press(key: string, init: KeyboardEventInit = {}): Promise<void> {
  await fireEvent.keyDown(window, { key, ...init });
}

beforeEach(() => {
  vi.spyOn(console, 'error').mockImplementation(() => undefined);
  useFake(new FakeCatalog(30));
  // 1000 x 600 px viewport: 5 columns, 3 to 4 rows in view.
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

describe('catalog status', () => {
  it('shows the error instead of the grid if the catalog is unavailable', async () => {
    fake.status = { state: 'unavailable', path: null, error: 'The database file is locked.' };
    render(AppShell, { modules: app.modules, layout: app.layout, library: app.library, registry: app.registry, toasts });
    const alert = await screen.findByTestId('catalog-unavailable');
    expect(alert.textContent).toContain('The database file is locked.');
    expect(screen.queryByTestId('library-grid')).toBeNull();
    expect(fake.callsOf('list_photos')).toHaveLength(0);
  });

  it('shows an error and a toast if the status cannot be read', async () => {
    fake.failing.add('catalog_status');
    render(AppShell, { modules: app.modules, layout: app.layout, library: app.library, registry: app.registry, toasts });
    await screen.findByTestId('catalog-unavailable');
    expect(toasts.items.map((t) => t.message)).toContain('Read the catalog status failed.');
  });
});

describe('grid', () => {
  it('loads the first page of the first folder', async () => {
    await renderLibrary();
    const [first] = fake.callsOf('list_photos');
    expect(first?.args['query']).toMatchObject({
      folderId: 1,
      recursive: true,
      sort: { field: 'captureTime', direction: 'descending' },
      limit: 200,
      offset: 0,
    });
  });

  it('shows name, rating, flag, color label, and capture time of a photo', async () => {
    useFake(
      Object.assign(new FakeCatalog(3), {
        photos: [
          makePhoto(1, { rating: 3, flag: 'pick', colorLabel: 'red' }),
          makePhoto(2, { captureTimeMs: null }),
          makePhoto(3),
        ],
      }),
    );
    await renderLibrary();
    const first = within(cell('photo-1.jpg'));
    expect(first.getByTestId('rating').textContent).toBe('★★★☆☆');
    expect(first.getByTestId('flag').title).toBe('Pick');
    expect(first.getByTestId('color-label').title).toBe('Label: red');
    // Capture time is shown as stored (UTC): 12:01 camera time, whatever the local zone is.
    expect(cell('photo-1.jpg').textContent).toContain(formatCaptureTime(makePhoto(1).captureTimeMs));
    expect(cell('photo-1.jpg').textContent).toContain('12:01');
    expect(cell('photo-2.jpg').textContent).toContain('—');
    expect(within(cell('photo-3.jpg')).queryByTestId('flag')).toBeNull();
  });

  it('renders only the cells near the viewport, not all 1000 photos', async () => {
    useFake(new FakeCatalog(1000));
    await renderLibrary();
    expect(cellCount()).toBeGreaterThan(0);
    expect(cellCount()).toBeLessThan(60);
    expect(screen.getByText('1000 photos')).toBeTruthy();
  });

  it('loads the next page when the user scrolls near it', async () => {
    useFake(new FakeCatalog(1000));
    await renderLibrary();
    const offsets = (): unknown[] => fake.callsOf('list_photos').map((c) => (c.args['query'] as { offset: number }).offset);
    expect(offsets()).toEqual([0]);

    const grid = screen.getByTestId('library-grid');
    // Row height is about 197 px. Page 1 starts at row 40 (200 photos / 5 columns).
    grid.scrollTop = 45 * 197;
    await fireEvent.scroll(grid);
    await screen.findAllByRole('button', { name: /^photo-2\d\d\.jpg$/ });
    expect(offsets()).toContain(200);
    expect(offsets()).not.toContain(600);
  });

  it('shows a message for an empty catalog', async () => {
    useFake(new FakeCatalog(0, []));
    await renderLibrary(false);
    expect(await screen.findByText(/The catalog has no photos/)).toBeTruthy();
    expect(fake.callsOf('list_photos')).toHaveLength(0);
  });

  it('shows a retry button when a page fails, and loads again', async () => {
    fake.failing.add('list_photos');
    await renderLibrary(false);
    const retry = await screen.findByRole('button', { name: 'Retry' });
    expect(toasts.items[0]?.message).toBe('Load photos failed.');
    fake.failing.delete('list_photos');
    await fireEvent.click(retry);
    await screen.findByRole('button', { name: 'photo-1.jpg' });
    expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull();
  });

  it('opens the loupe view with a double-click', async () => {
    await renderLibrary();
    await fireEvent.dblClick(cell('photo-4.jpg'));
    expect(screen.getByTestId('library-loupe').textContent).toContain('photo-4.jpg');
  });
});

describe('sort and filter', () => {
  it('starts a new query with the chosen sort', async () => {
    await renderLibrary();
    await fireEvent.change(screen.getByRole('combobox', { name: 'Sort' }), { target: { value: 'fileName' } });
    await fireEvent.click(screen.getByRole('button', { name: /Sort direction/ }));
    await waitFor(() => {
      const last = fake.callsOf('list_photos').at(-1);
      expect(last?.args['query']).toMatchObject({ sort: { field: 'fileName', direction: 'ascending' }, offset: 0 });
    });
  });

  it('sends the filter to the backend', async () => {
    await renderLibrary();
    await fireEvent.change(screen.getByRole('combobox', { name: 'Rating' }), { target: { value: '3' } });
    await fireEvent.change(screen.getByRole('combobox', { name: 'Flag' }), { target: { value: 'pick' } });
    await fireEvent.change(screen.getByRole('combobox', { name: 'Label' }), { target: { value: 'purple' } });
    await waitFor(() => {
      const last = fake.callsOf('list_photos').at(-1);
      expect(last?.args['query']).toMatchObject({ filter: { minRating: 3, flag: 'pick', colorLabel: 'purple' } });
    });
  });

  it('sends no filter values for "any"', async () => {
    await renderLibrary();
    const query = fake.callsOf('list_photos')[0]?.args['query'];
    expect(query).toMatchObject({ filter: { minRating: null, flag: null, colorLabel: null } });
  });
});

describe('selection', () => {
  const pressed = (name: string): string | null => cell(name).getAttribute('aria-pressed');

  it('selects with click, toggles with Cmd+click, and selects a range with Shift+click', async () => {
    await renderLibrary();
    await fireEvent.click(cell('photo-2.jpg'));
    expect(pressed('photo-2.jpg')).toBe('true');

    await fireEvent.click(cell('photo-4.jpg'), { metaKey: true });
    expect(pressed('photo-2.jpg')).toBe('true');
    expect(pressed('photo-4.jpg')).toBe('true');
    expect(pressed('photo-3.jpg')).toBe('false');

    await fireEvent.click(cell('photo-4.jpg'), { ctrlKey: true });
    expect(pressed('photo-4.jpg')).toBe('false');

    await fireEvent.click(cell('photo-3.jpg'));
    await fireEvent.click(cell('photo-5.jpg'), { shiftKey: true });
    expect(['photo-3.jpg', 'photo-4.jpg', 'photo-5.jpg'].map(pressed)).toEqual(['true', 'true', 'true']);
    expect(pressed('photo-2.jpg')).toBe('false');
    expect(screen.getByText('30 photos, 3 selected')).toBeTruthy();
  });

  it('moves the selection with the arrow keys; up and down move one row', async () => {
    await renderLibrary();
    await fireEvent.click(cell('photo-1.jpg'));
    await press('ArrowRight');
    expect(pressed('photo-2.jpg')).toBe('true');
    expect(pressed('photo-1.jpg')).toBe('false');
    await press('ArrowDown');
    // 5 columns: one row down from photo 2 is photo 7.
    expect(pressed('photo-7.jpg')).toBe('true');
    await press('ArrowLeft');
    expect(pressed('photo-6.jpg')).toBe('true');
    await press('ArrowRight', { shiftKey: true });
    expect(pressed('photo-6.jpg')).toBe('true');
    expect(pressed('photo-7.jpg')).toBe('true');
  });

  it('selects all loaded photos with Cmd+A', async () => {
    await renderLibrary();
    await press('a', { metaKey: true });
    expect(screen.getByText('30 photos, 30 selected')).toBeTruthy();
  });

  it('does not select all while the user types in the search field', async () => {
    await renderLibrary();
    const search = screen.getByRole('searchbox', { name: 'Search photos' });
    await fireEvent.keyDown(search, { key: 'a', metaKey: true });
    expect(screen.queryByText('30 photos, 30 selected')).toBeNull();
  });
});

describe('rating, flag, and label shortcuts', () => {
  it('sets the rating of the selection and calls the backend once', async () => {
    await renderLibrary();
    await fireEvent.click(cell('photo-1.jpg'));
    await fireEvent.click(cell('photo-2.jpg'), { metaKey: true });
    await press('4');
    // The cell shows the new value at once, before the backend answers.
    expect(within(cell('photo-1.jpg')).getByTestId('rating').textContent).toBe('★★★★☆');
    expect(within(cell('photo-2.jpg')).getByTestId('rating').textContent).toBe('★★★★☆');
    expect(within(cell('photo-3.jpg')).getByTestId('rating').textContent).toBe('☆☆☆☆☆');
    await waitFor(() => expect(fake.callsOf('set_rating')).toHaveLength(1));
    expect(fake.callsOf('set_rating')[0]?.args).toEqual({ ids: [1, 2], rating: 4 });
  });

  it('sets flags and color labels', async () => {
    await renderLibrary();
    await fireEvent.click(cell('photo-1.jpg'));
    await press('p');
    await press('7');
    await waitFor(() => {
      expect(fake.callsOf('set_flag')[0]?.args).toEqual({ ids: [1], flag: 'pick' });
      expect(fake.callsOf('set_color_label')[0]?.args).toEqual({ ids: [1], label: 'yellow' });
    });
    expect(within(cell('photo-1.jpg')).getByTestId('flag').title).toBe('Pick');
    await press('u');
    expect(within(cell('photo-1.jpg')).queryByTestId('flag')).toBeNull();
    await press('x');
    expect(within(cell('photo-1.jpg')).getByTestId('flag').title).toBe('Reject');
  });

  it('sets the purple label with the toolbar button', async () => {
    await renderLibrary();
    expect((screen.getByRole('button', { name: 'Set label purple' }) as HTMLButtonElement).disabled).toBe(true);
    await fireEvent.click(cell('photo-1.jpg'));
    await fireEvent.click(screen.getByRole('button', { name: 'Set label purple' }));
    await waitFor(() => expect(fake.callsOf('set_color_label')[0]?.args).toEqual({ ids: [1], label: 'purple' }));
    expect(within(cell('photo-1.jpg')).getByTestId('color-label').title).toBe('Label: purple');
  });

  it('rolls the change back and shows a toast if the backend fails', async () => {
    useFake(Object.assign(new FakeCatalog(3), { photos: [makePhoto(1, { rating: 2 }), makePhoto(2), makePhoto(3)] }));
    fake.failing.add('set_rating');
    await renderLibrary();
    await fireEvent.click(cell('photo-1.jpg'));
    await press('5');
    await waitFor(() => expect(toasts.items).toHaveLength(1));
    expect(toasts.items[0]).toMatchObject({ kind: 'error', message: 'Set the rating failed.' });
    expect(within(cell('photo-1.jpg')).getByTestId('rating').textContent).toBe('★★☆☆☆');
  });

  it('does nothing without a selection', async () => {
    await renderLibrary();
    await press('3');
    expect(fake.callsOf('set_rating')).toHaveLength(0);
  });

  it('does not act while another module is active', async () => {
    await renderLibrary();
    await fireEvent.click(cell('photo-1.jpg'));
    app.modules.switchTo('develop');
    await press('3');
    expect(fake.callsOf('set_rating')).toHaveLength(0);
  });

  it('does not act while the user types in the search field', async () => {
    await renderLibrary();
    await fireEvent.click(cell('photo-1.jpg'));
    await fireEvent.keyDown(screen.getByRole('searchbox', { name: 'Search photos' }), { key: '3' });
    expect(fake.callsOf('set_rating')).toHaveLength(0);
  });
});

describe('import', () => {
  it('imports the picked folder, shows the report, and loads the tree and grid again', async () => {
    fake.report = { root: '/photos/new', added: 12, skipped: 3, ignored: 0, addedIds: [], errors: [] };
    await renderLibrary();
    const before = fake.callsOf('folder_tree').length;
    await fireEvent.click(screen.getByRole('button', { name: 'Import folder…' }));
    await waitFor(() => expect(toasts.items).toHaveLength(1));
    expect(toasts.items[0]).toMatchObject({ kind: 'success', message: 'Imported 12 photos. Skipped 3. Errors: 0.' });
    expect(fake.callsOf('import_folder')[0]?.args).toEqual({ path: '/photos/new', recursive: true });
    await waitFor(() => expect(fake.callsOf('folder_tree').length).toBe(before + 1));
  });

  it('shows a warning with the first error', async () => {
    fake.report = {
      root: '/photos/new',
      added: 1,
      skipped: 0,
      ignored: 0,
      addedIds: [],
      errors: [{ path: '/photos/new/bad.jpg', message: 'Unreadable file.' }],
    };
    await renderLibrary();
    await fireEvent.click(screen.getByRole('button', { name: 'Import folder…' }));
    await waitFor(() => expect(toasts.items).toHaveLength(1));
    expect(toasts.items[0]).toMatchObject({ kind: 'warning', detail: '/photos/new/bad.jpg: Unreadable file.' });
  });

  it('does nothing if the user cancels the picker', async () => {
    fake.pickedPath = null;
    await renderLibrary();
    await fireEvent.click(screen.getByRole('button', { name: 'Import folder…' }));
    await waitFor(() => expect(fake.callsOf('pick_folder')).toHaveLength(1));
    expect(fake.callsOf('import_folder')).toHaveLength(0);
    expect(toasts.items).toHaveLength(0);
    expect((screen.getByRole('button', { name: 'Import folder…' }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('disables the button while the import runs', async () => {
    let finish: (value: unknown) => void = () => undefined;
    const inner = fake.invoke;
    vi.mocked(invoke).mockImplementation((command, args) =>
      command === 'import_folder'
        ? new Promise((resolve) => {
            finish = resolve;
          })
        : inner(command, args),
    );
    await renderLibrary();
    await fireEvent.click(screen.getByRole('button', { name: 'Import folder…' }));
    const busy = await screen.findByRole('button', { name: 'Importing…' });
    expect((busy as HTMLButtonElement).disabled).toBe(true);
    finish({ root: '/x', added: 0, skipped: 0, ignored: 0, addedIds: [], errors: [] });
    await screen.findByRole('button', { name: 'Import folder…' });
    expect((screen.getByRole('button', { name: 'Import folder…' }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('shows a toast if the import fails', async () => {
    fake.failing.add('import_folder');
    await renderLibrary();
    await fireEvent.click(screen.getByRole('button', { name: 'Import folder…' }));
    await waitFor(() => expect(toasts.items[0]?.message).toBe('Import photos failed.'));
    expect((screen.getByRole('button', { name: 'Import folder…' }) as HTMLButtonElement).disabled).toBe(false);
  });
});

describe('folder tree', () => {
  beforeEach(() => {
    useFake(
      new FakeCatalog(10, [
        makeFolder(1, 'Trips', 4, [makeFolder(2, 'Spain', 3), makeFolder(3, 'Italy', 2, [makeFolder(5, 'Rome', 1)])]),
        makeFolder(4, 'Family', 7),
      ]),
    );
  });

  const row = (name: string): HTMLElement => screen.getByRole('treeitem', { name: new RegExp(name) });

  it('shows the folders with their photo counts', async () => {
    await renderLibrary();
    expect(row('Trips').textContent).toContain('4');
    expect(row('Family').textContent).toContain('7');
    expect(row('Spain').getAttribute('aria-level')).toBe('2');
  });

  it('selects the first folder at the start and loads its photos', async () => {
    await renderLibrary();
    expect(row('Trips').getAttribute('aria-selected')).toBe('true');
  });

  it('loads the photos of a folder that the user selects', async () => {
    await renderLibrary();
    await fireEvent.click(row('Family'));
    expect(row('Family').getAttribute('aria-selected')).toBe('true');
    await waitFor(() => expect(fake.callsOf('list_photos').at(-1)?.args['query']).toMatchObject({ folderId: 4, offset: 0 }));
  });

  it('collapses and expands a folder', async () => {
    await renderLibrary();
    expect(screen.queryByRole('treeitem', { name: /Rome/ })).not.toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Collapse Trips' }));
    expect(screen.queryByRole('treeitem', { name: /Spain/ })).toBeNull();
    expect(row('Trips').getAttribute('aria-expanded')).toBe('false');
    // A click on the twisty does not select the folder.
    expect(row('Trips').getAttribute('aria-selected')).toBe('true');
    await fireEvent.click(screen.getByRole('button', { name: 'Expand Trips' }));
    expect(screen.queryByRole('treeitem', { name: /Spain/ })).not.toBeNull();
  });

  it('uses the arrow keys in the tree and keeps them from the grid', async () => {
    await renderLibrary();
    await fireEvent.click(row('Italy'));
    await fireEvent.keyDown(row('Italy'), { key: 'ArrowLeft' });
    expect(row('Italy').getAttribute('aria-expanded')).toBe('false');
    await fireEvent.keyDown(row('Italy'), { key: 'ArrowRight' });
    expect(row('Italy').getAttribute('aria-expanded')).toBe('true');
    await fireEvent.keyDown(row('Italy'), { key: 'ArrowDown' });
    expect(row('Rome').getAttribute('aria-selected')).toBe('true');
    // The grid has no selection: its arrow shortcut did not run.
    expect(screen.queryByText(/selected/)).toBeNull();
  });

  it('switches "include subfolders" off and starts a new query', async () => {
    await renderLibrary();
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Include subfolders' }));
    await waitFor(() => expect(fake.callsOf('list_photos').at(-1)?.args['query']).toMatchObject({ recursive: false }));
  });

  it('shows a message if there are no folders', async () => {
    useFake(new FakeCatalog(0, []));
    await renderLibrary(false);
    expect(await screen.findByText('No folders yet.')).toBeTruthy();
  });
});
