/**
 * State of the Library module: catalog status, folder tree, the current
 * query (folder, sort, filter), the paged photo list, and the selection.
 *
 * Backend calls go through `callBackend`, so a failure shows a toast.
 *
 * Query changes: every change of the folder, the "include subfolders" flag,
 * the sort, or the filter starts a new query. A generation counter makes
 * sure that a late answer of an old query is dropped.
 */
import { callBackend } from '../backend';
import {
  catalogStatus,
  folderTree,
  importFolder as importFolderCommand,
  listPhotos,
  pickFolder,
  setColorLabel,
  setFlag,
  setRating,
  type ColorLabel,
  type Flag,
  type FolderId,
  type FolderNode,
  type Photo,
  type PhotoId,
  type PhotoQuery,
  type SortDirection,
  type SortField,
} from '../catalog';
import { ancestorIds, collectFolders } from '../library/folderRows';
import type { LibraryCommands } from '../library/libraryActions';
import { DEFAULT_PAGE_SIZE, PhotoCache } from '../library/pageCache';
import {
  clearSelection,
  clickSelect,
  EMPTY_SELECTION,
  moveSelection,
  selectAllLoaded,
  type ClickModifiers,
  type SelectionState,
} from '../library/selection';
import type { ToastStore } from './toasts.svelte';

export interface LibrarySort {
  field: SortField;
  direction: SortDirection;
}

/** The grid filter. `minRating` 0 and the value 'any' mean: no filter. */
export interface LibraryFilter {
  minRating: number;
  flag: Flag | 'any';
  colorLabel: ColorLabel | 'any';
}

/** What the Library view shows. */
export type CatalogView =
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready' }
  | { readonly kind: 'unavailable'; readonly message: string };

export const DEFAULT_SORT: Readonly<LibrarySort> = { field: 'captureTime', direction: 'descending' };
export const DEFAULT_FILTER: Readonly<LibraryFilter> = { minRating: 0, flag: 'any', colorLabel: 'any' };

export class LibraryStore {
  /** Catalog status. The view shows the grid only when `ready`. */
  view = $state<CatalogView>({ kind: 'loading' });
  tree = $state.raw<readonly FolderNode[]>([]);
  /** The folders that are open in the tree. */
  expanded = $state.raw<ReadonlySet<FolderId>>(new Set());
  /** The selected folder. Null if the catalog has no folder. */
  folderId = $state<FolderId | null>(null);
  /** If true, the grid also shows the photos of the subfolders. */
  recursive = $state(true);
  sort = $state<LibrarySort>({ ...DEFAULT_SORT });
  filter = $state<LibraryFilter>({ ...DEFAULT_FILTER });
  /** Number of photos of the current query. */
  total = $state(0);
  /** True while the first page of a query loads. */
  loading = $state(false);
  /** True if a page failed to load. The view then offers a retry. */
  loadFailed = $state(false);
  /** True while the folder picker or an import runs. */
  importing = $state(false);
  selection = $state.raw<SelectionState>(EMPTY_SELECTION);
  /** Number of grid columns. The grid component sets it. Needed for the up and down keys. */
  columns = $state(1);

  /** Changes at each change of the cache. Reading it makes `photoAt` reactive. */
  #version = $state(0);
  #cache: PhotoCache;
  #generation = 0;
  #initStarted = false;
  /** Folders that the tree showed already. A new folder with children opens by default. */
  readonly #knownFolders = new Set<FolderId>();
  readonly #toasts: ToastStore;

  constructor(toasts: ToastStore, pageSize: number = DEFAULT_PAGE_SIZE) {
    this.#toasts = toasts;
    this.#cache = new PhotoCache(pageSize);
  }

  /** The number of photos per page. */
  get pageSize(): number {
    return this.#cache.pageSize;
  }

  /** The photo at a grid position, or undefined if it is not loaded (yet). */
  photoAt(index: number): Photo | undefined {
    void this.#version;
    return this.#cache.get(index);
  }

  /** The grid position of the keyboard focus, or null. */
  get focusIndex(): number | null {
    return this.selection.focus;
  }

  /** The selected photo ids. */
  get selectedIds(): PhotoId[] {
    return [...this.selection.ids];
  }

