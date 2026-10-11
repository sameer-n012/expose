/**
 * Asks the user before the window closes while the Editor document has
 * changes that are not saved.
 *
 * The shell installs the guard only while the document is dirty. When a
 * close listener is installed, the window does not close by itself: the
 * listener closes it (`destroy`) if the handler does not prevent it. So a
 * clean document closes the window with no listener in the path.
 */

/** The close event: `preventDefault` keeps the window open. */
export interface CloseEvent {
  preventDefault(): void;
}

/** Installs a close handler and returns the function that removes it. */
export type CloseListener = (handler: (event: CloseEvent) => Promise<void>) => Promise<() => void>;

/** The close listener of the Tauri window. */
export const tauriCloseListener: CloseListener = async (handler) => {
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  return getCurrentWindow().onCloseRequested(handler);
};

/**
 * Installs a guard: on a close request, `confirmLeave` runs; if it returns
 * false (Cancel, or a failed save), the window stays open. Returns the
 * function that removes the guard. `onError` receives an install failure.
 */
export function guardWindowClose(
  listen: CloseListener,
  confirmLeave: () => Promise<boolean>,
  onError: (error: unknown) => void,
): () => void {
  let unlisten: (() => void) | null = null;
  let removed = false;
  listen(async (event) => {
    if (!(await confirmLeave())) {
      event.preventDefault();
    }
  }).then(
    (remove) => {
      if (removed) {
        remove();
      } else {
        unlisten = remove;
      }
    },
    onError,
  );
  return () => {
    removed = true;
    unlisten?.();
    unlisten = null;
  };
}
