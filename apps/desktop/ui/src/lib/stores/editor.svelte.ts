/**
 * State of the Editor module: the open document, the layer selection, the
 * canvas view, and the edit queue.
 *
 * - One document is open at a time. `openPhoto` closes the old document
 *   first. A late answer for an old request is dropped (generation
 *   counter), and a document that opens too late is closed again.
 * - Edits go to the shell one after the other, in order. A live edit
 *   (slider drag) replaces a waiting live edit of the same stream, so at
 *   most one edit of a stream is in flight and the newest value is kept.
 * - A drag uses one `gesture` number, so that the shell merges the drag
 *   into one undo step. A gesture ends on `endGesture()`, on any other
 *   edit, on undo and redo, and after `GESTURE_IDLE_MS` without a live edit.
 * - After each edit, undo, or redo, the document info is replaced. The
 *   canvas view watches `info.revision` and requests a new frame.
 */
import type { PhotoId } from '../catalog';
import type { DisplaySpace } from '../develop/transport';
import { editorApi, type DocumentInfo, type EditDto, type EditorApi, type LayerInfo } from '../editor/api';
import { allLayerIds, findLayer, moveStep, newLayerIds } from '../editor/layers';
import {
  clampView,
  clampZoom,
  fitView,
  hasArea,
  panBy,
  steppedZoom,
  zoomAbout,
  type Point,
  type Size,
  type View,
} from '../editor/view';
import type { LibraryStore } from './library.svelte';
import type { ModuleStore } from './modules.svelte';
import { describeError, type ToastStore } from './toasts.svelte';

/** A live edit stream keeps its gesture for this time after the last live edit. */
export const GESTURE_IDLE_MS = 600;

export interface EditorStoreOptions {
  readonly library: Pick<LibraryStore, 'focusIndex' | 'photoAt' | 'selectedIds'>;
  readonly modules: Pick<ModuleStore, 'active' | 'switchTo'>;
  readonly toasts: ToastStore;
  readonly api?: EditorApi;
  readonly displaySpace?: DisplaySpace | null;
  /** The clock in milliseconds. For tests. */
  readonly now?: () => number;
}

/** What the Editor view shows. */
export type EditorStatus = 'empty' | 'loading' | 'ready' | 'error';

/** Returns true if the value looks like a `CommandError` of the given kind. */
function errorKind(error: unknown): string | null {
  if (typeof error === 'object' && error !== null && 'kind' in error && typeof (error as { kind: unknown }).kind === 'string') {
    return (error as { kind: string }).kind;
  }
  return null;
}

export class EditorStore {
  /** The photo of the open document, or null. */
  photoId = $state<PhotoId | null>(null);
  /** The newest document info, or null. */
  info = $state.raw<DocumentInfo | null>(null);
  status = $state<EditorStatus>('empty');
  loadError = $state<string | null>(null);
  /** The selected layer id, or null. */
  selectedId = $state<number | null>(null);
  /** Ids of the expanded groups. */
  expanded = $state.raw<ReadonlySet<number>>(new Set());
  /** The canvas view, or null until the document and the viewport are known. */
  view = $state.raw<View | null>(null);
  /** True while the view follows "fit to window". */
  fit = $state(true);
  viewport = $state.raw<Size>({ width: 0, height: 0 });
  /** The number of edits that wait or run. */
  pending = $state(0);

  readonly #library: EditorStoreOptions['library'];
  readonly #modules: EditorStoreOptions['modules'];
  readonly #toasts: ToastStore;
  readonly #api: EditorApi;
  readonly #displaySpace: DisplaySpace | null;
  readonly #now: () => number;

  #generation = 0;
  #chain: Promise<void> = Promise.resolve();
  /** Waiting live edits by stream key. */
  readonly #live = new Map<string, { edit: EditDto; gesture: number }>();
  #gesture: { key: string; id: number; at: number } | null = null;
  #nextGesture = 1;

  constructor(options: EditorStoreOptions) {
    this.#library = options.library;
    this.#modules = options.modules;
    this.#toasts = options.toasts;
    this.#api = options.api ?? editorApi;
    this.#displaySpace = options.displaySpace ?? null;
    this.#now = options.now ?? (() => Date.now());
  }

  /** The display color space of the document (null: the shell default). */
  get displaySpace(): DisplaySpace | null {
    return this.#displaySpace;
  }

  /** True when no edit waits or runs. */
  get idle(): boolean {
    return this.pending === 0;
  }

  /** The photo that the library points at (focused, else first selected), or null. */
  get candidate(): PhotoId | null {
    const focus = this.#library.focusIndex;
    const focused = focus === null ? undefined : this.#library.photoAt(focus);
    return focused?.id ?? this.#library.selectedIds[0] ?? null;
  }

