/**
 * Test helper: a fake of the catalog commands for DOM tests.
 *
 * Use it with a mock of `@tauri-apps/api/core`:
 *
 *   vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
 *   vi.mocked(invoke).mockImplementation(fake.invoke);
 *
 * The fake keeps the photos in memory. It does not sort or filter. It
 * records every call in `calls`.
 */
import type { InvokeArgs } from '@tauri-apps/api/core';
import type { CatalogStatus, FolderNode, ImportReport, Photo } from '../lib/catalog';

export interface RecordedCall {
  readonly command: string;
  readonly args: Record<string, unknown>;
}

/** Creates a photo with fixed test values. `over` replaces fields. */
export function makePhoto(id: number, over: Partial<Photo> = {}): Photo {
  return {
    id,
    folderId: 1,
    fileName: `photo-${id}.jpg`,
    fileSize: 1000,
    fileModifiedMs: 0,
    importedMs: id,
    captureTimeMs: Date.UTC(2024, 0, 1, 12, 0) + id * 60_000,
    width: 300,
    height: 200,
    rating: 0,
    flag: 'none',
    colorLabel: 'none',
    ...over,
  };
}

export function makeFolder(id: number, name: string, photoCount: number, children: FolderNode[] = []): FolderNode {
  return { folder: { id, parentId: null, path: `/photos/${name}`, name }, photoCount, children };
}

export class FakeCatalog {
  photos: Photo[];
  tree: FolderNode[];
  status: CatalogStatus = { state: 'ready', path: '/catalog.db', schemaVersion: 1, photoCount: 0 };
  pickedPath: string | null = '/photos/new';
  report: ImportReport = { root: '/photos/new', added: 0, skipped: 0, ignored: 0, addedIds: [], errors: [] };
  /** Commands that fail with a structured error. */
  readonly failing = new Set<string>();
  readonly calls: RecordedCall[] = [];

  constructor(photoCount = 0, tree: FolderNode[] = [makeFolder(1, 'photos', photoCount)]) {
    this.photos = Array.from({ length: photoCount }, (_, i) => makePhoto(i + 1));
    this.tree = tree;
  }

  /** Calls recorded for one command. */
  callsOf(command: string): RecordedCall[] {
    return this.calls.filter((call) => call.command === command);
  }

  /** Arrow function, so that it can be passed as a mock implementation. */
  readonly invoke = (command: string, rawArgs?: InvokeArgs): Promise<unknown> => {
    // The catalog commands always send an object. Binary arguments do not occur.
    const args = (rawArgs ?? {}) as Record<string, unknown>;
    this.calls.push({ command, args });
    if (this.failing.has(command)) {
      return Promise.reject({ kind: 'database', message: `${command} failed in the test` });
    }
    return Promise.resolve(this.handle(command, args));
  };

  private handle(command: string, args: Record<string, unknown>): unknown {
    switch (command) {
      case 'catalog_status':
        return this.status;
      case 'folder_tree':
        return this.tree;
      case 'list_photos': {
        const query = args['query'] as { limit?: number | null; offset?: number };
        const offset = query.offset ?? 0;
        const limit = query.limit ?? this.photos.length;
        return { photos: this.photos.slice(offset, offset + limit), total: this.photos.length };
      }
      case 'set_rating':
        return void this.patch(args['ids'] as number[], { rating: args['rating'] as number });
      case 'set_flag':
        return void this.patch(args['ids'] as number[], { flag: args['flag'] as Photo['flag'] });
      case 'set_color_label':
        return void this.patch(args['ids'] as number[], { colorLabel: args['label'] as Photo['colorLabel'] });
      case 'pick_folder':
        return this.pickedPath;
      case 'import_folder':
        return this.report;
      default:
        throw new Error(`The fake catalog does not know the command "${command}".`);
    }
  }

  private patch(ids: number[], change: Partial<Photo>): void {
    this.photos = this.photos.map((photo) => (ids.includes(photo.id) ? { ...photo, ...change } : photo));
  }
}

/**
 * happy-dom does no layout, so `clientWidth` and `clientHeight` are 0. This
 * sets fixed sizes for all elements. Call the returned function to undo it.
 */
export function mockElementSize(width: number, height: number): () => void {
  const proto = HTMLElement.prototype;
  const widthBefore = Object.getOwnPropertyDescriptor(proto, 'clientWidth');
  const heightBefore = Object.getOwnPropertyDescriptor(proto, 'clientHeight');
  Object.defineProperty(proto, 'clientWidth', { configurable: true, get: () => width });
  Object.defineProperty(proto, 'clientHeight', { configurable: true, get: () => height });
  return () => {
    for (const [name, before] of [
      ['clientWidth', widthBefore],
      ['clientHeight', heightBefore],
    ] as const) {
      if (before === undefined) {
        Reflect.deleteProperty(proto, name);
      } else {
        Object.defineProperty(proto, name, before);
      }
    }
  };
}
