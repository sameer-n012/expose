/**
 * Pure helpers for the "New document" dialog: size presets, defaults, and
 * the validation of a `NewDocumentSpec`.
 */
import type { NewDocumentSpec } from './api';

// The four limits below are copies of Rust constants. `lib/rustConstants.test.ts` checks them.
/** Largest side of a document in pixels. Copy of `MAX_DIMENSION` in crates/expose-editor/src/document.rs. */
export const MAX_DIMENSION = 30000;
/** Largest pixel count of a document. Copy of `MAX_PIXELS` in crates/expose-editor/src/document.rs (1 << 28). */
export const MAX_PIXELS = 2 ** 28;
/** Copy of `MAX_NAME_CHARS` in apps/desktop/src-tauri/src/editor/dto.rs. */
export const MAX_NAME_LENGTH = 120;
/** Copy of `MAX_RESOLUTION_PPI` in crates/expose-editor/src/document.rs. */
export const MAX_RESOLUTION_PPI = 10000;

export interface DocumentPreset {
  readonly id: string;
  readonly label: string;
  readonly width: number;
  readonly height: number;
  readonly resolutionPpi: number;
}

export const DOCUMENT_PRESETS: readonly DocumentPreset[] = [
  { id: 'hd', label: '1920 × 1080', width: 1920, height: 1080, resolutionPpi: 72 },
  { id: 'uhd', label: '3840 × 2160', width: 3840, height: 2160, resolutionPpi: 72 },
  { id: 'a4', label: 'A4 300 ppi', width: 2480, height: 3508, resolutionPpi: 300 },
  { id: 'square', label: 'Square 2048', width: 2048, height: 2048, resolutionPpi: 72 },
];

/** The default spec for a document name. */
export function defaultSpec(name: string): NewDocumentSpec {
  return { name, width: 1920, height: 1080, resolutionPpi: 72, bitDepth: 16, background: 'white' };
}

/** The default name of the n-th new document of a session. */
export function untitledName(n: number): string {
  return `Untitled-${n}`;
}

function validSize(value: number, max: number): boolean {
  return Number.isInteger(value) && value >= 1 && value <= max;
}

/** Returns a message for the first problem of the spec, or null if the spec is valid. */
export function validateSpec(spec: NewDocumentSpec): string | null {
  const name = spec.name;
  if (name.trim() === '') {
    return 'Enter a name.';
  }
  if (name.length > MAX_NAME_LENGTH) {
    return `The name must have at most ${MAX_NAME_LENGTH} characters.`;
  }
  // The name becomes a file name: path separators, ":" and control characters are not allowed.
  if (/[/\\:\u0000-\u001f\u007f]/.test(name)) {
    return 'The name must not have "/", "\\", ":" or control characters.';
  }
  if (!validSize(spec.width, MAX_DIMENSION)) {
    return `The width must be a whole number from 1 to ${MAX_DIMENSION}.`;
  }
  if (!validSize(spec.height, MAX_DIMENSION)) {
    return `The height must be a whole number from 1 to ${MAX_DIMENSION}.`;
  }
  if (spec.width * spec.height > MAX_PIXELS) {
    return `The document must have at most ${MAX_PIXELS.toLocaleString('en-US')} pixels.`;
  }
  if (!Number.isFinite(spec.resolutionPpi) || spec.resolutionPpi < 1 || spec.resolutionPpi > MAX_RESOLUTION_PPI) {
    return `The resolution must be from 1 to ${MAX_RESOLUTION_PPI} ppi.`;
  }
  return null;
}
