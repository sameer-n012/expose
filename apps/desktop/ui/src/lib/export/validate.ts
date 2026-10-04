/**
 * Validation of the export form. The rules are the same as the limits of the
 * backend, so that the dialog shows an error before the user submits.
 */
import type { PhotoId } from '../catalog';
import type { ExportForm } from './options';
import { EXPORT_LIMITS as L } from './types';

export type FormField = 'photos' | 'destination' | 'quality' | 'size' | 'suffix';

export type FormErrors = Partial<Record<FormField, string>>;

/** True for an absolute POSIX path, a drive path, or a UNC path. */
export function isAbsolutePath(path: string): boolean {
  return path.startsWith('/') || /^[A-Za-z]:[\\/]/.test(path) || path.startsWith('\\\\');
}

/** True if the text has a control character (code below 32, or 127). */
function hasControlChar(text: string): boolean {
  for (const ch of text) {
    const code = ch.codePointAt(0) ?? 0;
    if (code < 32 || code === 127) {
      return true;
    }
  }
  return false;
}

function isInt(value: number): boolean {
  return Number.isInteger(value);
}

function validateSize(form: ExportForm): string | undefined {
  const v = form.sizeValue;
  switch (form.sizeMode) {
    case 'original':
      return undefined;
    case 'megapixels':
      return Number.isFinite(v) && v > 0 && v <= L.maxMegapixels
        ? undefined
        : `Megapixels must be more than 0 and at most ${L.maxMegapixels}.`;
    case 'percent':
      if (!isInt(v) || v < L.minPercent || v > L.maxPercent) {
        return `Percent must be a whole number from ${L.minPercent} to ${L.maxPercent}.`;
      }
      return v > 100 && !form.allowUpscale ? 'More than 100 percent needs "Allow upscaling".' : undefined;
    default:
      return isInt(v) && v >= L.minPixels && v <= L.maxPixels
        ? undefined
        : `Pixels must be a whole number from ${L.minPixels} to ${L.maxPixels}.`;
  }
}

/**
 * Checks the form and the photo count.
 * @returns the error text per field. An empty object means: valid.
 */
export function validateForm(form: ExportForm, photoCount: number): FormErrors {
  const errors: FormErrors = {};
  if (photoCount < 1 || photoCount > L.maxPhotos) {
    errors.photos = `Select 1 to ${L.maxPhotos} photos.`;
  }
  if (form.destination.trim() === '') {
    errors.destination = 'Choose a destination folder.';
  } else if (!isAbsolutePath(form.destination)) {
    errors.destination = 'The destination must be an absolute path.';
  }
  if (form.formatKind === 'jpeg') {
    const q = form.jpegQuality;
    if (!isInt(q) || q < L.minQuality || q > L.maxQuality) {
      errors.quality = `Quality must be a whole number from ${L.minQuality} to ${L.maxQuality}.`;
    }
  }
  const size = validateSize(form);
  if (size !== undefined) {
    errors.size = size;
  }
  if (form.namingMode === 'suffix') {
    const s = form.suffix;
    if ([...s].length > L.maxSuffixLength) {
      errors.suffix = `The suffix has at most ${L.maxSuffixLength} characters.`;
    } else if (s.includes('/') || s.includes('\\')) {
      errors.suffix = 'The suffix must not have "/" or "\\".';
    } else if (hasControlChar(s)) {
      errors.suffix = 'The suffix must not have control characters.';
    }
  }
  return errors;
}

/** Convenience: true if the form has no error. */
export function isValid(form: ExportForm, photoIds: readonly PhotoId[]): boolean {
  return Object.keys(validateForm(form, photoIds.length)).length === 0;
}
