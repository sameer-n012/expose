/**
 * Navigation between the grid view and the loupe view of the Library module.
 *
 * The selection is the single source of truth. The loupe shows the photo at
 * the focus position of the selection. A step in the loupe moves the
 * selection. Thus, when the user goes back to the grid, the same photo is
 * selected and the grid scrolls to it. Rating, flag, and label shortcuts act
 * on the selection, so they act on the loupe photo.
 */
import type { LibraryStore } from '../stores/library.svelte';
import type { ModuleStore } from '../stores/modules.svelte';
import type { ShortcutAction, ShortcutRegistry } from '../shortcuts/registry';
import type { LibraryCommands } from './libraryActions';

/** Ids of the navigation actions. */
export const NAV_ACTION = {
  openLoupe: 'library.loupe.open',
  closeLoupe: 'library.loupe.close',
} as const;

/** True if the loupe can open: the view has photos and one has the focus. */
export function canOpenLoupe(library: LibraryStore): boolean {
  const focus = library.focusIndex;
  return library.total > 0 && focus !== null && focus < library.total;
}

/** Opens the loupe on the focused photo. Does nothing if `canOpenLoupe` is false. */
export function openLoupeOnSelection(modules: ModuleStore, library: LibraryStore): void {
  if (canOpenLoupe(library)) {
    modules.libraryView = 'loupe';
  }
}

/** Selects the photo at `index` and opens the loupe on it (double-click). */
export function openLoupeAt(modules: ModuleStore, library: LibraryStore, index: number): void {
  library.click(index, { toggle: false, range: false });
  modules.libraryView = 'loupe';
}

/** True if the Library module is active and shows the loupe. */
export function loupeShows(modules: ModuleStore): boolean {
  return modules.active === 'library' && modules.libraryView === 'loupe';
}

/** Returns to the grid if the loupe shows. */
export function closeLoupe(modules: ModuleStore): void {
  if (loupeShows(modules)) {
    modules.libraryView = 'grid';
  }
}

/**
 * Wraps the library commands for the loupe. In the loupe, left and right
 * step to the previous and next photo (clamped, no wrap). Up, down, and
 * extend have no meaning there and do nothing. In the grid, the commands do
 * not change.
 */
export function loupeAwareCommands(
  modules: ModuleStore,
  library: LibraryStore,
  commands: LibraryCommands,
): LibraryCommands {
  return {
    ...commands,
    moveSelection: (dx, dy, extend) => {
      if (modules.libraryView !== 'loupe') {
        commands.moveSelection(dx, dy, extend);
        return;
      }
      if (dy === 0 && !extend && dx !== 0) {
        library.moveSelection(Math.sign(dx), 0, false);
      }
    },
  };
}

/** Creates the navigation actions (scope `library`). */
export function libraryNavigationActions(modules: ModuleStore, library: LibraryStore): ShortcutAction[] {
  return [
    {
      id: NAV_ACTION.openLoupe,
      label: 'Library: loupe view',
      scope: 'library',
      defaultBinding: 'L',
      run: () => openLoupeOnSelection(modules, library),
    },
    {
      id: NAV_ACTION.closeLoupe,
      label: 'Library: back to grid',
      scope: 'library',
      defaultBinding: 'Escape',
      run: () => closeLoupe(modules),
    },
  ];
}

/**
 * Registers the navigation actions.
 * @returns a function that removes them again.
 */
export function registerLibraryNavigation(
  registry: ShortcutRegistry,
  modules: ModuleStore,
  library: LibraryStore,
): () => void {
  const removers: Array<() => void> = [];
  try {
    for (const action of libraryNavigationActions(modules, library)) {
      removers.push(registry.register(action));
    }
  } catch (error: unknown) {
    // Do not leave a part of the actions registered.
    for (const remove of removers) {
      remove();
    }
    throw error;
  }
  return () => {
    for (const remove of removers) {
      remove();
    }
  };
}
