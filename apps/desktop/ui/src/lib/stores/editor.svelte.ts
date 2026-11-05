/**
 * State of the Editor module: the open documents (tabs), the tools, and the
 * canvas viewport.
 *
 * - Each open document is one `EditorTab`. One tab is active; the getters
 *   and commands of this store (`info`, `view`, `edit`, `save`, …) work on
 *   the active tab. An async result (open, edit, save) changes only the tab
 *   that started it, also when another tab is active now.
 * - `openPhoto` and `newDocument` open a new tab and make it active. A photo
 *   that has a tab already (its source photo or its saved photo) activates
 *   that tab. A failed open leaves an error tab, which the user can close.
 * - `closeTab` asks about unsaved changes of that tab first (Save, Don't
 *   save, or Cancel). A tab that closes while its document opens closes the
 *   document when the open finishes.
 * - Edits of a tab go to the shell one after the other, in order. A live
 *   edit (slider drag) replaces a waiting live edit of the same stream, so
 *   at most one edit of a stream is in flight and the newest value is kept.
 * - A drag uses one `gesture` number, so that the shell merges the drag
 *   into one undo step. A gesture ends on `endGesture()`, on any other
 *   edit, on undo and redo, on a tab switch, and after `GESTURE_IDLE_MS`
 *   without a live edit.
 * - After each edit, undo, or redo, the document info is replaced. The
 *   canvas view watches `info.revision` and requests a new frame.
 * - `save()` waits for the queued edits of the tab, and then saves the
 *   document to the library (a layered `.exdoc` file and a flattened TIFF).
 *   Edits during a save go on; they keep the document dirty.
 * - Auto-open: while the Editor module shows no tab, the view opens the
 *   library photo (`openTarget`). After the user closes the last tab,
 *   auto-open stays off until the user leaves the Editor module.
 */
import { pickFolder, type PhotoId } from '../catalog';
import type { DisplaySpace } from '../develop/transport';
import {
  editorApi,
  type DocumentInfo,
  type EditDto,
  type EditorApi,
  type LayerBounds,
  type LayerInfo,
  type LocksDto,
  type MaskFillId,
  type NewDocumentSpec,
  type SampleSource,
  type SaveResult,
  type SelectionModeId,
  type SelectionOutline,
  type SelectionShapeDto,
  type StrokeTargetDto,
} from '../editor/api';
import {
  DEFAULT_BACKGROUND,
  DEFAULT_FOREGROUND,
  defaultBrush,
  defaultEraser,
  hexToRgb,
  normalizeBrush,
  steppedHardness,
  steppedSize,
  strokeBrush,
  strokeTarget,
  type BrushSettings,
  type PaintTarget,
  type PaintTool,
} from '../editor/brush';
import { defaultBucket, defaultWand, fillEdit, magicWandEdit, type FloodSettings, type PaintBucketSettings } from '../editor/flood';
import { clampFeather, cycleFill, cycleLasso, cycleMarquee, isFillTool, isLassoTool, type EditorTool, type FillTool, type LassoTool } from '../editor/selection';
import { defaultGradient, gradientEdit, presetStops, type GradientSettings } from '../editor/gradient';
import { NO_LOCKS, allLayerIds, canMaskFromSelection, effectiveLocks, findLayer, maskCommands, moveStep, newLayerIds } from '../editor/layers';
import { untitledName } from '../editor/newDocument';
import { decodeMaskThumbnail, type MaskThumbnail } from '../editor/maskThumbnail';
import { clampZoom, normalizeRotation, panBy, rotateVector, steppedZoom, zoomAbout, type Point, type Size, type View } from '../editor/view';
import { identityState, isIdentityState, stateMatrix, type Interpolation, type TransformState } from '../editor/transform';
import { EditorTab, type TransformSession } from './editorTab.svelte';
import type { LibraryStore } from './library.svelte';
import type { ModuleStore } from './modules.svelte';
import { describeError, type ToastStore } from './toasts.svelte';

export { EditorTab, GESTURE_IDLE_MS, type EditorTabStatus } from './editorTab.svelte';

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

