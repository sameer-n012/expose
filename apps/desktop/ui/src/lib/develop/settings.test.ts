import { describe, expect, it } from 'vitest';
import {
  resetSharpenKey,
  resetSharpening,
  sharpeningOf,
  withSharpening,
  type DevelopDocument,
} from './settings';

const OLD: DevelopDocument = {
  version: 1,
  white_balance: { mode: 'as_shot', temperature: 5500, tint: 0 },
  tone: { exposure: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0 },
  crop: { keep: 'me' },
};

describe('sharpening settings', () => {
  it('reads the defaults when the field is missing', () => {
    expect(sharpeningOf(OLD)).toEqual({ amount: 0, radius: 1, detail: 25, masking: 0 });
  });

  it('fills missing values and keeps other fields', () => {
    const next = withSharpening({ ...OLD, sharpening: { amount: 10 } }, 'masking', 30);
    expect(next.sharpening).toEqual({ amount: 10, radius: 1, detail: 25, masking: 30 });
    expect(next.crop).toEqual({ keep: 'me' });
  });

  it('resets one value or all values', () => {
    const edited = withSharpening(withSharpening(OLD, 'amount', 90), 'radius', 2);
    expect(resetSharpenKey(edited, 'amount').sharpening).toEqual({ amount: 0, radius: 2, detail: 25, masking: 0 });
    expect(resetSharpening(edited).sharpening).toEqual({ amount: 0, radius: 1, detail: 25, masking: 0 });
  });

  it('returns the same document when there is nothing to reset', () => {
    expect(resetSharpening(OLD)).toBe(OLD);
    expect(resetSharpenKey(OLD, 'detail')).toBe(OLD);
  });
});
