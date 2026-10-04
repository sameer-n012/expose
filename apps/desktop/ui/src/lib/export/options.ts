/**
 * The export form model, its defaults, the conversion to a request, and the
 * storage of the last used options.
 *
 * The form model is flat. Thus a value (for example the JPEG quality) stays
 * when the user changes the format and changes it back.
 */
import type { PhotoId } from '../catalog';
import type {
  BitDepth,
  ExportColorSpace,
  ExportConflict,
  ExportFormat,
  ExportRequest,
  ExportSize,
  JpegSubsampling,
  TiffCompression,
} from './types';

export type FormatKind = ExportFormat['kind'];
export type SizeMode = ExportSize['mode'];
export type NamingMode = 'original' | 'suffix';

export interface ExportForm {
  formatKind: FormatKind;
  jpegQuality: number;
  jpegSubsampling: JpegSubsampling;
  pngBitDepth: BitDepth;
  tiffBitDepth: BitDepth;
  tiffCompression: TiffCompression;
  colorSpace: ExportColorSpace;
  sizeMode: SizeMode;
  /** The number for the size mode: pixels, megapixels, or percent. Not used for "original". */
  sizeValue: number;
  allowUpscale: boolean;
  /** Absolute folder path. Empty until the user picks a folder. */
  destination: string;
  namingMode: NamingMode;
  suffix: string;
  onConflict: ExportConflict;
  dither: boolean;
}

/** The value that a size mode has when the user selects it. */
export const SIZE_MODE_DEFAULTS: Readonly<Record<SizeMode, number>> = {
  original: 0,
  longEdge: 2048,
  shortEdge: 1080,
  width: 2048,
  height: 2048,
  megapixels: 8,
  percent: 50,
};

/**
 * Defaults: JPEG quality 85 with 4:2:0 subsampling, sRGB, original size, the
 * original file name, and a unique name on conflict (never overwrites).
 */
export function defaultForm(): ExportForm {
  return {
    formatKind: 'jpeg',
    jpegQuality: 85,
    jpegSubsampling: '420',
    pngBitDepth: 8,
    tiffBitDepth: 16,
    tiffCompression: 'lzw',
    colorSpace: 'srgb',
    sizeMode: 'original',
    sizeValue: SIZE_MODE_DEFAULTS.longEdge,
    allowUpscale: false,
    destination: '',
    namingMode: 'original',
    suffix: '-export',
    onConflict: 'uniqueName',
    dither: true,
  };
}

/** The bit depth of the selected format. JPEG output has 8 bits. */
export function bitDepthOf(form: ExportForm): BitDepth {
  switch (form.formatKind) {
    case 'jpeg':
      return 8;
    case 'png':
      return form.pngBitDepth;
    case 'tiff':
      return form.tiffBitDepth;
  }
}

export function formatOf(form: ExportForm): ExportFormat {
  switch (form.formatKind) {
    case 'jpeg':
      return { kind: 'jpeg', quality: form.jpegQuality, subsampling: form.jpegSubsampling };
    case 'png':
      return { kind: 'png', bitDepth: form.pngBitDepth };
    case 'tiff':
      return { kind: 'tiff', bitDepth: form.tiffBitDepth, compression: form.tiffCompression };
  }
}

export function sizeOf(form: ExportForm): ExportSize {
  switch (form.sizeMode) {
    case 'original':
      return { mode: 'original' };
    case 'megapixels':
      return { mode: 'megapixels', megapixels: form.sizeValue };
    case 'percent':
      return { mode: 'percent', percent: form.sizeValue };
    default:
      return { mode: form.sizeMode, pixels: form.sizeValue };
  }
}

/** Builds the request. It does not validate. Use `validateForm` first. */
export function buildRequest(form: ExportForm, photoIds: readonly PhotoId[]): ExportRequest {
  return {
    photoIds: [...photoIds],
    destination: form.destination,
    format: formatOf(form),
    colorSpace: form.colorSpace,
    size: sizeOf(form),
    allowUpscale: form.allowUpscale,
    naming: form.namingMode === 'original' ? { mode: 'original' } : { mode: 'suffix', suffix: form.suffix },
    onConflict: form.onConflict,
    // The backend ignores dither for 16-bit output. Send false to be exact.
    dither: bitDepthOf(form) === 8 && form.dither,
  };
}

