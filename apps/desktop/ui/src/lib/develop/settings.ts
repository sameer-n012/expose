/**
 * The develop settings document, as the Rust shell sends it and stores it.
 *
 * The types mirror `expose_core::develop::DevelopSettings` (serde, snake_case
 * names). The UI edits only the fields that it knows. Each edit copies the
 * document and changes one value, so all other fields (also fields that
 * this UI does not know) go back to the shell unchanged.
 */

/** The ranges of the sliders. Keep them equal to `expose_core::develop::ranges`. */
export const RANGES = {
  temperature: { min: 2000, max: 25000, step: 50 },
  tint: { min: -150, max: 150, step: 1 },
  exposure: { min: -5, max: 5, step: 0.01 },
  tone: { min: -100, max: 100, step: 1 },
} as const;

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

/** The settings document. Other fields (tone curve, crop, ...) stay as they are. */
export interface DevelopDocument {
  readonly version: number;
  readonly white_balance: WhiteBalanceSettings;
  readonly tone: ToneSettings;
  readonly [key: string]: unknown;
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
