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
 * - `save()` waits for the queued edits, and then saves the document to the
 *   library (a layered `.exdoc` file and a flattened TIFF). Edits during a
 *   save go on; they keep the document dirty.
 * - Before the open document is replaced, a dirty document asks the user
 *   (`leavePrompt`: Save, Don't save, or Cancel).
 */
import { pickFolder, type PhotoId } from '../catalog';
import type { DisplaySpace } from '../develop/transport';
import {
  editorApi,
  type DocumentInfo,
  type EditDto,
  type EditorApi,
  type LayerInfo,
  type LocksDto,
  type MaskFillId,
  type NewDocumentSpec,
  type SaveResult,
} from '../editor/api';
import { NO_LOCKS, allLayerIds, effectiveLocks, findLayer, maskCommands, moveStep, newLayerIds } from '../editor/layers';
import { untitledName } from '../editor/newDocument';
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
  /** Called after each successful save (the shell refreshes the library). */
  readonly onSaved?: (result: SaveResult) => void;
  /** Shows the folder dialog for "Save As". For tests. */
  readonly pickFolder?: () => Promise<string | null>;
}

/** What the Editor view shows. */
export type EditorStatus = 'empty' | 'loading' | 'ready' | 'error';

/** The answer to the unsaved-changes question. */
export type LeaveChoice = 'save' | 'discard' | 'cancel';

/** An open unsaved-changes question. */
export interface LeavePrompt {
  /** The question for the user. */
  readonly message: string;
}