  isSelected(photo: Photo): boolean {
    return this.selection.ids.has(photo.id);
  }

  /** Reads the catalog status. If the catalog is ready, loads the tree and the photos. Runs once. */
  async init(): Promise<void> {
    if (this.#initStarted) {
      return;
    }
    this.#initStarted = true;
    const status = await callBackend('Read the catalog status', catalogStatus, this.#toasts);
    if (status === undefined) {
      this.view = { kind: 'unavailable', message: 'The catalog status could not be read.' };
      return;
    }
    if (status.state === 'unavailable') {
      this.view = { kind: 'unavailable', message: status.error };
      return;
    }
    this.view = { kind: 'ready' };
    await this.refresh();
  }

  /** Loads the folder tree again, and then the photos of the current folder. */
  async refresh(): Promise<void> {
    const tree = await callBackend('Load the folder tree', folderTree, this.#toasts);
    if (tree === undefined) {
      return;
    }
    this.tree = tree;
    const { all, withChildren } = collectFolders(tree);
    const expanded = new Set(this.expanded);
    for (const id of withChildren) {
      if (!this.#knownFolders.has(id)) {
        expanded.add(id);
      }
    }
    for (const id of all) {
      this.#knownFolders.add(id);
    }
    if (this.folderId === null || !all.includes(this.folderId)) {
      this.folderId = tree[0]?.folder.id ?? null;
    }
    if (this.folderId !== null) {
      for (const id of ancestorIds(tree, this.folderId)) {
        expanded.add(id);
      }
    }
    this.expanded = expanded;
    await this.#reload();
  }

  toggleExpanded(id: FolderId): void {
    const next = new Set(this.expanded);
    if (!next.delete(id)) {
      next.add(id);
    }
    this.expanded = next;
  }

  async selectFolder(id: FolderId): Promise<void> {
    if (id === this.folderId) {
      return;
    }
    this.folderId = id;
    await this.#reload();
  }

  async setRecursive(recursive: boolean): Promise<void> {
    if (recursive === this.recursive) {
      return;
    }
    this.recursive = recursive;
    await this.#reload();
  }

  async setSort(sort: Partial<LibrarySort>): Promise<void> {
    this.sort = { ...this.sort, ...sort };
    await this.#reload();
  }

  async setFilter(filter: Partial<LibraryFilter>): Promise<void> {
    this.filter = { ...this.filter, ...filter };
    await this.#reload();
  }

  /** Starts the current query again, for example after a failed page. */
  retry(): Promise<void> {
    return this.#reload();
  }

  /**
   * Loads the pages that cover the positions `start` to `end` (both
   * included), if they are not loaded. Call it when the grid scrolls.
   */
  async ensureRange(start: number, end: number): Promise<void> {
    const folderId = this.folderId;
    if (folderId === null) {
      return;
    }
    const generation = this.#generation;
    const pages = this.#cache.claimMissing(start, end);
    await Promise.all(
      pages.map(async (page) => {
        const query = this.#query(folderId, page);
        const result = await callBackend('Load photos', () => listPhotos(query), this.#toasts);
        if (generation !== this.#generation) {
          return; // The query changed. The answer is old.
        }
        if (result === undefined) {
          this.loadFailed = true;
          return;
        }
        this.#cache.setPage(page, result.photos, result.total);
        this.total = result.total;
        this.#version++;
      }),
    );
  }

  // --- Selection ---

  click(index: number, mods: ClickModifiers): void {
    this.selection = clickSelect(this.selection, index, (i) => this.#cache.get(i)?.id, mods);
  }

  moveSelection(dx: number, dy: number, extend: boolean): void {
    const delta = dx + dy * this.columns;
    const before = this.selection;
    this.selection = moveSelection(before, delta, this.total, (i) => this.#cache.get(i)?.id, extend);
    if (this.selection === before && before.focus !== null) {
      // The target is not loaded. Load it, so that the next key press works.
      const target = Math.min(Math.max(before.focus + delta, 0), this.total - 1);
      void this.ensureRange(target, target);
    }
  }

  selectAll(): void {
    this.selection = selectAllLoaded(this.selection, this.#cache.loadedIds());
  }

  clearSelection(): void {
    this.selection = clearSelection();
  }

  // --- Metadata changes on the selection ---

  /** Sets the rating (0 to 5) of the selected photos. */
  setRating(rating: number): Promise<void> {
    if (!Number.isInteger(rating) || rating < 0 || rating > 5) {
      throw new RangeError(`The rating must be an integer from 0 to 5, not ${rating}.`);
    }
    return this.#mutate(
      'Set the rating',
      (photo) => ({ ...photo, rating }),
      (ids) => setRating(ids, rating),
    );
  }

  setFlag(flag: Flag): Promise<void> {
    return this.#mutate(
      'Set the flag',
      (photo) => ({ ...photo, flag }),
      (ids) => setFlag(ids, flag),
    );
  }

  setColorLabel(label: ColorLabel): Promise<void> {
    return this.#mutate(
      'Set the color label',
      (photo) => ({ ...photo, colorLabel: label }),
      (ids) => setColorLabel(ids, label),
    );
  }

  /** The commands for the shortcut actions. Errors show as toasts; the promises never reject. */
  readonly commands: LibraryCommands = {
    setRating: (rating) => void this.setRating(rating),
    setFlag: (flag) => void this.setFlag(flag),
    setColorLabel: (label) => void this.setColorLabel(label),
    selectAll: () => this.selectAll(),
    moveSelection: (dx, dy, extend) => this.moveSelection(dx, dy, extend),
  };

  // --- Import ---

  /** Asks for a folder, imports it, and shows the result as a toast. */
  async importPhotos(): Promise<void> {
    if (this.importing) {
      return;
    }
    this.importing = true;
    try {
      const path = await callBackend('Open the folder picker', pickFolder, this.#toasts);
      if (path === undefined || path === null) {
        return; // Failure (toast shown) or the user cancelled.
      }
      const report = await callBackend('Import photos', () => importFolderCommand(path, true), this.#toasts);
      if (report === undefined) {
        return;
      }
      const message = `Imported ${report.added} photos. Skipped ${report.skipped}. Errors: ${report.errors.length}.`;
      const firstError = report.errors[0];
      if (firstError === undefined) {
        this.#toasts.success(message);
      } else {
        this.#toasts.warning(message, { detail: `${firstError.path}: ${firstError.message}` });
      }
      await this.refresh();
    } finally {
      this.importing = false;
    }
  }

  // --- Internals ---

  #query(folderId: FolderId, page: number): PhotoQuery {
    const { minRating, flag, colorLabel } = this.filter;
    return {
      folderId,
      recursive: this.recursive,
      sort: { field: this.sort.field, direction: this.sort.direction },
      filter: {
        minRating: minRating > 0 ? minRating : null,
        flag: flag === 'any' ? null : flag,
        colorLabel: colorLabel === 'any' ? null : colorLabel,
      },
      limit: this.#cache.pageSize,
      offset: page * this.#cache.pageSize,
    };
  }

  /** Drops the loaded pages and the selection, and loads the first page of the query. */
  async #reload(): Promise<void> {
    this.#generation++;
    const generation = this.#generation;
    this.#cache.clear();
    this.selection = EMPTY_SELECTION;
    this.total = 0;
    this.loadFailed = false;
    this.#version++;
    if (this.folderId === null) {
      this.loading = false;
      return;
    }
    this.loading = true;
    try {
      await this.ensureRange(0, this.#cache.pageSize - 1);
    } finally {
      if (generation === this.#generation) {
        this.loading = false;
      }
    }
  }

  /**
   * Changes the selected photos in the list at once, and then in the
   * backend. If the backend fails, the old values come back. `callBackend`
   * shows the error toast.
   */
  async #mutate(
    description: string,
    change: (photo: Photo) => Photo,
    call: (ids: PhotoId[]) => Promise<void>,
  ): Promise<void> {
    const ids = this.selectedIds;
    if (ids.length === 0) {
      return;
    }
    const previous = this.#cache.update(ids, change);
    this.#version++;
    const done = await callBackend(
      description,
      async () => {
        await call(ids);
        return true;
      },
      this.#toasts,
    );
    if (done === undefined) {
      this.#cache.restore(previous);
      this.#version++;
    }
  }
}
