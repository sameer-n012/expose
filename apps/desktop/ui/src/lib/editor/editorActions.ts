/**
 * The shortcut actions of the Editor.
 *
 * - `Mod+E` (global): open the selected photo in the Editor.
 * - `Mod+N` (global): open the New document dialog (shows the Editor).
 * - `Mod+Shift+N` (scope `editor`): add a pixel layer above the selected layer.
 * - `\` (scope `editor`): show or hide the mask of the selected layer.
 * - `Mod+S` (scope `editor`): save the document to the library.
 * - `Mod+Shift+S` (scope `editor`): save as new files in a chosen folder.
 * - `Mod+Z`, `Mod+Shift+Z` (scope `editor`): undo and redo.
 * - `Mod+A`, `Mod+D`, `Mod+Shift+I`, `Shift+F6` (scope `editor`): select all,
 *   deselect, invert the selection, open the Feather dialog.
 * - `M`, `L`, `H`, `R` (scope `editor`): marquee tool (rectangle, then ellipse),
 *   lasso (the last used kind; `L` again: freehand or polygon), hand, rotate view.
 * - "Editor: reset view rotation" (scope `editor`, no default key).
 * - `B`, `E` (scope `editor`): brush and eraser tool.
 * - `[`, `]`, `{`, `}` (scope `editor`): brush size and hardness (`{` and `}`
 *   are Shift+`[` and Shift+`]`).
 * - `X`, `D` (scope `editor`): swap the colors, default colors.
 * - `Mod+=`, `Mod+-`, `Mod+0`, `Mod+1` (scope `editor`): zoom in, zoom out,
 *   fit to window, 100 %.
 * - `Mod+W` (scope `editor`): close the active document tab (a tab with
 *   unsaved changes asks first).
 * - `Ctrl+Tab`, `Ctrl+Shift+Tab` (scope `editor`): next and previous document
 *   tab (physical Control on all platforms; it wraps at the ends).
 */
import type { ShortcutAction } from '../shortcuts/registry';
import type { EditorTool } from './selection';

export const EDITOR_ACTION = {
  edit: 'editor.open',
  save: 'editor.save',
  saveAs: 'editor.saveAs',
  undo: 'editor.undo',
  redo: 'editor.redo',
  zoomIn: 'editor.zoomIn',
  zoomOut: 'editor.zoomOut',
  zoomFit: 'editor.zoomFit',
  zoom100: 'editor.zoom100',
  newDocument: 'editor.newDocument',
  newLayer: 'editor.newLayer',
  showMask: 'editor.showMask',
  selectAll: 'editor.selectAll',
  deselect: 'editor.deselect',
  reselect: 'editor.reselect',
  invertSelection: 'editor.invertSelection',
  featherSelection: 'editor.featherSelection',
  marqueeTool: 'editor.marqueeTool',
  lassoTool: 'editor.lassoTool',
  handTool: 'editor.handTool',
  rotateTool: 'editor.rotateTool',
  resetRotation: 'editor.resetRotation',
  brushTool: 'editor.brushTool',
  eraserTool: 'editor.eraserTool',
  eyedropperTool: 'editor.eyedropperTool',
  magicWandTool: 'editor.magicWandTool',
  paintBucketTool: 'editor.paintBucketTool',
  brushSizeDown: 'editor.brushSizeDown',
  brushSizeUp: 'editor.brushSizeUp',
  brushHarder: 'editor.brushHarder',
  brushSofter: 'editor.brushSofter',
  swapColors: 'editor.swapColors',
  resetColors: 'editor.resetColors',
  closeTab: 'editor.closeTab',
  nextTab: 'editor.nextTab',
  previousTab: 'editor.previousTab',
} as const;

