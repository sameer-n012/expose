import { describe, expect, it } from 'vitest';
import {
  GAMMA_MAX,
  GAMMA_MIN,
  identityChannel,
  isIdentityChannel,
  MIN_INPUT_SPAN,
  setChannelField,
  withChannel,
  type LevelsAdjustment,
} from './levels';

describe('setChannelField', () => {
  it('keeps the input black below the input white', () => {
    const c = { ...identityChannel(), inputWhite: 0.5 };
    expect(setChannelField(c, 'inputBlack', 0.9).inputBlack).toBeCloseTo(0.5 - MIN_INPUT_SPAN);
    expect(setChannelField(c, 'inputBlack', -1).inputBlack).toBe(0);
  });

  it('keeps the input white above the input black', () => {
    const c = { ...identityChannel(), inputBlack: 0.5 };
    expect(setChannelField(c, 'inputWhite', 0.1).inputWhite).toBeCloseTo(0.5 + MIN_INPUT_SPAN);
    expect(setChannelField(c, 'inputWhite', 2).inputWhite).toBe(1);
  });

  it('limits gamma to the Rust range', () => {
    expect(setChannelField(identityChannel(), 'gamma', 0).gamma).toBe(GAMMA_MIN);
    expect(setChannelField(identityChannel(), 'gamma', 50).gamma).toBe(GAMMA_MAX);
  });

  it('allows inverted output points and limits them to 0..1', () => {
    const c = setChannelField(identityChannel(), 'outputWhite', -0.5);
    expect(c.outputWhite).toBe(0);
    expect(c.outputBlack).toBe(0);
    expect(setChannelField(identityChannel(), 'outputBlack', 3).outputBlack).toBe(1);
  });

  it('ignores a value that is not finite', () => {
    expect(setChannelField(identityChannel(), 'gamma', Number.NaN)).toEqual(identityChannel());
  });
});

describe('helpers', () => {
  it('detects the identity channel', () => {
    expect(isIdentityChannel(identityChannel())).toBe(true);
    expect(isIdentityChannel({ ...identityChannel(), gamma: 1.1 })).toBe(false);
  });

  it('replaces only one channel', () => {
    const base: LevelsAdjustment = {
      type: 'levels',
      master: identityChannel(),
      red: identityChannel(),
      green: identityChannel(),
      blue: identityChannel(),
    };
    const next = withChannel(base, 'red', { ...identityChannel(), gamma: 2 });
    expect(next.red.gamma).toBe(2);
    expect(next.master).toEqual(identityChannel());
    expect(base.red.gamma).toBe(1);
  });
});
