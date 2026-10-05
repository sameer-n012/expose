/**
 * State of the active application module.
 *
 * Module views mount when the user opens them the first time and then stay
 * mounted. A module that is not active is hidden, not destroyed. Thus a
 * switch is instant and each module keeps its state (scroll position, GPU
 * resources, open documents). `mounted` tells the view host which modules
 * to render.
 */
import { MODULE_IDS, type ModuleId } from '../modules';

/** View modes of the Library module. */
export type LibraryViewMode = 'grid' | 'loupe';

export class ModuleStore {
  /** The module that is visible now. */
  active = $state<ModuleId>('library');
  /** The view mode of the Library module. */
  libraryView = $state<LibraryViewMode>('grid');
  /** Modules that the user opened at least once. These stay mounted. */
  #mounted = $state<Record<ModuleId, boolean>>({ library: false, develop: false, editor: false });

  constructor(initial: ModuleId = 'library') {
    this.active = initial;
    this.#mounted[initial] = true;
  }

  /** The modules to render, in switcher order. */
  readonly mounted: readonly ModuleId[] = $derived(MODULE_IDS.filter((id) => this.#mounted[id]));

  /** Makes a module active. It mounts the module view if it is the first visit. */
  switchTo(id: ModuleId): void {
    this.#mounted[id] = true;
    this.active = id;
  }

  /**
   * Handles a click on a module tab. A click on the Library tab while the
   * Library loupe shows returns to the grid. Else it switches the module.
   */
  activateTab(id: ModuleId): void {
    if (id === 'library' && this.active === 'library' && this.libraryView === 'loupe') {
      this.libraryView = 'grid';
      return;
    }
    this.switchTo(id);
  }

  /** Opens the Library module in grid view. */
  showLibraryGrid(): void {
    this.libraryView = 'grid';
    this.switchTo('library');
  }

  /** True if the module view is mounted. */
  isMounted(id: ModuleId): boolean {
    return this.#mounted[id];
  }
}