  get selected(): LayerInfo | null {
    const info = this.info;
    return info === null || this.selectedId === null ? null : findLayer(info.layers, this.selectedId);
  }

  get canUndo(): boolean {
    return this.info?.canUndo ?? false;
  }

  get canRedo(): boolean {
    return this.info?.canRedo ?? false;
  }

  // ---- Document ----

  /**
   * The photo to open now: the library `candidate`, if the Editor module is
   * active and no document is open. Else null. The view opens this photo.
   */
  get openTarget(): PhotoId | null {
    return this.#modules.active === 'editor' && this.status === 'empty' ? this.candidate : null;
  }

  /** Opens a photo in the Editor and shows the module. */
  async editInEditor(photoId: PhotoId | null): Promise<void> {
    if (photoId === null) {
      this.#toasts.info('Select a photo to edit it in the Editor.');
      return;
    }
    this.#modules.switchTo('editor');
    if (photoId === this.photoId && this.info !== null) {
      return;
    }
    await this.openPhoto(photoId);
  }

  /** Closes the old document and opens a document for the photo. */
  async openPhoto(photoId: PhotoId): Promise<void> {
    const generation = ++this.#generation;
    const old = this.info;
    this.#live.clear();
    this.#gesture = null;
    this.photoId = photoId;
    this.info = null;
    this.view = null;
    this.fit = true;
    this.selectedId = null;
    this.expanded = new Set();
    this.status = 'loading';
    this.loadError = null;
    if (old !== null) {
      await this.#closeDocument(old.documentId);
      if (generation !== this.#generation) {
        return;
      }
    }
    let info: DocumentInfo;
    try {
      info = await this.#api.open(photoId, this.#displaySpace);
    } catch (error: unknown) {
      if (generation !== this.#generation) {
        return;
      }
      this.status = 'error';
      this.loadError = this.#describe(error);
      this.#toasts.error(this.#messageFor('Open the photo in the Editor', error), error);
      return;
    }
    if (generation !== this.#generation) {
      // Another photo was requested while this one opened.
      await this.#closeDocument(info.documentId);
      return;
    }
    this.info = info;
    this.status = 'ready';
    this.selectedId = info.layers[info.layers.length - 1]?.id ?? null;
    this.#refit();
  }

  /** Closes the open document (the view is destroyed). */
  async dispose(): Promise<void> {
    this.#generation += 1;
    const old = this.info;
    this.#live.clear();
    this.info = null;
    this.view = null;
    this.status = 'empty';
    this.photoId = null;
    if (old !== null) {
      await this.#closeDocument(old.documentId);
    }
  }

