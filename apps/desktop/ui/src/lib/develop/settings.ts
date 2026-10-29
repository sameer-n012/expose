/**
 * The develop settings document, as the Rust shell sends it and stores it.
 *
 * The types mirror `expose_core::develop::DevelopSettings` (serde, snake_case
 * names). The UI edits only the fields that it knows. Each edit copies the
 * document and changes one value, so all other fields (also fields that
 * this UI does not know) go back to the shell unchanged.
 */
import { identityCurve, type CurveChannel, type CurvePoint, type ToneCurveSettings } from './curve';

/** The ranges of the sliders. Keep them equal to `expose_core::develop::ranges`. `lib/rustConstants.test.ts` checks min and max. */
export const RANGES = {
  temperature: { min: 2000, max: 25000, step: 50 },
  tint: { min: -150, max: 150, step: 1 },
  exposure: { min: -5, max: 5, step: 0.01 },
  tone: { min: -100, max: 100, step: 1 },
} as const;

/**
 * The ranges of the sharpening sliders. Keep them equal to the constants
 * `ranges::SHARPEN_*` in `crates/expose-core/src/develop.rs`.
 * `lib/rustConstants.test.ts` checks min and max.
 */
export const SHARPEN_RANGES = {
  amount: { min: 0, max: 150, step: 1 },
  radius: { min: 0.5, max: 3, step: 0.1 },
  detail: { min: 0, max: 100, step: 1 },
  masking: { min: 0, max: 100, step: 1 },
} as const;

export const SHARPEN_KEYS = ['amount', 'radius', 'detail', 'masking'] as const;
export type SharpenKey = (typeof SHARPEN_KEYS)[number];

/** The identity values of sharpening. Keep them equal to `Default` of `Sharpening` in `expose_core::develop`. */
export const SHARPEN_DEFAULTS: Readonly<Record<SharpenKey, number>> = {
  amount: 0,
  radius: 1,
  detail: 25,
  masking: 0,
};

/** The sharpening settings (`sharpening` field of the document). */
export type SharpeningSettings = Readonly<Record<SharpenKey, number>>;

export const TONE_KEYS = ['contrast', 'highlights', 'shadows', 'whites', 'blacks'] as const;
export type ToneKey = (typeof TONE_KEYS)[number];

/** `as_shot`, `custom`, or a preset object (`{ preset: ... }`). */
export type WhiteBalanceMode = 'as_shot' | 'custom' | { readonly preset: unknown };

export interface WhiteBalanceSettings {
  readonly mode: WhiteBalanceMode;
  readonly temperature: number;
  readonly tint: number;
  readonly [key: string]: unknown;
}

export interface ToneSettings {
  readonly exposure: number;
  readonly contrast: number;
  readonly highlights: number;
  readonly shadows: number;
  readonly whites: number;
  readonly blacks: number;
  readonly [key: string]: unknown;
}

/** The settings document. Other fields (crop, ...) stay as they are. */
export interface DevelopDocument {
  readonly version: number;
  readonly white_balance: WhiteBalanceSettings;
  readonly tone: ToneSettings;
  /** Missing in old documents: every channel is then the identity curve. */
  readonly tone_curve?: Partial<ToneCurveSettings>;
  /** Missing in old documents: every value is then its default. */
  readonly sharpening?: Partial<SharpeningSettings>;
  readonly [key: string]: unknown;
}

/** The sharpening values of a document (defaults for missing values). */
export function sharpeningOf(doc: DevelopDocument): SharpeningSettings {
  return { ...SHARPEN_DEFAULTS, ...doc.sharpening };
}

/** Returns a copy of `doc` with one sharpening value changed. All four values are written. */
export function withSharpening(doc: DevelopDocument, key: SharpenKey, value: number): DevelopDocument {
  return { ...doc, sharpening: { ...sharpeningOf(doc), [key]: value } };
}

/** Returns a copy of `doc` with one sharpening value at its default. */
export function resetSharpenKey(doc: DevelopDocument, key: SharpenKey): DevelopDocument {
  if (sharpeningOf(doc)[key] === SHARPEN_DEFAULTS[key]) {
    return doc;
  }
  return withSharpening(doc, key, SHARPEN_DEFAULTS[key]);
}

