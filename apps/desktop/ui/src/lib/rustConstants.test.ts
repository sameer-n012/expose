/**
 * The UI keeps copies of some Rust constants (limits and slider ranges). These
 * tests read the Rust sources and compare them with the copies, so a change
 * on one side only fails a test.
 *
 * Not checked here (no plain constant in Rust): the identity values in
 * `DEFAULTS`, `SHARPEN_DEFAULTS`, and `CROP_DEFAULTS` (they come from `Default`
 * impls), and the slider `step` values (UI only).
 */
import { describe, expect, it } from 'vitest';
import { rustConst, rustRange } from '../test/rustConst';
import { CROP_ANGLE_RANGE } from './develop/crop';
import { MAX_CURVE_POINTS } from './develop/curve';
import { MAX_TARGET_LONG_EDGE, MIN_TARGET_LONG_EDGE } from './develop/size';
import { RANGES, SHARPEN_RANGES } from './develop/settings';
import { MAX_SIZE, MAX_SPACING, MIN_SIZE, MIN_SPACING } from './editor/brush';
import { MAX_FLOOD_TOLERANCE } from './editor/flood';
import { GAMMA_MAX, GAMMA_MIN } from './editor/levels';
import { MAX_DIMENSION, MAX_NAME_LENGTH, MAX_PIXELS, MAX_RESOLUTION_PPI } from './editor/newDocument';
import { MAX_FEATHER } from './editor/selection';

const DOCUMENT = 'crates/expose-editor/src/document.rs';
const ADJUST = 'crates/expose-editor/src/adjust.rs';
const BRUSH = 'crates/expose-editor/src/brush.rs';
const SELECTION = 'crates/expose-editor/src/selection.rs';
const DEVELOP = 'crates/expose-core/src/develop.rs';
const DTO = 'apps/desktop/src-tauri/src/editor/dto.rs';
const PREVIEW = 'apps/desktop/src-tauri/src/preview/engine.rs';

describe('the parser', () => {
  it('fails on a missing constant', () => {
    expect(() => rustConst(DOCUMENT, 'NO_SUCH_CONSTANT')).toThrow(/not found/);
    expect(() => rustRange(DEVELOP, 'NO_SUCH_RANGE')).toThrow(/not found/);
  });
});

describe('new document limits', () => {
  it('match crates/expose-editor/src/document.rs', () => {
    expect(MAX_DIMENSION).toBe(rustConst(DOCUMENT, 'MAX_DIMENSION'));
    expect(MAX_PIXELS).toBe(rustConst(DOCUMENT, 'MAX_PIXELS'));
    expect(MAX_RESOLUTION_PPI).toBe(rustConst(DOCUMENT, 'MAX_RESOLUTION_PPI'));
    // `validateSpec` writes the lower limit as the literal 1.
    expect(rustConst(DOCUMENT, 'MIN_RESOLUTION_PPI')).toBe(1);
  });

  it('match the name limit of the shell', () => {
    expect(MAX_NAME_LENGTH).toBe(rustConst(DTO, 'MAX_NAME_CHARS'));
  });
});

describe('levels limits', () => {
  it('match crates/expose-editor/src/adjust.rs', () => {
    expect(GAMMA_MIN).toBe(rustConst(ADJUST, 'LEVELS_GAMMA_MIN'));
    expect(GAMMA_MAX).toBe(rustConst(ADJUST, 'LEVELS_GAMMA_MAX'));
  });
});

describe('brush and selection limits', () => {
  it('match crates/expose-editor', () => {
    expect(MIN_SIZE).toBe(rustConst(BRUSH, 'MIN_BRUSH_SIZE'));
    expect(MAX_SIZE).toBe(rustConst(BRUSH, 'MAX_BRUSH_SIZE'));
    expect(MIN_SPACING).toBe(rustConst(BRUSH, 'MIN_BRUSH_SPACING'));
    expect(MAX_SPACING).toBe(rustConst(BRUSH, 'MAX_BRUSH_SPACING'));
    expect(MAX_FEATHER).toBe(rustConst(SELECTION, 'MAX_FEATHER_RADIUS'));
    expect(MAX_FLOOD_TOLERANCE).toBe(rustConst('crates/expose-editor/src/flood.rs', 'MAX_FLOOD_TOLERANCE'));
  });
});

describe('develop slider ranges', () => {
  const same = (ui: { min: number; max: number }, name: string): void => {
    const rust = rustRange(DEVELOP, name);
    expect({ min: ui.min, max: ui.max }).toEqual(rust);
  };

  it('match expose_core::develop::ranges', () => {
    same(RANGES.temperature, 'TEMPERATURE');
    same(RANGES.tint, 'TINT');
    same(RANGES.exposure, 'EXPOSURE');
    same(RANGES.tone, 'TONE');
    same(CROP_ANGLE_RANGE, 'CROP_ANGLE');
    same(SHARPEN_RANGES.amount, 'SHARPEN_AMOUNT');
    same(SHARPEN_RANGES.radius, 'SHARPEN_RADIUS');
    same(SHARPEN_RANGES.detail, 'SHARPEN_DETAIL');
    same(SHARPEN_RANGES.masking, 'SHARPEN_MASKING');
  });

  it('match the curve point limit', () => {
    expect(MAX_CURVE_POINTS).toBe(rustConst(DEVELOP, 'MAX_CURVE_POINTS'));
  });
});

describe('preview target size', () => {
  it('matches the shell', () => {
    expect(MIN_TARGET_LONG_EDGE).toBe(rustConst(PREVIEW, 'MIN_LONG_EDGE'));
    expect(MAX_TARGET_LONG_EDGE).toBe(rustConst(PREVIEW, 'MAX_LONG_EDGE'));
  });
});
