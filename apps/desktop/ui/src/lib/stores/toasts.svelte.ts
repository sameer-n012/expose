/**
 * Non-blocking notifications (toasts).
 *
 * Use `toasts.error(...)` for every failure that the user must know about,
 * for example a failed backend call. Do not drop errors silently.
 *
 * Info, success, and warning toasts close automatically after a timeout.
 * Error toasts stay until the user closes them, so that the user does not
 * miss an error.
 */

export type ToastKind = 'info' | 'success' | 'warning' | 'error';

export interface Toast {
  readonly id: number;
  readonly kind: ToastKind;
  readonly message: string;
  /** Optional technical detail, for example the error text from the backend. */
  readonly detail?: string;
}

export interface ToastOptions {
  readonly detail?: string;
  /** Time until the toast closes, in milliseconds. 0 means: stay until closed. */
  readonly timeoutMs?: number;
}

/** Default timeouts per kind, in milliseconds. 0 means: stay until closed. */
export const DEFAULT_TIMEOUT_MS: Readonly<Record<ToastKind, number>> = {
  info: 4000,
  success: 4000,
  warning: 8000,
  error: 0,
};

/** Maximum number of toasts on screen. The oldest toast closes first. */
export const MAX_TOASTS = 5;

/** Converts an unknown thrown value into a readable message. */
export function describeError(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === 'string') {
    return error;
  }
  try {
    return JSON.stringify(error) ?? String(error);
  } catch (jsonError: unknown) {
    // A value with circular references cannot be serialized; show its type
    // and the reason instead.
    return `${String(error)} (${jsonError instanceof Error ? jsonError.message : 'not serializable'})`;
  }
}

export class ToastStore {
  /** The toasts on screen, oldest first. */
  items = $state<Toast[]>([]);
  #nextId = 1;
  readonly #timers = new Map<number, ReturnType<typeof setTimeout>>();

  /** Shows a toast. Returns its id. */
  push(kind: ToastKind, message: string, options: ToastOptions = {}): number {
    const id = this.#nextId++;
    const toast: Toast = options.detail === undefined ? { id, kind, message } : { id, kind, message, detail: options.detail };
    this.items.push(toast);
    while (this.items.length > MAX_TOASTS) {
      const oldest = this.items[0];
      if (oldest === undefined) {
        break;
      }
      this.dismiss(oldest.id);
    }
    const timeout = options.timeoutMs ?? DEFAULT_TIMEOUT_MS[kind];
    if (timeout > 0) {
      this.#timers.set(
        id,
        setTimeout(() => this.dismiss(id), timeout),
      );
    }
    return id;
  }

  info(message: string, options?: ToastOptions): number {
    return this.push('info', message, options);
  }

  success(message: string, options?: ToastOptions): number {
    return this.push('success', message, options);
  }

  warning(message: string, options?: ToastOptions): number {
    return this.push('warning', message, options);
  }

  /**
   * Shows an error toast. `error` is the thrown value; its text becomes the
   * detail line. The error is also written to the console for debugging.
   */
  error(message: string, error?: unknown, options: Omit<ToastOptions, 'detail'> = {}): number {
    if (error !== undefined) {
      console.error(message, error);
    }
    return this.push('error', message, error === undefined ? options : { ...options, detail: describeError(error) });
  }

  /** Closes a toast. Does nothing if the toast is already closed. */
  dismiss(id: number): void {
    const timer = this.#timers.get(id);
    if (timer !== undefined) {
      clearTimeout(timer);
      this.#timers.delete(id);
    }
    const index = this.items.findIndex((t) => t.id === id);
    if (index !== -1) {
      this.items.splice(index, 1);
    }
  }

  /** Closes all toasts. */
  clear(): void {
    for (const toast of [...this.items]) {
      this.dismiss(toast.id);
    }
  }
}

/** The application-wide toast store. */
export const toasts = new ToastStore();
