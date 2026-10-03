/**
 * The shortcut actions of the application shell (module switching and
 * panel visibility). Module-specific actions are registered by the modules.
 */
import type { LayoutStore } from './stores/layout.svelte';
import type { ModuleStore } from './stores/modules.svelte';
import type { ShortcutAction, ShortcutRegistry } from './shortcuts/registry';

/** Stable ids of the shell actions. Use these to look up bindings. */
export const APP_ACTION = {
  libraryGrid: 'module.library.grid',
  develop: 'module.develop',
  editor: 'module.editor',
  toggleSidePanels: 'panels.toggleSides',
  toggleAllPanels: 'panels.toggleAll',
} as const;

export interface AppActionTargets {
  readonly modules: ModuleStore;
  readonly layout: LayoutStore;
}

/** Creates the shell action definitions. */
export function appActions({ modules, layout }: AppActionTargets): ShortcutAction[] {
  return [
    {
      id: APP_ACTION.libraryGrid,
      label: 'Library: grid view',
      scope: 'global',
      defaultBinding: 'G',
      run: () => modules.showLibraryGrid(),
    },
    {
      id: APP_ACTION.develop,
      label: 'Develop',
      scope: 'global',
      defaultBinding: 'D',
      run: () => modules.switchTo('develop'),
    },
    {
      id: APP_ACTION.editor,
      label: 'Editor',
      scope: 'global',
      defaultBinding: 'E',
      run: () => modules.switchTo('editor'),
    },
    {
      id: APP_ACTION.toggleSidePanels,
      label: 'Show or hide side panels',
      scope: 'global',
      defaultBinding: 'Tab',
      run: () => layout.toggleSides(),
    },
    {
      id: APP_ACTION.toggleAllPanels,
      label: 'Show or hide all panels',
      scope: 'global',
      defaultBinding: 'Shift+Tab',
      run: () => layout.toggleAll(),
    },
  ];
}

/**
 * Registers the shell actions.
 * @returns a function that removes them again.
 */
export function registerAppActions(registry: ShortcutRegistry, targets: AppActionTargets): () => void {
  const removers: Array<() => void> = [];
  const removeAll = (): void => {
    for (const remove of removers) {
      remove();
    }
  };
  try {
    for (const action of appActions(targets)) {
      removers.push(registry.register(action));
    }
  } catch (error: unknown) {
    // Do not leave a part of the actions registered.
    removeAll();
    throw error;
  }
  return removeAll;
}