/** The commands that the Editor actions call (the editor store has them). */
export interface EditorCommands {
  save(): Promise<unknown>;
  saveAs(): Promise<unknown>;
  undo(): void;
  redo(): void;
  zoomStep(direction: 1 | -1): void;
  fitToWindow(): void;
  zoomTo(zoom: number): void;
  resetRotation(): void;
  requestNewDocument(): void;
  addPixelLayer(): void;
  toggleShowMask(): void;
  selectAll(): void;
  deselect(): void;
  reselect(): void;
  invertSelection(): void;
  openFeatherDialog(): void;
  cycleMarquee(): void;
  cycleLasso(): void;
  cycleFill(): void;
  setTool(tool: EditorTool): void;
  selectPaintTool(tool: 'brush' | 'eraser'): void;
  stepBrushSize(direction: 1 | -1): void;
  stepBrushHardness(direction: 1 | -1): void;
  swapColors(): void;
  resetColors(): void;
  closeActiveTab(): void;
  nextTab(step: 1 | -1): void;
}

/** The action that opens the selected photo in the Editor. The shell knows the photo. */
export function editInEditorAction(run: () => void): ShortcutAction {
  return { id: EDITOR_ACTION.edit, label: 'Edit in Editor', scope: 'global', defaultBinding: 'Mod+E', run };
}

