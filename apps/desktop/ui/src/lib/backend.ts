/**
 * Typed wrappers for the Tauri commands of the Rust backend.
 *
 * All calls to `invoke` go through this module, so that command names and
 * result types are defined in one place.
 */
import { invoke } from '@tauri-apps/api/core';
import { toasts } from './stores/toasts.svelte';

/**
 * Runs a backend call. If the call fails, shows an error toast with the
 * description and the error text, and returns `undefined`.
 *
 * Use this in UI code for every backend call whose failure the UI does not
 * handle itself. Thus no backend error is lost.
 *
 * @param description What the call does, for the toast ("Load the catalog").
 */
export async function callBackend<T>(description: string, call: () => Promise<T>): Promise<T | undefined> {
  try {
    return await call();
  } catch (error: unknown) {
    toasts.error(`${description} failed.`, error);
    return undefined;
  }
}

/** Returns the application version from the Rust backend. */
export function appVersion(): Promise<string> {
  return invoke<string>('app_version');
}

/** Returns true when the UI runs inside the Tauri shell (not a plain browser). */
export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}