/** What the Editor view shows ('empty': no tab is active). */
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
  /** The open tabs, in strip order. */
  #tabs = $state.raw<readonly EditorTab[]>([]);
  #activeTab = $state.raw<EditorTab | null>(null);
  viewport = $state.raw<Size>({ width: 0, height: 0 });
  /** The open unsaved-changes question, or null. */
  leavePrompt = $state.raw<LeavePrompt | null>(null);
  /** True while the New document dialog is open. */
  newDocumentOpen = $state(false);
  /** The tool of the canvas. */
  tool = $state<EditorTool>('hand');
  /** The feather of new selections, 0..250 px. */
  feather = $state(0);
  antiAlias = $state(true);
  /** True while the Feather dialog is open. */
  featherDialogOpen = $state(false);
  /** The settings of the brush tool. */
  brush = $state<BrushSettings>(defaultBrush());
  /** The settings of the eraser tool. */
  eraser = $state<BrushSettings>(defaultEraser());
  /** The foreground color, "#rrggbb". */
  foreground = $state(DEFAULT_FOREGROUND);
  /** The background color, "#rrggbb". */
  background = $state(DEFAULT_BACKGROUND);
  /** The sample size of the eyedropper, in pixels (a square). */
  sampleSize = $state<1 | 3 | 5>(1);
  /** What the eyedropper reads: all layers (the composite) or the selected layer. */
  sampleSource = $state<SampleSource>('composite');
  /** The settings of the magic wand. Shared by all tabs. */
  wand = $state<FloodSettings>(defaultWand());
  /** The settings of the paint bucket. Shared by all tabs. */
  bucket = $state<PaintBucketSettings>(defaultBucket());
  /** The settings of the gradient tool. Shared by all tabs. */
  gradient = $state<GradientSettings>(defaultGradient());
  /** True while the gradient editor dialog is open. */
  gradientDialogOpen = $state(false);

  readonly #library: EditorStoreOptions['library'];
  readonly #modules: EditorStoreOptions['modules'];
  readonly #toasts: ToastStore;
  readonly #api: EditorApi;
  readonly #displaySpace: DisplaySpace | null;
  readonly #now: () => number;
  readonly #onSaved: ((result: SaveResult) => void) | undefined;
  readonly #pickFolder: () => Promise<string | null>;
  #answerLeave: ((choice: LeaveChoice) => void) | null = null;
  /** The number in the next suggested "Untitled-N" name. */
  #untitled = 1;
  #nextTabId = 1;
  /**
   * True after the user closed the last tab: auto-open is off. This is not
   * reactive state on purpose, because `openTarget` clears it (see there).
   */
  #autoOpenOff = false;

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

  // ---- Tabs ----

  /** The open tabs, in strip order. */
  get tabs(): readonly EditorTab[] {
    return this.#tabs;
  }

  /** The active tab, or null when no tab is open. */
  get activeTab(): EditorTab | null {
    return this.#activeTab;
  }

  /** True if a tab has unsaved changes. */
  get anyDirty(): boolean {
    return this.#tabs.some((tab) => tab.dirty);
  }

  /**
   * Makes a tab active. The gesture of the old tab ends. The view of the new
   * tab follows the current viewport (a tab in fit mode fits again).
   */
  activate(tab: EditorTab): void {
    const old = this.#activeTab;
    if (tab === old || !this.#tabs.includes(tab)) {
      return;
    }
    old?.endGesture();
    // A tab switch ends a free transform without an edit (ADR 0029).
    if (old !== null && !old.transformBusy) {
      old.transform = null;
    }
    this.#activeTab = tab;
    tab.fitTo(this.viewport);
  }

  /** Activates the next (1) or the previous (-1) tab. It wraps at the ends. */
  nextTab(step: 1 | -1): void {
    const tabs = this.#tabs;
    const active = this.#activeTab;
    if (tabs.length < 2 || active === null) {
      return;
    }
    const index = tabs.indexOf(active);
    const next = tabs[(index + step + tabs.length) % tabs.length];
    if (next !== undefined) {
      this.activate(next);
    }
  }

  /** Closes the active tab (`Mod+W`). */
  closeActiveTab(): void {
    const tab = this.#activeTab;
    if (tab !== null) {
      void this.closeTab(tab);
    }
  }

  /**
   * Closes a tab. A dirty tab becomes active and asks first. Then the shell
   * closes the document, and the tab to the right (else to the left)
   * becomes active. Returns false if the tab stays open (Cancel, or a failed
   * save).
   */
  async closeTab(tab: EditorTab): Promise<boolean> {
    if (!this.#tabs.includes(tab)) {
      return true;
    }
    if (tab.info !== null) {
      // Queued edits and a running save change `dirty`.
      await this.#settle(tab);
      if (tab.dirty) {
        this.activate(tab);
        if (!(await this.#confirmTab(tab))) {
          return false;
        }
      }
      if (!this.#tabs.includes(tab)) {
        // The tab closed while the question was open.
        return true;
      }
    }
    this.#removeTab(tab);
    if (this.#tabs.length === 0) {
      this.#autoOpenOff = true;
    }
    if (tab.info !== null) {
      await this.#closeDocument(tab.info.documentId);
    }
    return true;
  }

  /**
   * Asks about the unsaved changes of all tabs before the window closes. Each
   * dirty tab becomes active in turn and asks. Returns false on Cancel or a
   * failed save (the window stays open).
   */
  async confirmCloseAll(): Promise<boolean> {
    for (const tab of [...this.#tabs]) {
      if (!this.#tabs.includes(tab)) {
        continue;
      }
      await this.#settle(tab);
      if (!tab.dirty) {
        continue;
      }
      this.activate(tab);
      if (!(await this.#confirmTab(tab))) {
        return false;
      }
    }
    return true;
  }

  /** Removes a tab from the strip and activates its neighbor (right, else left). */
  #removeTab(tab: EditorTab): void {
    const tabs = this.#tabs;
    const index = tabs.indexOf(tab);
    if (index < 0) {
      return;
    }
    tab.markClosed();
    const rest = tabs.filter((t) => t !== tab);
    this.#tabs = rest;
    if (this.#activeTab === tab) {
      this.#activeTab = null;
      const next = rest[index] ?? rest[index - 1];
      if (next !== undefined) {
        this.activate(next);
      }
    }
  }

  /** Waits for the queued edits and the running save of a tab. */
  async #settle(tab: EditorTab): Promise<void> {
    await tab.chain;
    if (tab.saveRun !== null) {
      await tab.saveRun;
    }
  }

  /** The open tab (not an error tab) that shows the photo, or null. */
  #tabFor(photoId: PhotoId): EditorTab | null {
    return this.#tabs.find((tab) => tab.status !== 'error' && tab.showsPhoto(photoId)) ?? null;
  }

  // ---- Active tab ----

  /** The photo of the active tab, or null. */
  get photoId(): PhotoId | null {
    return this.#activeTab?.photoId ?? null;
  }

  /** The newest document info of the active tab, or null. */
  get info(): DocumentInfo | null {
    return this.#activeTab?.info ?? null;
  }

  get status(): EditorStatus {
    return this.#activeTab?.status ?? 'empty';
  }

  get loadError(): string | null {
    return this.#activeTab?.loadError ?? null;
  }

  /** The selected layer id of the active tab, or null. */
  get selectedId(): number | null {
    return this.#activeTab?.selectedId ?? null;
  }

  /** Ids of the expanded groups of the active tab. */
  get expanded(): ReadonlySet<number> {
    return this.#activeTab?.expanded ?? new Set();
  }

  /** The canvas view of the active tab, or null. */
  get view(): View | null {
    return this.#activeTab?.view ?? null;
  }

  /** Sets the view of the active tab as it is (for tests). */
  set view(view: View | null) {
    const tab = this.#activeTab;
    if (tab !== null) {
      tab.view = view;
    }
  }

  /** True while the view of the active tab follows "fit to window". */
  get fit(): boolean {
    return this.#activeTab?.fit ?? true;
  }

  /** The number of edits of the active tab that wait or run. */
  get pending(): number {
    return this.#activeTab?.pending ?? 0;
  }

  /** True while a save of the active tab runs. */
  get saving(): boolean {
    return this.#activeTab?.saving ?? false;
  }

  /** True if the active document has changes after its last save (or its open). */
  get dirty(): boolean {
    return this.#activeTab?.dirty ?? false;
  }

  /** True if the Save command can run now. */
  get canSave(): boolean {
    return this.#activeTab?.canSave ?? false;
  }

  /** The display color space of the document (null: the shell default). */
  get displaySpace(): DisplaySpace | null {
    return this.#displaySpace;
  }

  /** True when no edit of the active tab waits or runs. */
  get idle(): boolean {
    return this.pending === 0;
  }

  /** The photo that the library points at (focused, else first selected), or null. */
  get candidate(): PhotoId | null {
    const focus = this.#library.focusIndex;
    const focused = focus === null ? undefined : this.#library.photoAt(focus);
    return focused?.id ?? this.#library.selectedIds[0] ?? null;
  }

  /**
   * The layer whose mask the canvas shows as a gray image, or null. It is
   * the selected layer, and only while that layer has a mask. A change of the
   * selection, a lost mask, or a new document turns the mask view off.
   */
  get showMaskLayerId(): number | null {
    const target = this.#activeTab?.maskTarget ?? null;
    if (target === null || target !== this.selectedId) {
      return null;
    }
    return this.selected?.hasMask === true ? target : null;
  }

  /** True while the canvas shows a mask. */
  get showMask(): boolean {
    return this.showMaskLayerId !== null;
  }

  /**
   * Turns the mask view on or off for `layerId` (default: the selected
   * layer). Does nothing if the layer has no mask. A layer that is not
   * selected becomes selected first.
   */
  toggleShowMask(layerId: number | null = this.selectedId): void {
    const tab = this.#activeTab;
    const info = tab?.info ?? null;
    if (tab === null || info === null || layerId === null) {
      return;
    }
    if (findLayer(info.layers, layerId)?.hasMask !== true) {
      return;
    }
    if (layerId === this.showMaskLayerId) {
      tab.maskTarget = null;
      return;
    }
    tab.selectedId = layerId;
    tab.maskTarget = layerId;
  }

  get selected(): LayerInfo | null {
    const info = this.info;
    const id = this.selectedId;
    return info === null || id === null ? null : findLayer(info.layers, id);
  }

  /** The locks that apply to the selected layer (own and from ancestor groups). */
  get selectedLocks(): LocksDto {
    const info = this.info;
    const id = this.selectedId;
    return info === null || id === null ? NO_LOCKS : effectiveLocks(info.layers, id);
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

  // ---- Selection ----

  /** True if the active document has a selection. */
  get hasSelection(): boolean {
    return this.info?.hasSelection ?? false;
  }

  /** Sets the tool of the canvas. */
  setTool(tool: EditorTool): void {
    // A tool change ends a free transform without an edit (ADR 0029).
    this.cancelTransform();
    this.tool = tool;
    if (isLassoTool(tool)) {
      this.lastLasso = tool;
    }
    if (isFillTool(tool)) {
      this.lastFill = tool;
    }
  }

  /** The fill tool that `G` selects when no fill tool is active. */
  private lastFill: FillTool = 'paintBucket';

  /** `G`: the last used fill tool; while one is active, the other one. */
  cycleFill(): void {
    if (this.info !== null) {
      this.setTool(cycleFill(this.tool, this.lastFill));
    }
  }

  /** The lasso tool that `L` selects when no lasso tool is active. */
  private lastLasso: LassoTool = 'lasso';

  /** `L`: the last used lasso tool; while one is active, the other one. */
  cycleLasso(): void {
    if (this.info !== null) {
      this.setTool(cycleLasso(this.tool, this.lastLasso));
    }
  }

  /** `M`: the rectangle tool, then the ellipse tool. */
  cycleMarquee(): void {
    if (this.info !== null) {
      this.tool = cycleMarquee(this.tool);
    }
  }

  // ---- Brush and eraser ----

  /**
   * What a stroke changes: the pixels or the mask of the selected layer. The
   * mask is the target only while the user chose it for the selected layer
   * and that layer still has a mask. Else the target is the pixels.
   */
  get paintTarget(): PaintTarget {
    const id = this.#activeTab?.paintMaskId ?? null;
    return id !== null && id === this.selectedId && this.selected?.hasMask === true ? 'mask' : 'pixels';
  }

  /** Chooses the mask of a layer as the paint target. Does nothing if the layer has no mask. */
  selectMaskTarget(layerId: number): void {
    const tab = this.#activeTab;
    const info = tab?.info ?? null;
    if (tab === null || info === null || findLayer(info.layers, layerId)?.hasMask !== true) {
      return;
    }
    // Not `select()`: it would set the target to the pixels.
    if (layerId !== tab.maskTarget) {
      tab.maskTarget = null;
    }
    tab.selectedId = layerId;
    tab.paintMaskId = layerId;
  }

  /**
   * The reason why the brush and the eraser cannot paint now, or null if they
   * can. The tools paint on a pixel layer, or on the mask of a layer.
   */
  get paintBlockReason(): string | null {
    if (this.info === null) {
      return 'No document is open.';
    }
    const layer = this.selected;
    if (layer === null) {
      return 'Select a layer to paint on.';
    }
    if (this.paintTarget === 'pixels' && layer.kind !== 'pixel') {
      return 'The brush and the eraser paint on pixel layers. Select a pixel layer, or click the mask of a layer.';
    }
    return null;
  }

  /** The settings of a paint tool. */
  brushOf(tool: PaintTool): BrushSettings {
    return tool === 'brush' ? this.brush : this.eraser;
  }

  /** The paint tool that is active, or null. */
  get paintTool(): PaintTool | null {
    return this.tool === 'brush' || this.tool === 'eraser' ? this.tool : null;
  }

  /** Changes some settings of a paint tool. Values go to their ranges. */
  updateBrush(tool: PaintTool, patch: Partial<BrushSettings>): void {
    const next = normalizeBrush({ ...this.brushOf(tool), ...patch });
    if (tool === 'brush') {
      this.brush = next;
    } else {
      this.eraser = next;
    }
  }

  /** Selects the brush or the eraser. Does nothing while the tool cannot paint. */
  selectPaintTool(tool: PaintTool): void {
    if (this.paintBlockReason === null) {
      this.tool = tool;
    }
  }

  /** `]` (1) and `[` (-1): the size of the active paint tool. */
  stepBrushSize(direction: 1 | -1): void {
    const tool = this.paintTool;
    if (tool !== null) {
      this.updateBrush(tool, { size: steppedSize(this.brushOf(tool).size, direction) });
    }
  }

  /** `}` (1) and `{` (-1): the hardness of the active paint tool. */
  stepBrushHardness(direction: 1 | -1): void {
    const tool = this.paintTool;
    if (tool !== null) {
      this.updateBrush(tool, { hardness: steppedHardness(this.brushOf(tool).hardness, direction) });
    }
  }

  /** `X`: swaps the foreground and the background color. */
  swapColors(): void {
    [this.foreground, this.background] = [this.background, this.foreground];
  }

  /** `D`: black foreground and white background. */
  resetColors(): void {
    this.foreground = DEFAULT_FOREGROUND;
    this.background = DEFAULT_BACKGROUND;
  }

  /**
   * The layer, target, and brush for a stroke of the active paint tool, or
   * null if no stroke can start.
   */
  strokeSpec(): { layerId: number; target: StrokeTargetDto; brush: BrushSettings } | null {
    const tool = this.paintTool;
    const layer = this.selected;
    if (tool === null || layer === null || this.paintBlockReason !== null) {
      return null;
    }
    const target = strokeTarget(tool, this.paintTarget, this.foreground, this.background);
    return target === null ? null : { layerId: layer.id, target, brush: strokeBrush(tool, this.paintTarget, this.brushOf(tool)) };
  }

  /**
   * Samples the color at a canvas pixel and sets the foreground color (or the
   * background color, with `toBackground`). `source` and `size` default to the
   * eyedropper options; the Alt+click of the brush uses all layers.
   * Errors and empty pixels show a toast. Returns true if a color was set.
   * The colors are shared by all tabs, so a tab switch while the call runs
   * does not cancel the result.
   */
  async sampleAt(
    x: number,
    y: number,
    options: { toBackground?: boolean; source?: SampleSource } = {},
  ): Promise<boolean> {
    const info = this.info;
    if (info === null) {
      return false;
    }
    const source = options.source ?? this.sampleSource;
    let layerId: number | null = null;
    if (source === 'layer') {
      const layer = this.selected;
      if (layer === null || layer.kind !== 'pixel') {
        this.#toasts.info('Select a pixel layer to sample the current layer.');
        return false;
      }
      layerId = layer.id;
    }
    try {
      const sampled = await this.#api.sampleColor(info.documentId, Math.floor(x), Math.floor(y), this.sampleSize, source, layerId);
      if (sampled.color === null) {
        this.#toasts.info('No color here: the pixels are transparent.');
        return false;
      }
      if (options.toBackground === true) {
        this.background = sampled.color;
      } else {
        this.foreground = sampled.color;
      }
      return true;
    } catch (error) {
      this.#toasts.error('Sample the color failed.', error);
      return false;
    }
  }

  /**
   * The mask thumbnail of a layer of the active tab, or null while it loads or
   * if it failed. A thumbnail of an old `maskVersion` is not returned.
   */
  maskThumbnailOf(layer: LayerInfo): MaskThumbnail | null {
    const entry = this.#activeTab?.maskThumbs.get(layer.id);
    return entry !== undefined && entry.version === layer.maskVersion ? entry.thumb : null;
  }

  /**
   * Fetches the mask thumbnail of a layer of the active tab if the cache has
   * no thumbnail for its `maskVersion`. A late result for an old version or
   * a closed tab is dropped. An error is logged only (the chip stays plain).
   */
  requestMaskThumbnail(layer: LayerInfo, maxSize: number): void {
    const tab = this.#activeTab;
    const version = layer.maskVersion;
    const documentId = tab?.info?.documentId;
    if (tab === null || documentId === undefined || version === null || !layer.hasMask) {
      return;
    }
    if (tab.maskThumbs.get(layer.id)?.version === version || tab.maskThumbPending.get(layer.id) === version) {
      return;
    }
    tab.maskThumbPending.set(layer.id, version);
    const layerId = layer.id;
    this.#api
      .maskThumbnail(documentId, layerId, maxSize)
      .then((buffer) => {
        const thumb = decodeMaskThumbnail(buffer);
        const current = tab.info !== null ? findLayer(tab.info.layers, layerId) : null;
        if (tab.closed || current === null || current.maskVersion !== version) {
          return;
        }
        const next = new Map(tab.maskThumbs);
        next.set(layerId, { version, thumb });
        tab.maskThumbs = next;
      })
      .catch((error: unknown) => {
        console.error('Mask thumbnail failed.', error);
      })
      .finally(() => {
        if (tab.maskThumbPending.get(layerId) === version) {
          tab.maskThumbPending.delete(layerId);
        }
      });
  }

  /** The backend calls of this store. A stroke uses the same calls. */
  get api(): EditorApi {
    return this.#api;
  }

  /**
   * Takes the document info that a finished stroke returns. It goes to the
   * tab of that document (which can be inactive now), or nowhere if that
   * tab is closed.
   */
  applyInfo(next: DocumentInfo): void {
    this.#tabs.find((tab) => tab.info?.documentId === next.documentId)?.apply(next);
  }

  /** Sets the feather of the selection tools (0..250 px). */
  setFeather(radius: number): void {
    this.feather = clampFeather(radius);
  }

  /** Selects a shape, with the feather and anti-alias options of the tools. */
  selectShape(shape: SelectionShapeDto, mode: SelectionModeId): void {
    if (this.info !== null) {
      this.edit({ type: 'selectShape', shape, mode, antiAlias: this.antiAlias, feather: this.feather });
    }
  }

  /**
   * Magic wand click at a canvas point (the shell floors it). Without
   * "sample all layers" a selected layer is needed; else a toast shows and
   * nothing is sent.
   */
  magicWandAt(x: number, y: number, mode: SelectionModeId): void {
    if (this.info === null) {
      return;
    }
    const layerId = this.selectedId;
    if (layerId === null && !this.wand.sampleAllLayers) {
      this.#toasts.info('Select a layer to sample, or turn on "Sample all layers".');
      return;
    }
    this.edit(magicWandEdit(x, y, mode, this.wand, layerId));
  }

  /**
   * Paint bucket click at a canvas point. It needs a selected pixel layer as
   * the paint target (not a mask); else a toast shows and nothing is sent.
   */
  fillAt(x: number, y: number): void {
    if (this.info === null) {
      return;
    }
    const layer = this.selected;
    if (layer === null) {
      this.#toasts.info('Select a pixel layer to fill.');
      return;
    }
    if (this.paintTarget === 'mask' || layer.kind !== 'pixel') {
      this.#toasts.info('The paint bucket fills pixel layers. Select a pixel layer, not a mask.');
      return;
    }
    const color = hexToRgb(this.foreground);
    if (color === null) {
      this.#toasts.error('The foreground color is not valid.');
      return;
    }
    this.edit(fillEdit(layer.id, x, y, color, this.bucket));
  }

  /**
   * Gradient drag from `start` to `end` (canvas pixels, not rounded). The
   * target is the current paint target. The pixel target needs a selected
   * pixel layer; else a toast shows and nothing is sent. The foreground and
   * background presets read the colors now.
   */
  gradientFrom(start: { x: number; y: number }, end: { x: number; y: number }): void {
    if (this.info === null) {
      return;
    }
    const layer = this.selected;
    if (layer === null) {
      this.#toasts.info('Select a layer for the gradient.');
      return;
    }
    const target = this.paintTarget;
    if (target === 'pixels' && layer.kind !== 'pixel') {
      this.#toasts.info('The gradient draws on pixel layers or masks. Select a pixel layer or a mask.');
      return;
    }
    const foreground = hexToRgb(this.foreground);
    const background = hexToRgb(this.background);
    if (foreground === null || background === null) {
      this.#toasts.error('The foreground or background color is not valid.');
      return;
    }
    const stops = presetStops(this.gradient, foreground, background);
    this.edit(gradientEdit(layer.id, target, start, end, stops, this.gradient));
  }

  /** Opens the gradient editor dialog. */
  openGradientDialog(): void {
    if (this.status === 'ready') {
      this.gradientDialogOpen = true;
    }
  }

  closeGradientDialog(): void {
    this.gradientDialogOpen = false;
  }

  selectAll(): void {
    if (this.info !== null) {
      this.edit({ type: 'selectAll' });
    }
  }

  /** Removes the selection. The shell refuses this without a selection, so the store does nothing then. */
  deselect(): void {
    if (this.hasSelection) {
      this.edit({ type: 'deselect' });
    }
  }

  /** True if Reselect can restore the last removed selection. */
  get canReselect(): boolean {
    return this.info?.canReselect ?? false;
  }

  /** Restores the selection that the last Deselect removed. The shell refuses this when it cannot, so the store does nothing then. */
  reselect(): void {
    if (this.canReselect) {
      this.edit({ type: 'reselect' });
    }
  }

  invertSelection(): void {
    if (this.hasSelection) {
      this.edit({ type: 'invertSelection' });
    }
  }

  featherSelection(radius: number): void {
    if (this.hasSelection) {
      this.edit({ type: 'featherSelection', radius: clampFeather(radius) });
    }
  }

  /** Opens the Feather dialog (only with a selection). */
  openFeatherDialog(): void {
    if (this.hasSelection && this.status === 'ready') {
      this.featherDialogOpen = true;
    }
  }

  closeFeatherDialog(): void {
    this.featherDialogOpen = false;
  }

  /** Makes the mask of a layer (default: the selected layer) from the selection. */
  maskFromSelection(layerId: number | null = this.selectedId): void {
    if (layerId !== null && this.info !== null && canMaskFromSelection(this.info.layers, layerId)) {
      this.edit({ type: 'maskFromSelection', layerId });
    }
  }

  /**
   * Asks the shell for the outline of the selection. An error shows a toast
   * and gives null. The caller must compare the document and the selection
   * version of the answer with the current ones.
   */
  async selectionOutline(documentId: number, step: number): Promise<SelectionOutline | null> {
    try {
      return await this.#api.selectionOutline(documentId, step);
    } catch (error: unknown) {
      if (this.info?.documentId === documentId) {
        this.#toasts.error('Read the selection outline failed.', error);
      }
      return null;
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
   * active, no tab is open, the New document dialog is closed, and the user
   * did not close the last tab. Else null. The view opens this photo.
   *
   * This getter turns auto-open on again while another module is active.
   * The view reads it in an effect that runs at each module change, so the
   * next visit of the Editor module opens the photo again.
   */
  get openTarget(): PhotoId | null {
    if (this.#modules.active !== 'editor') {
      this.#autoOpenOff = false;
      return null;
    }
    if (this.#autoOpenOff || this.newDocumentOpen || this.#tabs.length > 0) {
      return null;
    }
    return this.candidate;
  }

  /** Opens a photo in the Editor and shows the module. */
  async editInEditor(photoId: PhotoId | null): Promise<void> {
    if (photoId === null) {
      this.#toasts.info('Select a photo to edit it in the Editor.');
      return;
    }
    this.#modules.switchTo('editor');
    await this.openPhoto(photoId);
  }

  /**
   * Opens a photo in a new tab and makes the tab active. If a tab shows the
   * photo already (its source photo or its saved photo, which opens the same
   * layered file), that tab becomes active instead.
   */
  async openPhoto(photoId: PhotoId): Promise<void> {
    const existing = this.#tabFor(photoId);
    if (existing !== null) {
      this.activate(existing);
      return;
    }
    await this.#openTab(new EditorTab(this.#nextTabId++, photoId), () => this.#api.open(photoId, this.#displaySpace), 'Open the photo in the Editor');
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

  /** Makes a new empty document in a new tab. Returns true if the document opened. */
  async newDocument(spec: NewDocumentSpec): Promise<boolean> {
    const tab = new EditorTab(this.#nextTabId++, null, spec.name);
    const ok = await this.#openTab(tab, () => this.#api.newDocument(spec, this.#displaySpace), 'Make the new document');
    if (ok) {
      this.#untitled += 1;
    }
    return ok;
  }

  /**
   * The shared flow of `openPhoto` and `newDocument`: add the tab, make it
   * active, and open its document. Returns true if the document is open.
   */
  async #openTab(tab: EditorTab, open: () => Promise<DocumentInfo>, action: string): Promise<boolean> {
    // The tab goes into the strip before the first await, so that auto-open
    // sees it at once.
    this.#tabs = [...this.#tabs, tab];
    this.activate(tab);
    let info: DocumentInfo;
    try {
      info = await open();
    } catch (error: unknown) {
      if (tab.closed) {
        return false;
      }
      tab.status = 'error';
      tab.loadError = describeError(error);
      this.#toasts.error(this.#messageFor(action, error), error);
      return false;
    }
    if (tab.closed) {
      // The user closed the tab while the document opened.
      await this.#closeDocument(info.documentId);
      return false;
    }
    tab.opened(info);
    if (tab === this.#activeTab) {
      tab.refit(this.viewport);
    }
    return true;
  }

  /** Closes all documents (the view is destroyed). */
  async dispose(): Promise<void> {
    // An open question has no view any more.
    this.answerLeave('cancel');
    const tabs = this.#tabs;
    for (const tab of tabs) {
      tab.markClosed();
    }
    this.#tabs = [];
    this.#activeTab = null;
    await Promise.all(tabs.flatMap((tab) => (tab.info === null ? [] : [this.#closeDocument(tab.info.documentId)])));
  }

  // ---- Save ----

  /**
   * Saves the active document to the library. Waits for the queued edits
   * first. A second call while a save runs returns the same result.
   * Returns true on success. Errors show a toast.
   */
  save(): Promise<boolean> {
    const tab = this.#activeTab;
    return tab === null ? Promise.resolve(false) : this.#saveTab(tab);
  }

  /**
   * "Save As": asks for a folder, then saves the active document as new
   * files in that folder (a new library photo). The earlier files stay as
   * they are. Returns false if the user cancels the dialog or the save fails.
   */
  saveAs(): Promise<boolean> {
    const tab = this.#activeTab;
    return tab === null ? Promise.resolve(false) : this.#saveTabAs(tab);
  }

  #saveTab(tab: EditorTab): Promise<boolean> {
    // A new document has no source photo and no saved files: it needs a folder.
    const info = tab.info;
    if (info !== null && info.savedPhotoId === null && info.sourcePhotoId === null) {
      return this.#saveTabAs(tab);
    }
    return this.#startSave(tab);
  }

  async #saveTabAs(tab: EditorTab): Promise<boolean> {
    if (!tab.canSave) {
      return tab.saveRun ?? false;
    }
    let folder: string | null;
    try {
      folder = await this.#pickFolder();
    } catch (error: unknown) {
      this.#toasts.error('Open the folder picker failed.', error);
      return false;
    }
    if (folder === null || tab.closed) {
      return false;
    }
    return this.#startSave(tab, folder);
  }

  #startSave(tab: EditorTab, folder?: string): Promise<boolean> {
    if (tab.saveRun !== null) {
      return tab.saveRun;
    }
    if (!tab.canSave) {
      return Promise.resolve(false);
    }
    const run = this.#runSave(tab, folder).finally(() => {
      tab.saveRun = null;
    });
    tab.saveRun = run;
    return run;
  }

  async #runSave(tab: EditorTab, folder?: string): Promise<boolean> {
    tab.endGesture();
    tab.saving = true;
    try {
      // The save must contain the edits that the user made before it.
      await tab.chain;
      const info = tab.info;
      if (info === null || tab.closed) {
        return false;
      }
      const result = await this.#api.save(info.documentId, folder);
      this.#applySaved(tab, result);
      this.#toasts.success(`Saved to library: ${fileNameOf(result.imagePath)}`);
      this.#onSaved?.(result);
      return true;
    } catch (error: unknown) {
      if (!tab.closed) {
        let message = this.#messageFor('Save the document', error);
        // A plain save writes next to the source photo; that folder can be read-only.
        if (folder === undefined && errorKind(error) === 'io') {
          message += ' Try "Save As…" to choose a different folder.';
        }
        this.#toasts.error(message, error);
      }
      return false;
    } finally {
      tab.saving = false;
    }
  }

  /** Takes the save result. Edits after the save snapshot keep their newer info. */
  #applySaved(tab: EditorTab, result: SaveResult): void {
    const current = tab.info;
    if (tab.closed || current === null || current.documentId !== result.info.documentId) {
      return;
    }
    if (current.revision === result.info.revision) {
      tab.apply(result.info);
    } else {
      // The saved file can have a new name (Save As), so the name comes from the save, too.
      tab.info = { ...current, savedPhotoId: result.info.savedPhotoId, name: result.info.name };
    }
  }

  /**
   * Asks the user what to do with the unsaved changes of the active tab.
   * Returns true if the caller may go on (saved, or the user does not want
   * to save), false on Cancel or a failed save.
   */
  async confirmLeave(): Promise<boolean> {
    const tab = this.#activeTab;
    if (tab === null) {
      return true;
    }
    await this.#settle(tab);
    return this.#confirmTab(tab);
  }

  /** Asks about the unsaved changes of a tab (the caller waited for its edits). */
  async #confirmTab(tab: EditorTab): Promise<boolean> {
    if (!tab.dirty) {
      return true;
    }
    if (this.#answerLeave !== null) {
      // A question is open already. This request does not go on.
      return false;
    }
    const choice = await new Promise<LeaveChoice>((resolve) => {
      this.#answerLeave = resolve;
      this.leavePrompt = { message: `"${tab.title}" has changes that are not saved.` };
    });
    if (choice === 'cancel') {
      return false;
    }
    if (choice === 'discard') {
      return true;
    }
    return this.#saveTab(tab);
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

  // ---- Layer selection ----

  select(id: number | null): void {
    const tab = this.#activeTab;
    if (tab === null) {
      return;
    }
    if (id !== tab.maskTarget) {
      tab.maskTarget = null;
    }
    // Another layer ends a free transform without an edit (ADR 0029).
    if (id !== tab.selectedId) {
      this.cancelTransform();
    }
    // A click on a layer row chooses the pixels as the paint target.
    tab.paintMaskId = null;
    tab.selectedId = id;
  }

  toggleExpanded(id: number): void {
    const tab = this.#activeTab;
    if (tab === null) {
      return;
    }
    const next = new Set(tab.expanded);
    if (!next.delete(id)) {
      next.add(id);
    }
    tab.expanded = next;
  }

  // ---- Edits ----

  /** Sends one edit to the active document, after the edits before it. */
  edit(edit: EditDto): void {
    const tab = this.#activeTab;
    if (tab === null) {
      return;
    }
    tab.endGesture();
    this.#enqueue(tab, async (info) => {
      const before = allLayerIds(info.layers);
      const next = await this.#api.edit(info.documentId, edit, null);
      if (tab.closed) {
        return;
      }
      tab.apply(next);
      if (edit.type === 'duplicate' || edit.type === 'addAdjustment' || edit.type === 'addPixelLayer') {
        const added = newLayerIds(before, next.layers);
        const top = added[added.length - 1];
        if (top !== undefined) {
          tab.maskTarget = null;
          tab.selectedId = top;
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
    const tab = this.#activeTab;
    if (tab === null) {
      return;
    }
    const gesture = tab.gestureFor(streamKey, this.#now());
    const queued = tab.live.has(streamKey);
    tab.live.set(streamKey, { edit, gesture });
    if (queued) {
      return;
    }
    this.#enqueue(tab, async (info) => {
      const entry = tab.live.get(streamKey);
      tab.live.delete(streamKey);
      if (entry === undefined) {
        return;
      }
      const next = await this.#api.edit(info.documentId, entry.edit, entry.gesture);
      tab.apply(next);
    });
  }

  /** Ends the current gesture (the slider is released). The next live edit starts a new undo step. */
  endGesture(): void {
    this.#activeTab?.endGesture();
  }

  // ---- Free transform (ADR 0029) ----

  /** The free transform of the active tab, or null. */
  get transform(): TransformSession | null {
    return this.#activeTab?.transform ?? null;
  }

  /** True while the transform sends its commit. */
  get transformBusy(): boolean {
    return this.#activeTab?.transformBusy ?? false;
  }

  /** The preview for the frame request: the layer and the matrix, or null without a transform. */
  get transformPreview(): { layerId: number; matrix: ReturnType<typeof stateMatrix> } | null {
    const session = this.transform;
    return session === null ? null : { layerId: session.layerId, matrix: stateMatrix(session.state) };
  }

  /**
   * Starts a free transform of the selected pixel layer. Shows a toast and
   * stays out of the mode if no pixel layer is selected or the layer is empty.
   */
  async startTransform(): Promise<void> {
    const tab = this.#activeTab;
    const info = tab?.info ?? null;
    if (tab === null || info === null || tab.status !== 'ready' || tab.transform !== null) {
      return;
    }
    const layer = this.selected;
    if (layer === null || layer.kind !== 'pixel') {
      this.#toasts.info('Select a pixel layer to transform.');
      return;
    }
    let bounds: LayerBounds | null;
    try {
      bounds = await this.#api.layerBounds(info.documentId, layer.id);
    } catch (error: unknown) {
      this.#toasts.error('Read the layer bounds failed.', error);
      return;
    }
    // The user can switch the tab or the layer while the call runs.
    if (tab.closed || this.#activeTab !== tab || tab.selectedId !== layer.id || tab.transform !== null) {
      return;
    }
    if (bounds === null) {
      this.#toasts.info('The layer is empty.');
      return;
    }
    tab.transform = { layerId: layer.id, state: identityState(bounds), interpolation: 'bicubic' };
  }

  /** Replaces the transform state (a drag, a field, a key). Ignored while the commit runs. */
  updateTransform(state: TransformState): void {
    const tab = this.#activeTab;
    if (tab !== null && tab.transform !== null && !tab.transformBusy) {
      tab.transform = { ...tab.transform, state };
    }
  }

  setTransformInterpolation(interpolation: Interpolation): void {
    const tab = this.#activeTab;
    if (tab !== null && tab.transform !== null && !tab.transformBusy) {
      tab.transform = { ...tab.transform, interpolation };
    }
  }

  /** Ends the transform of the active tab without an edit. Ignored while the commit runs. */
  cancelTransform(): void {
    const tab = this.#activeTab;
    if (tab !== null && !tab.transformBusy) {
      tab.transform = null;
    }
  }

  /**
   * Sends the transform as one edit and leaves the mode. A transform that
   * does nothing (identity matrix) only leaves the mode. If the edit fails,
   * a toast shows and the mode stays. Resolves to true if the mode ended.
   */
  async commitTransform(): Promise<boolean> {
    const tab = this.#activeTab;
    const session = tab?.transform ?? null;
    if (tab === null || session === null || tab.transformBusy) {
      return false;
    }
    if (isIdentityState(session.state)) {
      tab.transform = null;
      return true;
    }
    const edit: EditDto = { type: 'transformLayer', layerId: session.layerId, matrix: stateMatrix(session.state), interpolation: session.interpolation };
    tab.transformBusy = true;
    tab.endGesture();
    let done = false;
    this.#enqueue(tab, async (info) => {
      const next = await this.#api.edit(info.documentId, edit, null);
      if (tab.closed) {
        return;
      }
      // Leave the mode before the new info: the frame must not show the preview on top of the result.
      tab.transform = null;
      tab.apply(next);
      done = true;
    });
    await tab.chain;
    tab.transformBusy = false;
    return done;
  }

  /** True if the tab has a transform: the history command then ends it and does nothing else. */
  #endsTransform(tab: EditorTab): boolean {
    if (tab.transform === null && !tab.transformBusy) {
      return false;
    }
    if (!tab.transformBusy) {
      tab.transform = null;
    }
    return true;
  }

  undo(): void {
    this.#history('undo');
  }

  redo(): void {
    this.#history('redo');
  }

  /** Undoes or redoes until `step` steps are applied (history panel). No call if `step` is the current cursor. */
  historyJump(step: number): void {
    const tab = this.#activeTab;
    if (tab === null || this.#endsTransform(tab)) {
      return;
    }
    tab.endGesture();
    this.#enqueue(tab, async (info) => {
      // Test the cursor after the queued edits: they can change it.
      if (info.history.cursor === step) {
        return;
      }
      tab.apply(await this.#api.historyJump(info.documentId, step));
    });
  }

  #history(kind: 'undo' | 'redo'): void {
    const tab = this.#activeTab;
    if (tab === null || this.#endsTransform(tab)) {
      return;
    }
    tab.endGesture();
    this.#enqueue(tab, async (info) => {
      const next = kind === 'undo' ? await this.#api.undo(info.documentId) : await this.#api.redo(info.documentId);
      tab.apply(next);
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
    const view = this.view;
    return view === null ? null : Math.round(view.zoom * 100);
  }

  /** Sets the viewport size. The active tab fits again, or keeps its view inside the limits. */
  setViewport(size: Size): void {
    if (size.width === this.viewport.width && size.height === this.viewport.height) {
      return;
    }
    this.viewport = size;
    this.#activeTab?.fitTo(size);
  }

  fitToWindow(): void {
    const tab = this.#activeTab;
    if (tab !== null) {
      tab.fit = true;
      tab.refit(this.viewport);
    }
  }

  /** Zooms to a fixed value about the center of the viewport. */
  zoomTo(zoom: number): void {
    this.#changeView((view) => zoomAbout(view, clampZoom(zoom), { x: this.viewport.width / 2, y: this.viewport.height / 2 }));
  }

  zoomStep(direction: 1 | -1, anchor?: Point): void {
    const at = anchor ?? { x: this.viewport.width / 2, y: this.viewport.height / 2 };
    this.#changeView((view) => zoomAbout(view, steppedZoom(view.zoom, direction), at));
  }

  /** Zooms by a factor about a content-space point (wheel and pinch; the view unrotates the pointer first). */
  zoomBy(factor: number, anchor: Point): void {
    if (Number.isFinite(factor)) {
      this.#changeView((view) => zoomAbout(view, view.zoom * factor, anchor));
    }
  }

  /** Moves the content by a screen distance. With a view rotation, the content still follows the pointer. */
  pan(dx: number, dy: number): void {
    this.#changeView((view) => {
      const d = rotateVector({ x: dx, y: dy }, -view.rotation);
      return panBy(view, d.x, d.y);
    });
  }

  /** The view rotation of the active tab in degrees (0 without a view). */
  get rotation(): number {
    return this.view?.rotation ?? 0;
  }

  /**
   * Sets the view rotation of the active tab (degrees, normalized to
   * (-180, 180]). It is view state: no undo step, and "fit" stays on.
   */
  setRotation(degrees: number): void {
    const tab = this.#activeTab;
    const view = tab?.view ?? null;
    if (tab !== null && view !== null) {
      tab.setView({ ...view, rotation: normalizeRotation(degrees) }, this.viewport);
    }
  }

  /** Sets the view rotation of the active tab to 0. */
  resetRotation(): void {
    this.setRotation(0);
  }

  /** Changes the view of the active tab by the user; the view stops following "fit". */
  #changeView(change: (view: View) => View): void {
    const tab = this.#activeTab;
    const view = tab?.view ?? null;
    if (tab === null || view === null) {
      return;
    }
    tab.fit = false;
    tab.setView(change(view), this.viewport);
  }

  // ---- Internal ----

  /**
   * Runs a task of a tab after the tasks of the tab before it. The task gets
   * the info at its start. Errors go to a toast. A closed tab skips its tasks.
   */
  #enqueue(tab: EditorTab, task: (info: DocumentInfo) => Promise<void>): void {
    tab.pending += 1;
    tab.chain = tab.chain
      .then(async () => {
        const info = tab.info;
        if (info === null || tab.closed) {
          return;
        }
        await task(info);
      })
      .catch((error: unknown) => {
        if (!tab.closed) {
          this.#toasts.error(this.#messageFor('Edit the document', error), error);
        }
      })
      .finally(() => {
        tab.pending -= 1;
      });
  }

  #messageFor(action: string, error: unknown): string {
    if (errorKind(error) === 'unavailable') {
      return `${action} failed: the Editor needs a GPU, and none is available.`;
    }
    return `${action} failed.`;
  }
}
