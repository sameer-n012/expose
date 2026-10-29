/**
 * The state of one open Editor document (one tab of the Editor module).
 *
 * - The `EditorStore` makes, activates, and closes tabs. A tab keeps its
 *   own document info, layer selection, view, edit queue, and save.
 * - An async result (edit, save, open) changes only the tab that started
 *   it. After a tab closes (`closed`), its late results are dropped.
 */
import type { PhotoId } from '../catalog';
import type { DocumentInfo, EditDto } from '../editor/api';
import { allLayerIds, findLayer } from '../editor/layers';
import type { MaskThumbnail } from '../editor/maskThumbnail';
import { FIT_MARGIN, clampView, fitView, hasArea, type Size, type View } from '../editor/view';

/** What a tab shows: the document is opening, is open, or did not open. */
export type EditorTabStatus = 'loading' | 'ready' | 'error';

/** A live edit stream keeps its gesture for this time after the last live edit. */
export const GESTURE_IDLE_MS = 600;

export class EditorTab {
  /** The id of the tab in the UI (not the shell document id). */
  readonly id: number;
  /** The photo that the tab opened, or null for a new document. */
  readonly photoId: PhotoId | null;
  /** The name to show until the document info arrives (new documents), or null. */
  readonly pendingName: string | null;
  /** The newest document info, or null while the document opens or after an open error. */
  info = $state.raw<DocumentInfo | null>(null);
  status = $state<EditorTabStatus>('loading');
  loadError = $state<string | null>(null);
  /** The selected layer id, or null. */
  selectedId = $state<number | null>(null);
  /** The layer that the user asked to show the mask of (the store reads it). */
  maskTarget = $state<number | null>(null);
  /** The layer whose mask the user chose as the paint target (the store reads it). */
  paintMaskId = $state<number | null>(null);
  /** Ids of the expanded groups. */
  expanded = $state.raw<ReadonlySet<number>>(new Set());
  /**
   * The canvas view, or null until the document and the viewport are known.
   * Its rotation is view state of this tab only: no undo step, not saved.
   */
  view = $state.raw<View | null>(null);
  /** True while the view follows "fit to window". */
  fit = $state(true);
  /** The number of edits that wait or run. */
  pending = $state(0);
  /** True while a save runs. */
  saving = $state(false);

  /** Mask thumbnails by layer id, each with the `maskVersion` it shows. Replaced on each change. */
  maskThumbs = $state.raw<ReadonlyMap<number, { version: number; thumb: MaskThumbnail }>>(new Map());
  /** Mask thumbnail fetches in flight: layer id to the `maskVersion` that is requested. */
  readonly maskThumbPending = new Map<number, number>();

  /** True after the tab closed. Late results of the tab are dropped. */
  closed = false;
  /** The edit queue: each edit starts after the edit before it. */
  chain: Promise<void> = Promise.resolve();
  /** Waiting live edits by stream key. */
  readonly live = new Map<string, { edit: EditDto; gesture: number }>();
  /** The gesture of the live edit stream that runs, or null. */
  gesture: { key: string; id: number; at: number } | null = null;
  /** The running save, or null. */
  saveRun: Promise<boolean> | null = null;
  #nextGesture = 1;

  constructor(id: number, photoId: PhotoId | null, pendingName: string | null = null) {
    this.id = id;
    this.photoId = photoId;
    this.pendingName = pendingName;
  }

  /** The title of the tab. */
  get title(): string {
    if (this.info !== null) {
      return this.info.name;
    }
    if (this.status === 'error') {
      return 'Error';
    }
    return this.pendingName ?? 'Loading…';
  }

  /** True if the document has changes after its last save (or its open). */
  get dirty(): boolean {
    return this.info?.dirty ?? false;
  }

  /** True if the Save command can run now for this tab. */
  get canSave(): boolean {
    return this.status === 'ready' && this.info !== null && !this.saving;
  }

  /** True if this tab is the document for the photo (the source photo or the saved photo). */
  showsPhoto(photoId: PhotoId): boolean {
    return this.photoId === photoId || this.info?.savedPhotoId === photoId;
  }

  /** Takes the info of an opened document and selects the top layer. */
  opened(info: DocumentInfo): void {
    this.info = info;
    this.status = 'ready';
    this.loadError = null;
    this.selectedId = info.layers[info.layers.length - 1]?.id ?? null;
  }

  /** Replaces the info and repairs the selection. Ignores info of another document. */
  apply(next: DocumentInfo): void {
    const current = this.info;
    if (this.closed || current === null || next.documentId !== current.documentId) {
      return;
    }
    this.info = next;
    const ids = allLayerIds(next.layers);
    if (this.selectedId !== null && !ids.has(this.selectedId)) {
      this.selectedId = null;
    }
    // Forget the mask target when its layer is gone or has lost its mask.
    if (this.maskTarget !== null && findLayer(next.layers, this.maskTarget)?.hasMask !== true) {
      this.maskTarget = null;
    }
    if ([...this.expanded].some((id) => !ids.has(id))) {
      this.expanded = new Set([...this.expanded].filter((id) => ids.has(id)));
    }
  }

  /** Ends the current gesture. The next live edit starts a new undo step. */
  endGesture(): void {
    this.gesture = null;
  }

  /** The gesture id of a live edit of the stream `key` at the time `now`. */
  gestureFor(key: string, now: number): number {
    const current = this.gesture;
    if (current !== null && current.key === key && now - current.at < GESTURE_IDLE_MS) {
      current.at = now;
      return current.id;
    }
    const id = this.#nextGesture;
    this.#nextGesture += 1;
    this.gesture = { key, id, at: now };
    return id;
  }

  /**
   * Fits the view to the viewport. Does nothing without a document or a
   * viewport area. The view keeps its rotation (ADR 0026).
   */
  refit(viewport: Size): void {
    const info = this.info;
    if (info !== null && hasArea(viewport)) {
      this.view = fitView(viewport, info, FIT_MARGIN, this.view?.rotation ?? 0);
    }
  }

  /** Sets the view, kept inside the limits of the viewport. */
  setView(view: View, viewport: Size): void {
    const info = this.info;
    if (info !== null && hasArea(viewport)) {
      this.view = clampView(view, viewport, info);
    }
  }

  /** Makes the view match a viewport after it changed or after the tab became active. */
  fitTo(viewport: Size): void {
    if (this.view === null || this.fit) {
      this.refit(viewport);
    } else {
      this.setView(this.view, viewport);
    }
  }

  /** Marks the tab closed and drops its waiting live edits. */
  markClosed(): void {
    this.closed = true;
    this.live.clear();
    this.gesture = null;
  }
}
