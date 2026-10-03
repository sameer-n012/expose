/**
 * Creates the application state: module store, layout store, and shortcut
 * registry with the shell actions. The app root and the component tests use
 * this function, so that both get the same wiring.
 */
import { registerAppActions } from './appActions';
import { registerLibraryActions } from './library/libraryActions';
import type { Platform } from './shortcuts/keys';
import { ShortcutRegistry } from './shortcuts/registry';
import { LayoutStore } from './stores/layout.svelte';
import { LibraryStore } from './stores/library.svelte';
import { ModuleStore } from './stores/modules.svelte';
import type { ToastStore } from './stores/toasts.svelte';

export interface AppState {
  readonly modules: ModuleStore;
  readonly layout: LayoutStore;
  readonly library: LibraryStore;
  readonly registry: ShortcutRegistry;
  /** Removes the shell and library actions from the registry. */
  readonly dispose: () => void;
}

export interface AppStateOptions {
  readonly platform: Platform;
  /** Receives errors from shortcut action handlers. */
  readonly toasts: ToastStore;
}

export function createAppState({ platform, toasts }: AppStateOptions): AppState {
  const modules = new ModuleStore();
  const layout = new LayoutStore();
  const registry = new ShortcutRegistry(platform, {
    onError: (error, action) => toasts.error(`The action "${action.label}" failed.`, error),
  });
  const library = new LibraryStore(toasts);
  const removeAppActions = registerAppActions(registry, { modules, layout });
  let removeLibraryActions: () => void;
  try {
    removeLibraryActions = registerLibraryActions(registry, library.commands);
  } catch (error: unknown) {
    removeAppActions();
    throw error;
  }
  const dispose = (): void => {
    removeLibraryActions();
    removeAppActions();
  };
  return { modules, layout, library, registry, dispose };
}
