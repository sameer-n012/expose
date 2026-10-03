/**
 * Sparse page cache for the library grid.
 *
 * The grid can show 50 000 photos, so it never loads all of them. It loads
 * pages of `pageSize` photos when the user scrolls near them. This class
 * keeps the loaded pages and tracks which pages are loading.
 *
 * It is plain logic with no reactivity. The store that owns it signals
 * changes to the UI.
 */
import type { Photo, PhotoId } from '../catalog';

export const DEFAULT_PAGE_SIZE = 200;

export class PhotoCache {
  readonly pageSize: number;
  /** Total number of photos of the query. Null until the first page arrives. */
  total: number | null = null;
  readonly #pages = new Map<number, Photo[]>();
  /**
   * Pages that are loading, or that failed. Neither is requested again before
   * `clear`. Thus a failed page does not cause a request at each scroll event.
   */
  readonly #claimed = new Set<number>();

  constructor(pageSize: number = DEFAULT_PAGE_SIZE) {
    if (!Number.isInteger(pageSize) || pageSize < 1) {
      throw new RangeError(`The page size must be a positive integer, not ${pageSize}.`);
    }
    this.pageSize = pageSize;
  }

  /** The photo at a position, or undefined if its page is not loaded. */
  get(index: number): Photo | undefined {
    if (!Number.isInteger(index) || index < 0) {
      return undefined;
    }
    return this.#pages.get(Math.floor(index / this.pageSize))?.[index % this.pageSize];
  }

  /**
   * Returns the pages that cover the positions `start` to `end` (both
   * included) and that are not loaded and not claimed. The pages become
   * claimed, so the next call does not return them again.
   * The range is limited to the known total.
   */
  claimMissing(start: number, end: number): number[] {
    const last = this.total === null ? end : Math.min(end, this.total - 1);
    const first = Math.max(start, 0);
    const missing: number[] = [];
    if (last < first) {
      return missing;
    }
    for (let page = Math.floor(first / this.pageSize); page <= Math.floor(last / this.pageSize); page++) {
      if (!this.#pages.has(page) && !this.#claimed.has(page)) {
        this.#claimed.add(page);
        missing.push(page);
      }
    }
    return missing;
  }

  /** Stores a loaded page and the total of the query. */
  setPage(page: number, photos: readonly Photo[], total: number): void {
    this.#pages.set(page, [...photos]);
    this.#claimed.add(page);
    this.total = total;
  }

  /** Removes all pages and the total. */
  clear(): void {
    this.#pages.clear();
    this.#claimed.clear();
    this.total = null;
  }

  /** The ids of all loaded photos, in grid order. */
  loadedIds(): PhotoId[] {
    const pages = [...this.#pages.keys()].sort((a, b) => a - b);
    return pages.flatMap((page) => (this.#pages.get(page) ?? []).map((photo) => photo.id));
  }

  /**
   * Changes loaded photos by id. `change` gets the old photo and returns
   * the new one. Photos that are not loaded are skipped.
   * @returns the old photos that changed, so that the caller can undo.
   */
  update(ids: Iterable<PhotoId>, change: (photo: Photo) => Photo): Photo[] {
    const wanted = new Set(ids);
    const previous: Photo[] = [];
    for (const photos of this.#pages.values()) {
      for (let i = 0; i < photos.length; i++) {
        const photo = photos[i];
        if (photo !== undefined && wanted.has(photo.id)) {
          previous.push(photo);
          photos[i] = change(photo);
        }
      }
    }
    return previous;
  }

  /** Puts photos back, for example to undo an `update`. Matches by id. */
  restore(photos: readonly Photo[]): void {
    const byId = new Map(photos.map((photo) => [photo.id, photo]));
    this.update(byId.keys(), (current) => byId.get(current.id) ?? current);
  }
}