/** The key of the stored options in `localStorage`. */
export const STORAGE_KEY = 'expose.export.options.v1';

const FORMAT_KINDS: readonly string[] = ['jpeg', 'png', 'tiff'];
const SUBSAMPLINGS: readonly string[] = ['444', '422', '420'];
const COMPRESSIONS: readonly string[] = ['none', 'lzw', 'deflate'];
const COLOR_SPACES: readonly string[] = ['srgb', 'display_p3', 'oprgb', 'prophoto_rgb', 'rec2020'];
const SIZE_MODES: readonly string[] = Object.keys(SIZE_MODE_DEFAULTS);
const NAMING_MODES: readonly string[] = ['original', 'suffix'];
const CONFLICTS: readonly string[] = ['uniqueName', 'overwrite', 'skip'];

/**
 * Takes the valid fields of an unknown value and fills the other fields with
 * defaults. Returns null if the value is not an object.
 */
export function parseForm(value: unknown): ExportForm | null {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    return null;
  }
  const raw = value as Record<string, unknown>;
  const form = defaultForm();
  const pick = <K extends keyof ExportForm>(key: K, ok: (v: unknown) => boolean): void => {
    if (key in raw && ok(raw[key])) {
      form[key] = raw[key] as ExportForm[K];
    }
  };
  const oneOf = (list: readonly string[]) => (v: unknown) => typeof v === 'string' && list.includes(v);
  const depth = (v: unknown): boolean => v === 8 || v === 16;
  const finite = (v: unknown): boolean => typeof v === 'number' && Number.isFinite(v);
  pick('formatKind', oneOf(FORMAT_KINDS));
  pick('jpegQuality', finite);
  pick('jpegSubsampling', oneOf(SUBSAMPLINGS));
  pick('pngBitDepth', depth);
  pick('tiffBitDepth', depth);
  pick('tiffCompression', oneOf(COMPRESSIONS));
  pick('colorSpace', oneOf(COLOR_SPACES));
  pick('sizeMode', oneOf(SIZE_MODES));
  pick('sizeValue', finite);
  pick('allowUpscale', (v) => typeof v === 'boolean');
  pick('destination', (v) => typeof v === 'string');
  pick('namingMode', oneOf(NAMING_MODES));
  pick('suffix', (v) => typeof v === 'string');
  pick('onConflict', oneOf(CONFLICTS));
  pick('dither', (v) => typeof v === 'boolean');
  return form;
}

/**
 * Loads the last used options. If storage is missing, or the stored value is
 * bad, it warns in the console and returns the defaults. It never throws.
 */
export function loadForm(storage: Pick<Storage, 'getItem'> | null = defaultStorage()): ExportForm {
  if (storage === null) {
    return defaultForm();
  }
  try {
    const text = storage.getItem(STORAGE_KEY);
    if (text === null) {
      return defaultForm();
    }
    const form = parseForm(JSON.parse(text));
    if (form === null) {
      console.warn('The stored export options are not an object. The defaults are used.');
      return defaultForm();
    }
    return form;
  } catch (error: unknown) {
    console.warn('The stored export options cannot be read. The defaults are used.', error);
    return defaultForm();
  }
}

/** Stores the options. A storage error only gives a console warning. */
export function saveForm(form: ExportForm, storage: Pick<Storage, 'setItem'> | null = defaultStorage()): void {
  if (storage === null) {
    return;
  }
  try {
    storage.setItem(STORAGE_KEY, JSON.stringify(form));
  } catch (error: unknown) {
    console.warn('The export options cannot be stored.', error);
  }
}

/** The browser storage, or null if it is missing or access is denied. */
function defaultStorage(): Storage | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch (error: unknown) {
    // Access to `localStorage` can throw (for example when storage is blocked).
    console.warn('The browser storage is not available.', error);
    return null;
  }
}
