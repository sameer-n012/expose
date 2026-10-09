/**
 * The shortcut actions of the Editor.
 *
 * - `Mod+E` (global): open the selected photo in the Editor.
 * - `Mod+Z`, `Mod+Shift+Z` (scope `editor`): undo and redo.
 * - `Mod+=`, `Mod+-`, `Mod+0`, `Mod+1` (scope `editor`): zoom in, zoom out,
 *   fit to window, 100 %.
 */
import type { ShortcutAction } from '../shortcuts/registry';

export const EDITOR_ACTION = {
  edit: 'editor.open',
  undo: 'editor.undo',
  redo: 'editor.redo',
  zoomIn: 'editor.zoomIn',
  zoomOut: 'editor.zoomOut',
  zoomFit: 'editor.zoomFit',
  zoom100: 'editor.zoom100',
} as const;

/** The commands that the Editor actions call (the editor store has them). */
export interface EditorCommands {
  undo(): void;
  redo(): void;
  zoomStep(direction: 1 | -1): void;
  fitToWindow(): void;
  zoomTo(zoom: number): void;
}

/** The action that opens the selected photo in the Editor. The shell knows the photo. */
export function editInEditorAction(run: () => void): ShortcutAction {
  return { id: EDITOR_ACTION.edit, label: 'Edit in Editor', scope: 'global', defaultBinding: 'Mod+E', run };
}

/** The actions that work while the Editor is active. */
export function editorActions(commands: EditorCommands): ShortcutAction[] {
  return [
    { id: EDITOR_ACTION.undo, label: 'Editor: undo', scope: 'editor', defaultBinding: 'Mod+Z', allowRepeat: true, run: () => commands.undo() },
    { id: EDITOR_ACTION.redo, label: 'Editor: redo', scope: 'editor', defaultBinding: 'Mod+Shift+Z', allowRepeat: true, run: () => commands.redo() },
    { id: EDITOR_ACTION.zoomIn, label: 'Editor: zoom in', scope: 'editor', defaultBinding: 'Mod+=', allowRepeat: true, run: () => commands.zoomStep(1) },
    { id: EDITOR_ACTION.zoomOut, label: 'Editor: zoom out', scope: 'editor', defaultBinding: 'Mod+-', allowRepeat: true, run: () => commands.zoomStep(-1) },
    { id: EDITOR_ACTION.zoomFit, label: 'Editor: fit to window', scope: 'editor', defaultBinding: 'Mod+0', run: () => commands.fitToWindow() },
    { id: EDITOR_ACTION.zoom100, label: 'Editor: zoom to 100 %', scope: 'editor', defaultBinding: 'Mod+1', run: () => commands.zoomTo(1) },
  ];
}
