/**
 * Levels rules without the DOM.
 *
 * The limits are equal to `LevelsChannel::validate` in
 * `crates/expose-editor/src/adjust.rs`:
 * - `inputBlack`, `inputWhite`, `outputBlack`, `outputWhite` are in `0..=1`.
 * - `inputWhite` is more than `inputBlack`.
 * - `gamma` is in `0.01..=10`.
 * The output points can be in any order (this inverts the channel).
 */
import type { AdjustmentDto, LevelsChannelDto } from './api';

export type LevelsAdjustment = Extract<AdjustmentDto, { type: 'levels' }>;
export const LEVELS_CHANNELS = ['master', 'red', 'green', 'blue'] as const;
export type LevelsChannelName = (typeof LEVELS_CHANNELS)[number];
export type LevelsField = keyof LevelsChannelDto;

/** Equal to `LEVELS_GAMMA_MIN` and `LEVELS_GAMMA_MAX` in `adjust.rs`. */
export const GAMMA_MIN = 0.01;
export const GAMMA_MAX = 10;
/** The least distance between the input points: one step of an 8-bit value. */
export const MIN_INPUT_SPAN = 1 / 255;

export function identityChannel(): LevelsChannelDto {
  return { inputBlack: 0, inputWhite: 1, gamma: 1, outputBlack: 0, outputWhite: 1 };
}

export function isIdentityChannel(c: LevelsChannelDto): boolean {
  return c.inputBlack === 0 && c.inputWhite === 1 && c.gamma === 1 && c.outputBlack === 0 && c.outputWhite === 1;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/**
 * Returns the channel with `field` set to `value`. The value is clamped so
 * that the channel stays valid. A value that is not finite leaves the channel
 * as it is.
 */
export function setChannelField(channel: LevelsChannelDto, field: LevelsField, value: number): LevelsChannelDto {
  if (!Number.isFinite(value)) {
    return { ...channel };
  }
  switch (field) {
    case 'inputBlack':
      return { ...channel, inputBlack: clamp(value, 0, channel.inputWhite - MIN_INPUT_SPAN) };
    case 'inputWhite':
      return { ...channel, inputWhite: clamp(value, channel.inputBlack + MIN_INPUT_SPAN, 1) };
    case 'gamma':
      return { ...channel, gamma: clamp(value, GAMMA_MIN, GAMMA_MAX) };
    case 'outputBlack':
      return { ...channel, outputBlack: clamp(value, 0, 1) };
    case 'outputWhite':
      return { ...channel, outputWhite: clamp(value, 0, 1) };
  }
}

/** Returns the adjustment with one channel replaced. The other channels stay. */
export function withChannel(adjustment: LevelsAdjustment, name: LevelsChannelName, channel: LevelsChannelDto): LevelsAdjustment {
  return { ...adjustment, [name]: channel };
}
