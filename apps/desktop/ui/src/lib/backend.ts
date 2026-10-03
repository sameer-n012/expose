/**
 * Typed wrappers for the Tauri commands of the Rust backend.
 *
 * All calls to `invoke` go through this module, so that command names and
 * result types are defined in one place.
 */
import { invoke } from '@tauri-apps/api/core';

/** Returns the application version from the Rust backend. */
export function appVersion(): Promise<string> {
  return invoke<string>('app_version');
}

/** Returns true when the UI runs inside the Tauri shell (not a plain browser). */
export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}