/** Returns a copy of `doc` with all sharpening values at their defaults. */
export function resetSharpening(doc: DevelopDocument): DevelopDocument {
  const current = sharpeningOf(doc);
  if (SHARPEN_KEYS.every((key) => current[key] === SHARPEN_DEFAULTS[key])) {
    // Nothing to reset: an old document without the field stays as it is.
    return doc;
  }
  return { ...doc, sharpening: { ...SHARPEN_DEFAULTS } };
}

/** The points of one curve channel of a document (identity when missing). */
export function curveOf(doc: DevelopDocument, channel: CurveChannel): readonly CurvePoint[] {
  return doc.tone_curve?.[channel] ?? identityCurve();
}

/** Returns a copy of `doc` with one curve channel changed. All four channels are written. */
export function withCurve(
  doc: DevelopDocument,
  channel: CurveChannel,
  points: readonly CurvePoint[],
): DevelopDocument {
  const full: Record<CurveChannel, readonly CurvePoint[]> = {
    master: curveOf(doc, 'master'),
    red: curveOf(doc, 'red'),
    green: curveOf(doc, 'green'),
    blue: curveOf(doc, 'blue'),
  };
  full[channel] = points;
  return { ...doc, tone_curve: { ...doc.tone_curve, ...full } };
}

/** A slider that the Basic panel edits. */
export type SliderKey = 'temperature' | 'tint' | 'exposure' | ToneKey;

/** The value of a slider in a document. */
export function sliderValue(doc: DevelopDocument, key: SliderKey): number {
  if (key === 'temperature' || key === 'tint') {
    return doc.white_balance[key];
  }
  return doc.tone[key];
}

/** Returns a copy of `doc` with one slider changed. A white balance slider sets the mode to custom. */
export function withSlider(doc: DevelopDocument, key: SliderKey, value: number): DevelopDocument {
  if (key === 'temperature' || key === 'tint') {
    return { ...doc, white_balance: { ...doc.white_balance, mode: 'custom', [key]: value } };
  }
  return { ...doc, tone: { ...doc.tone, [key]: value } };
}

/** Returns a copy of `doc` with the white balance mode changed. */
export function withWhiteBalanceMode(
  doc: DevelopDocument,
  mode: 'as_shot' | 'custom',
): DevelopDocument {
  return { ...doc, white_balance: { ...doc.white_balance, mode } };
}

/** The mode name for the UI: `preset` for every preset. */
export function modeName(doc: DevelopDocument): 'as_shot' | 'custom' | 'preset' {
  const mode = doc.white_balance.mode;
  return typeof mode === 'string' ? mode : 'preset';
}

/** The identity values of the sliders. Keep them equal to `Default` in `expose_core::develop`. */
export const DEFAULTS: Readonly<Record<SliderKey, number>> = {
  temperature: 5500,
  tint: 0,
  exposure: 0,
  contrast: 0,
  highlights: 0,
  shadows: 0,
  whites: 0,
  blacks: 0,
};

/**
 * Returns a copy of `doc` with one slider at its identity value. Resetting
 * a white balance slider does not change the mode.
 */
export function resetSlider(doc: DevelopDocument, key: SliderKey): DevelopDocument {
  if (key === 'temperature' || key === 'tint') {
    return { ...doc, white_balance: { ...doc.white_balance, [key]: DEFAULTS[key] } };
  }
  return { ...doc, tone: { ...doc.tone, [key]: DEFAULTS[key] } };
}

/**
 * Returns a copy of `doc` with all Basic panel values at identity (white
 * balance as shot). Other fields stay.
 */
export function resetBasic(doc: DevelopDocument): DevelopDocument {
  return {
    ...doc,
    white_balance: { ...doc.white_balance, mode: 'as_shot', temperature: DEFAULTS.temperature, tint: DEFAULTS.tint },
    tone: {
      ...doc.tone,
      exposure: DEFAULTS.exposure,
      contrast: DEFAULTS.contrast,
      highlights: DEFAULTS.highlights,
      shadows: DEFAULTS.shadows,
      whites: DEFAULTS.whites,
      blacks: DEFAULTS.blacks,
    },
  };
}

/** Compares two documents by value (JSON form, key order of the shell). */
export function sameDocument(a: DevelopDocument, b: DevelopDocument): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}
