/**
 * "Reset all" of the Develop module: every section of the settings
 * document goes back to its default in one document.
 */
import { CROP_DEFAULTS, cropOf, withCrop } from './crop';
import { CURVE_CHANNELS, identityCurve } from './curve';
import {
  curveOf,
  resetBasic,
  sameDocument,
  sharpeningOf,
  SHARPEN_DEFAULTS,
  SHARPEN_KEYS,
  type DevelopDocument,
} from './settings';

/**
 * True when every section equals its default. A missing section reads as
 * its default, so an old document without `tone_curve`, `sharpening`, or
 * `crop` can be at defaults.
 */
export function isAtDefaults(doc: DevelopDocument): boolean {
  const sharpening = sharpeningOf(doc);
  return (
    sameDocument(resetBasic(doc), doc) &&
    CURVE_CHANNELS.every((channel) => JSON.stringify(curveOf(doc, channel)) === JSON.stringify(identityCurve())) &&
    SHARPEN_KEYS.every((key) => sharpening[key] === SHARPEN_DEFAULTS[key]) &&
    JSON.stringify(cropOf(doc)) === JSON.stringify(CROP_DEFAULTS)
  );
}

/**
 * Returns a copy of `doc` with white balance, tone, the four curve
 * channels, sharpening, and crop at their defaults. Fields that the UI
 * does not know stay. A document that is already at defaults is returned
 * as it is.
 */
export function resetEverything(doc: DevelopDocument): DevelopDocument {
  if (isAtDefaults(doc)) {
    return doc;
  }
  const basic = resetBasic(doc);
  return withCrop(
    {
      ...basic,
      tone_curve: {
        master: identityCurve(),
        red: identityCurve(),
        green: identityCurve(),
        blue: identityCurve(),
      },
      sharpening: { ...SHARPEN_DEFAULTS },
    },
    CROP_DEFAULTS,
  );
}
