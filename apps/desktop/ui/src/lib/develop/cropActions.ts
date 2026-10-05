/**
 * The shortcut actions of the crop tool (scope `develop`).
 *
 * - `R` starts the crop tool, or ends it and keeps the crop. It is always
 *   registered while the shell lives.
 * - `Enter` (keep), `Escape` (cancel), `O` (next overlay), and `X` (swap
 *   portrait and landscape) are registered only while the crop tool is
 *   active, so that these keys keep their normal function at other times.
 */
import type { ShortcutAction, ShortcutRegistry } from '../shortcuts/registry';

/** Stable ids of the crop actions. */
export const CROP_ACTION = {
  toggle: 'develop.crop.toggle',
  commit: 'develop.crop.commit',
  cancel: 'develop.crop.cancel',
  overlay: 'develop.crop.overlay',
  swap: 'develop.crop.swapOrientation',
} as const;

/** The commands that the crop actions call (the develop store has them). */
export interface CropCommands {
  toggleCrop(): void;
  commitCrop(): void;
  cancelCrop(): void;
  cycleCropOverlay(): void;
  swapCropOrientation(): void;
}

/** The action that starts or ends the crop tool. */
export function cropToggleAction(commands: CropCommands): ShortcutAction {
  return {
    id: CROP_ACTION.toggle,
    label: 'Crop tool',
    scope: 'develop',
    defaultBinding: 'R',
    run: () => commands.toggleCrop(),
  };
}

/** The actions that exist only while the crop tool is active. */
export function cropModeActions(commands: CropCommands): ShortcutAction[] {
  return [
    {
      id: CROP_ACTION.commit,
      label: 'Crop: done',
      scope: 'develop',
      defaultBinding: 'Enter',
      run: () => commands.commitCrop(),
    },
    {
      id: CROP_ACTION.cancel,
      label: 'Crop: cancel',
      scope: 'develop',
      defaultBinding: 'Escape',
      run: () => commands.cancelCrop(),
    },
    {
      id: CROP_ACTION.overlay,
      label: 'Crop: next overlay',
      scope: 'develop',
      defaultBinding: 'O',
      run: () => commands.cycleCropOverlay(),
    },
    {
      id: CROP_ACTION.swap,
      label: 'Crop: swap portrait and landscape',
      scope: 'develop',
      defaultBinding: 'X',
      run: () => commands.swapCropOrientation(),
    },
  ];
}

/**
 * Registers all `actions`, or none when one fails.
 * @returns a function that removes them again.
 */
export function registerActions(registry: ShortcutRegistry, actions: readonly ShortcutAction[]): () => void {
  const removers: Array<() => void> = [];
  const removeAll = (): void => {
    for (const remove of removers) {
      remove();
    }
  };
  try {
    for (const action of actions) {
      removers.push(registry.register(action));
    }
  } catch (error: unknown) {
    removeAll();
    throw error;
  }
  return removeAll;
}