  async #closeDocument(documentId: number): Promise<void> {
    try {
      await this.#api.close(documentId);
    } catch (error: unknown) {
      this.#toasts.error('Close the Editor document failed.', error);
    }
  }

  // ---- Selection ----

  select(id: number | null): void {
    this.selectedId = id;
  }

  toggleExpanded(id: number): void {
    const next = new Set(this.expanded);
    if (!next.delete(id)) {
      next.add(id);
    }
    this.expanded = next;
  }

  // ---- Edits ----

  /** Sends one edit, after the edits before it. */
  edit(edit: EditDto): void {
    this.endGesture();
    this.#enqueue(async (info, generation) => {
      const before = allLayerIds(info.layers);
      const next = await this.#api.edit(info.documentId, edit, null);
      if (generation !== this.#generation) {
        return;
      }
      this.#apply(next);
      if (edit.type === 'duplicate' || edit.type === 'addAdjustment') {
        const added = newLayerIds(before, next.layers);
        const top = added[added.length - 1];
        if (top !== undefined) {
          this.selectedId = top;
        }
      }
    });
  }

  /**
   * Sends a live edit of a stream (for example `opacity:12`). A waiting live
   * edit of the same stream is replaced, so the newest value is kept and at
   * most one edit of the stream is in flight.
   */
  liveEdit(streamKey: string, edit: EditDto): void {
    const gesture = this.#gestureFor(streamKey);
    const queued = this.#live.has(streamKey);
    this.#live.set(streamKey, { edit, gesture });
    if (queued) {
      return;
    }
    this.#enqueue(async (info, generation) => {
      const entry = this.#live.get(streamKey);
      this.#live.delete(streamKey);
      if (entry === undefined) {
        return;
      }
      const next = await this.#api.edit(info.documentId, entry.edit, entry.gesture);
      if (generation === this.#generation) {
        this.#apply(next);
      }
    });
  }

  /** Ends the current gesture (the slider is released). The next live edit starts a new undo step. */
  endGesture(): void {
    this.#gesture = null;
  }

  #gestureFor(key: string): number {
    const now = this.#now();
    const current = this.#gesture;
    if (current !== null && current.key === key && now - current.at < GESTURE_IDLE_MS) {
      current.at = now;
      return current.id;
    }
    const id = this.#nextGesture;
    this.#nextGesture += 1;
    this.#gesture = { key, id, at: now };
    return id;
  }

  undo(): void {
    this.#history('undo');
  }

  redo(): void {
    this.#history('redo');
  }

  #history(kind: 'undo' | 'redo'): void {
    this.endGesture();
    this.#enqueue(async (info, generation) => {
      const next = kind === 'undo' ? await this.#api.undo(info.documentId) : await this.#api.redo(info.documentId);
      if (generation === this.#generation) {
        this.#apply(next);
      }
    });
  }

  // ---- Layer commands (for the panel) ----

  setVisible(layerId: number, visible: boolean): void {
    this.edit({ type: 'setVisible', layerId, visible });
  }

  rename(layerId: number, name: string): void {
    const trimmed = name.trim();
    const layer = this.info === null ? null : findLayer(this.info.layers, layerId);
    if (trimmed === '' || layer === null || layer.name === trimmed) {
      return;
    }
    this.edit({ type: 'rename', layerId, name: trimmed });
  }

  move(layerId: number, direction: 'up' | 'down'): void {
    if (this.info === null) {
      return;
    }
    const edit = moveStep(this.info.layers, layerId, direction);
    if (edit !== null) {
      this.edit(edit);
    }
  }

  // ---- View ----

  get zoomPercent(): number | null {
    return this.view === null ? null : Math.round(this.view.zoom * 100);
  }

  setViewport(size: Size): void {
    if (size.width === this.viewport.width && size.height === this.viewport.height) {
      return;
    }
    this.viewport = size;
    if (this.view === null || this.fit) {
      this.#refit();
    } else {
      this.#setView(this.view);
    }
  }

  fitToWindow(): void {
    this.fit = true;
    this.#refit();
  }

  /** Zooms to a fixed value about the center of the viewport. */
  zoomTo(zoom: number): void {
    const view = this.view;
    if (view === null) {
      return;
    }
    this.fit = false;
    this.#setView(zoomAbout(view, clampZoom(zoom), { x: this.viewport.width / 2, y: this.viewport.height / 2 }));
  }

  zoomStep(direction: 1 | -1, anchor?: Point): void {
    const view = this.view;
    if (view === null) {
      return;
    }
    this.fit = false;
    const at = anchor ?? { x: this.viewport.width / 2, y: this.viewport.height / 2 };
    this.#setView(zoomAbout(view, steppedZoom(view.zoom, direction), at));
  }

  /** Zooms by a factor about a screen point (wheel and pinch). */
  zoomBy(factor: number, anchor: Point): void {
    const view = this.view;
    if (view === null || !Number.isFinite(factor)) {
      return;
    }
    this.fit = false;
    this.#setView(zoomAbout(view, view.zoom * factor, anchor));
  }

  /** Moves the content by a screen distance. */
  pan(dx: number, dy: number): void {
    const view = this.view;
    if (view === null) {
      return;
    }
    this.fit = false;
    this.#setView(panBy(view, dx, dy));
  }

  #refit(): void {
    const info = this.info;
    if (info === null || !hasArea(this.viewport)) {
      return;
    }
    this.view = fitView(this.viewport, info);
  }

  #setView(view: View): void {
    const info = this.info;
    if (info === null || !hasArea(this.viewport)) {
      return;
    }
    this.view = clampView(view, this.viewport, info);
  }

  // ---- Internal ----

  /** Replaces the info and repairs the selection. */
  #apply(next: DocumentInfo): void {
    const current = this.info;
    if (current === null || next.documentId !== current.documentId) {
      return;
    }
    this.info = next;
    const ids = allLayerIds(next.layers);
    if (this.selectedId !== null && !ids.has(this.selectedId)) {
      this.selectedId = null;
    }
    if ([...this.expanded].some((id) => !ids.has(id))) {
      this.expanded = new Set([...this.expanded].filter((id) => ids.has(id)));
    }
  }

  /** Runs a task after the tasks before it. Errors go to a toast. */
  #enqueue(task: (info: DocumentInfo, generation: number) => Promise<void>): void {
    const generation = this.#generation;
    this.pending += 1;
    this.#chain = this.#chain
      .then(async () => {
        const info = this.info;
        if (info === null || generation !== this.#generation) {
          return;
        }
        await task(info, generation);
      })
      .catch((error: unknown) => {
        if (generation === this.#generation) {
          this.#toasts.error(this.#messageFor('Edit the document', error), error);
        }
      })
      .finally(() => {
        this.pending -= 1;
      });
  }

  #describe(error: unknown): string {
    return describeError(error);
  }

  #messageFor(action: string, error: unknown): string {
    if (errorKind(error) === 'unavailable') {
      return `${action} failed: the Editor needs a GPU, and none is available.`;
    }
    return `${action} failed.`;
  }
}