/** The actions that work while the Editor is active. */
export function editorActions(commands: EditorCommands): ShortcutAction[] {
  return [
    // Global: the command shows the Editor module and opens the dialog there.
    { id: EDITOR_ACTION.newDocument, label: 'Editor: new document', scope: 'global', defaultBinding: 'Mod+N', run: () => commands.requestNewDocument() },
    { id: EDITOR_ACTION.newLayer, label: 'Editor: new layer', scope: 'editor', defaultBinding: 'Mod+Shift+N', run: () => commands.addPixelLayer() },
    // The store ignores the key when the selected layer has no mask.
    { id: EDITOR_ACTION.showMask, label: 'Editor: show layer mask', scope: 'editor', defaultBinding: '\\', run: () => commands.toggleShowMask() },
    // The store does nothing for deselect, invert, and feather when there is no selection.
    { id: EDITOR_ACTION.selectAll, label: 'Editor: select all', scope: 'editor', defaultBinding: 'Mod+A', run: () => commands.selectAll() },
    { id: EDITOR_ACTION.deselect, label: 'Editor: deselect', scope: 'editor', defaultBinding: 'Mod+D', run: () => commands.deselect() },
    // The store sends no edit when there is nothing to reselect.
    { id: EDITOR_ACTION.reselect, label: 'Editor: reselect', scope: 'editor', defaultBinding: 'Mod+Shift+D', run: () => commands.reselect() },
    { id: EDITOR_ACTION.invertSelection, label: 'Editor: invert selection', scope: 'editor', defaultBinding: 'Mod+Shift+I', run: () => commands.invertSelection() },
    { id: EDITOR_ACTION.featherSelection, label: 'Editor: feather selection…', scope: 'editor', defaultBinding: 'Shift+F6', run: () => commands.openFeatherDialog() },
    { id: EDITOR_ACTION.marqueeTool, label: 'Editor: marquee tool (rectangle, ellipse)', scope: 'editor', defaultBinding: 'M', run: () => commands.cycleMarquee() },
    { id: EDITOR_ACTION.lassoTool, label: 'Editor: lasso tool (freehand, polygon)', scope: 'editor', defaultBinding: 'L', run: () => commands.cycleLasso() },
    { id: EDITOR_ACTION.handTool, label: 'Editor: hand tool', scope: 'editor', defaultBinding: 'H', run: () => commands.setTool('hand') },
    // The develop crop `R` is in the develop scope: no collision.
    { id: EDITOR_ACTION.rotateTool, label: 'Editor: rotate view tool', scope: 'editor', defaultBinding: 'R', run: () => commands.setTool('rotate') },
    { id: EDITOR_ACTION.resetRotation, label: 'Editor: reset view rotation', scope: 'editor', defaultBinding: null, run: () => commands.resetRotation() },
    // In the editor scope, `E` and `D` take precedence over the global module keys.
    { id: EDITOR_ACTION.brushTool, label: 'Editor: brush tool', scope: 'editor', defaultBinding: 'B', run: () => commands.selectPaintTool('brush') },
    { id: EDITOR_ACTION.eraserTool, label: 'Editor: eraser tool', scope: 'editor', defaultBinding: 'E', run: () => commands.selectPaintTool('eraser') },
    { id: EDITOR_ACTION.eyedropperTool, label: 'Editor: eyedropper tool', scope: 'editor', defaultBinding: 'I', run: () => commands.setTool('eyedropper') },
    // In the editor scope, `G` takes precedence over the global Library grid key.
    { id: EDITOR_ACTION.magicWandTool, label: 'Editor: magic wand tool', scope: 'editor', defaultBinding: 'W', run: () => commands.setTool('magicWand') },
    { id: EDITOR_ACTION.paintBucketTool, label: 'Editor: paint bucket / gradient tool', scope: 'editor', defaultBinding: 'G', run: () => commands.cycleFill() },
    { id: EDITOR_ACTION.brushSizeDown, label: 'Editor: brush smaller', scope: 'editor', defaultBinding: '[', allowRepeat: true, run: () => commands.stepBrushSize(-1) },
    { id: EDITOR_ACTION.brushSizeUp, label: 'Editor: brush larger', scope: 'editor', defaultBinding: ']', allowRepeat: true, run: () => commands.stepBrushSize(1) },
    // Shift+[ and Shift+] give the keys `{` and `}`. The key registry ignores Shift for symbols, so "Shift+[" would be the same chord as "[".
    { id: EDITOR_ACTION.brushSofter, label: 'Editor: brush softer', scope: 'editor', defaultBinding: '{', allowRepeat: true, run: () => commands.stepBrushHardness(-1) },
    { id: EDITOR_ACTION.brushHarder, label: 'Editor: brush harder', scope: 'editor', defaultBinding: '}', allowRepeat: true, run: () => commands.stepBrushHardness(1) },
    { id: EDITOR_ACTION.swapColors, label: 'Editor: swap foreground and background', scope: 'editor', defaultBinding: 'X', run: () => commands.swapColors() },
    { id: EDITOR_ACTION.resetColors, label: 'Editor: default colors', scope: 'editor', defaultBinding: 'D', run: () => commands.resetColors() },
    { id: EDITOR_ACTION.save, label: 'Editor: save to library', scope: 'editor', defaultBinding: 'Mod+S', run: () => void commands.save() },
    { id: EDITOR_ACTION.saveAs, label: 'Editor: save as…', scope: 'editor', defaultBinding: 'Mod+Shift+S', run: () => void commands.saveAs() },
    { id: EDITOR_ACTION.undo, label: 'Editor: undo', scope: 'editor', defaultBinding: 'Mod+Z', allowRepeat: true, run: () => commands.undo() },
    { id: EDITOR_ACTION.redo, label: 'Editor: redo', scope: 'editor', defaultBinding: 'Mod+Shift+Z', allowRepeat: true, run: () => commands.redo() },
    { id: EDITOR_ACTION.zoomIn, label: 'Editor: zoom in', scope: 'editor', defaultBinding: 'Mod+=', allowRepeat: true, run: () => commands.zoomStep(1) },
    { id: EDITOR_ACTION.zoomOut, label: 'Editor: zoom out', scope: 'editor', defaultBinding: 'Mod+-', allowRepeat: true, run: () => commands.zoomStep(-1) },
    { id: EDITOR_ACTION.zoomFit, label: 'Editor: fit to window', scope: 'editor', defaultBinding: 'Mod+0', run: () => commands.fitToWindow() },
    { id: EDITOR_ACTION.zoom100, label: 'Editor: zoom to 100 %', scope: 'editor', defaultBinding: 'Mod+1', run: () => commands.zoomTo(1) },
    { id: EDITOR_ACTION.closeTab, label: 'Editor: close document tab', scope: 'editor', defaultBinding: 'Mod+W', run: () => commands.closeActiveTab() },
    { id: EDITOR_ACTION.nextTab, label: 'Editor: next document tab', scope: 'editor', defaultBinding: 'Ctrl+Tab', allowRepeat: true, run: () => commands.nextTab(1) },
    { id: EDITOR_ACTION.previousTab, label: 'Editor: previous document tab', scope: 'editor', defaultBinding: 'Ctrl+Shift+Tab', allowRepeat: true, run: () => commands.nextTab(-1) },
  ];
}