/** Returns the file name of a path (after the last `/` or `\`). */
export function fileNameOf(path: string): string {
  const index = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
  return path.slice(index + 1);
}

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
  /** True while a save runs. */
  saving = $state(false);
  /** The open unsaved-changes question, or null. */
  leavePrompt = $state.raw<LeavePrompt | null>(null);
  /** True while the New document dialog is open. */
  newDocumentOpen = $state(false);

  readonly #library: EditorStoreOptions['library'];
  readonly #modules: EditorStoreOptions['modules'];
  readonly #toasts: ToastStore;
  readonly #api: EditorApi;
  readonly #displaySpace: DisplaySpace | null;
  readonly #now: () => number;
  readonly #onSaved: ((result: SaveResult) => void) | undefined;
  readonly #pickFolder: () => Promise<string | null>;
  #answerLeave: ((choice: LeaveChoice) => void) | null = null;
  /** The running save, or null. */
  #saveRun: Promise<boolean> | null = null;
  /** The number in the next suggested "Untitled-N" name. */
  #untitled = 1;

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
    this.#onSaved = options.onSaved;
    this.#pickFolder = options.pickFolder ?? pickFolder;
  }

  /** True if the open document has changes after its last save (or its open). */
  get dirty(): boolean {
    return this.info?.dirty ?? false;
  }

  /** True if the Save command can run now. */
  get canSave(): boolean {
    return this.status === 'ready' && this.info !== null && !this.saving;
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

  /** The locks that apply to the selected layer (own and from ancestor groups). */
  get selectedLocks(): LocksDto {
    const info = this.info;
    return info === null || this.selectedId === null ? NO_LOCKS : effectiveLocks(info.layers, this.selectedId);
  }

  /** Sets the own locks of a layer. */
  setLocks(layerId: number, locks: LocksDto): void {
    this.edit({ type: 'setLocks', layerId, locks });
  }

  /** Adds a transparent pixel layer above the selected layer. */
  addPixelLayer(): void {
    if (this.info !== null) {
      this.edit({ type: 'addPixelLayer', aboveLayerId: this.selectedId });
    }
  }

  /** Adds a canvas-size mask to a layer (default: the selected layer). */
  addMask(fill: MaskFillId, layerId: number | null = this.selectedId): void {
    if (layerId !== null && this.info !== null && maskCommands(this.info.layers, layerId).canAdd) {
      this.edit({ type: 'addMask', layerId, fill });
    }
  }

  /** Removes the mask of a layer (default: the selected layer). */
  deleteMask(layerId: number | null = this.selectedId): void {
    if (layerId !== null && this.info !== null && maskCommands(this.info.layers, layerId).canDelete) {
      this.edit({ type: 'deleteMask', layerId });
    }
  }

  /** Turns the mask of a layer on or off. */
  setMaskEnabled(layerId: number, enabled: boolean): void {
    if (this.info !== null && maskCommands(this.info.layers, layerId).canToggle) {
      this.edit({ type: 'setMaskEnabled', layerId, enabled });
    }
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
    const info = this.info;
    // The saved photo of the open document opens the same layered file:
    // keep the open document.
    if (info !== null && (photoId === this.photoId || photoId === info.savedPhotoId)) {
      return;
    }
    await this.openPhoto(photoId);
  }

  /**
   * Closes the old document and opens a document for the photo. If the old
   * document has unsaved changes, the user is asked first; on Cancel the old
   * document stays open.
   */
  async openPhoto(photoId: PhotoId): Promise<void> {
    await this.#replaceDocument(photoId, () => this.#api.open(photoId, this.#displaySpace), 'Open the photo in the Editor');
  }

  /** The name that the New document dialog suggests ("Untitled-N", N counts per session). */
  get suggestedName(): string {
    return untitledName(this.#untitled);
  }

  /** Opens the New document dialog and shows the Editor module (the dialog lives in the Editor view). */
  requestNewDocument(): void {
    this.#modules.switchTo('editor');
    this.newDocumentOpen = true;
  }

  closeNewDocument(): void {
    this.newDocumentOpen = false;
  }

  /**
   * Makes a new empty document and shows it. Like `openPhoto`, it asks about
   * unsaved changes of the old document first. Returns true if the document
   * opened.
   */
  async newDocument(spec: NewDocumentSpec): Promise<boolean> {
    const ok = await this.#replaceDocument(null, () => this.#api.newDocument(spec, this.#displaySpace), 'Make the new document');
    if (ok) {
      this.#untitled += 1;
    }
    return ok;
  }

  /**
   * The shared flow of `openPhoto` and `newDocument`: confirm, close the old
   * document, and open the new one. Returns true if the new document is open.
   */
  async #replaceDocument(photoId: PhotoId | null, open: () => Promise<DocumentInfo>, action: string): Promise<boolean> {
    if (this.info !== null && !(await this.confirmLeave())) {
      return false;
    }
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
        return false;
      }
    }
    let info: DocumentInfo;
    try {
      info = await open();
    } catch (error: unknown) {
      if (generation !== this.#generation) {
        return false;
      }
      this.status = 'error';
      this.loadError = this.#describe(error);
      this.#toasts.error(this.#messageFor(action, error), error);
      return false;
    }
    if (generation !== this.#generation) {
      // Another document was requested while this one opened.
      await this.#closeDocument(info.documentId);
      return false;
    }
    this.info = info;
    this.status = 'ready';
    this.selectedId = info.layers[info.layers.length - 1]?.id ?? null;
    this.#refit();
    return true;
  }

  /** Closes the open document (the view is destroyed). */
  async dispose(): Promise<void> {
    // An open question has no view any more.
    this.answerLeave('cancel');
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

  // ---- Save ----

  /**
   * Saves the open document to the library. Waits for the queued edits
   * first. A second call while a save runs returns the same result.
   * Returns true on success. Errors show a toast.
   */
  save(): Promise<boolean> {
    // A new document has no source photo and no saved files: it needs a folder.
    const info = this.info;
    if (info !== null && info.savedPhotoId === null && info.sourcePhotoId === null) {
      return this.saveAs();
    }
    return this.#startSave();
  }

  /**
   * "Save As": asks for a folder, then saves the document as new files in
   * that folder (a new library photo). The earlier files stay as they are.
   * Returns false if the user cancels the dialog or the save fails.
   */
  async saveAs(): Promise<boolean> {
    if (!this.canSave) {
      return this.#saveRun ?? false;
    }
    let folder: string | null;
    try {
      folder = await this.#pickFolder();
    } catch (error: unknown) {
      this.#toasts.error('Open the folder picker failed.', error);
      return false;
    }
    if (folder === null) {
      return false;
    }
    return this.#startSave(folder);
  }

  #startSave(folder?: string): Promise<boolean> {
    if (this.#saveRun !== null) {
      return this.#saveRun;
    }
    if (!this.canSave) {
      return Promise.resolve(false);
    }
    const run = this.#runSave(folder).finally(() => {
      this.#saveRun = null;
    });
    this.#saveRun = run;
    return run;
  }

  async #runSave(folder?: string): Promise<boolean> {
    this.endGesture();
    this.saving = true;
    const generation = this.#generation;
    try {
      // The save must contain the edits that the user made before it.
      await this.#chain;
      const info = this.info;
      if (info === null || generation !== this.#generation) {
        return false;
      }
      const result = await this.#api.save(info.documentId, folder);
      if (generation === this.#generation) {
        this.#applySaved(result);
      }
      this.#toasts.success(`Saved to library: ${fileNameOf(result.imagePath)}`);
      this.#onSaved?.(result);
      return true;
    } catch (error: unknown) {
      if (generation === this.#generation) {
        let message = this.#messageFor('Save the document', error);
        // A plain save writes next to the source photo; that folder can be read-only.
        if (folder === undefined && errorKind(error) === 'io') {
          message += ' Try "Save As…" to choose a different folder.';
        }
        this.#toasts.error(message, error);
      }
      return false;
    } finally {
      this.saving = false;
    }
  }

  /** Takes the save result. Edits after the save snapshot keep their newer info. */
  #applySaved(result: SaveResult): void {
    const current = this.info;
    if (current === null || current.documentId !== result.info.documentId) {
      return;
    }
    if (current.revision === result.info.revision) {
      this.#apply(result.info);
    } else {
      this.info = { ...current, savedPhotoId: result.info.savedPhotoId };
    }
  }

  /**
   * Asks the user what to do with unsaved changes before the document is
   * replaced. Returns true if the caller may go on (saved, or the user does
   * not want to save), false on Cancel or a failed save.
   */
  async confirmLeave(): Promise<boolean> {
    // Queued edits and a running save change `dirty`.
    await this.#chain;
    if (this.#saveRun !== null) {
      await this.#saveRun;
    }
    if (!this.dirty) {
      return true;
    }
    if (this.#answerLeave !== null) {
      // A question is open already. This request does not go on.
      return false;
    }
    const choice = await new Promise<LeaveChoice>((resolve) => {
      this.#answerLeave = resolve;
      this.leavePrompt = { message: 'The document has changes that are not saved.' };
    });
    if (choice === 'cancel') {
      return false;
    }
    if (choice === 'discard') {
      return true;
    }
    return this.save();
  }

  /** Answers the open unsaved-changes question (the dialog calls this). */
  answerLeave(choice: LeaveChoice): void {
    const answer = this.#answerLeave;
    this.#answerLeave = null;
    this.leavePrompt = null;
    answer?.(choice);
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
      if (edit.type === 'duplicate' || edit.type === 'addAdjustment' || edit.type === 'addPixelLayer') {
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
