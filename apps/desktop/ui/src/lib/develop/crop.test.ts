import { describe, expect, it } from 'vitest';
import {
  ASPECT_PRESETS,
  CROP_DEFAULTS,
  aspectFromId,
  aspectId,
  aspectRatio,
  cropOf,
  flipCrop,
  integerRatio,
  isIdentityCrop,
  resetCrop,
  setCropAngle,
  setCropAspect,
  setCropLock,
  setCropRect,
  swapCropOrientation,
  withCrop,
  withoutCrop,
  type CropSettings,
} from './crop';
import { maxInscribedRect, pixelAspect, rectIsInside } from './cropGeometry';
import type { DevelopDocument } from './settings';

const DOC: DevelopDocument = {
  version: 1,
  white_balance: { mode: 'as_shot', temperature: 5500, tint: 0 },
  tone: { exposure: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0 },
};

const SIZE = { width: 300, height: 200 };

describe('crop document', () => {
  it('reads defaults for a missing or partial field', () => {
    expect(cropOf(DOC)).toEqual(CROP_DEFAULTS);
    const partial = { ...DOC, crop: { angle: 3, rect: { left: 0.1 }, aspect: { ratio: { width: 3, height: 2 } } } };
    expect(cropOf(partial)).toEqual({
      ...CROP_DEFAULTS,
      angle: 3,
      rect: { left: 0.1, top: 0, width: 1, height: 1 },
      aspect: { ratio: { width: 3, height: 2 } },
    });
    expect(cropOf({ ...DOC, crop: { aspect: { ratio: { width: 0, height: 2 } } } }).aspect).toBe('original');
    expect(cropOf({ ...DOC, crop: { aspect: 'free' } }).aspect).toBe('free');
  });

  it('writes the serde shape of the Rust type', () => {
    const crop: CropSettings = { ...CROP_DEFAULTS, angle: -2.5, aspect: { ratio: { width: 3, height: 2 } } };
    const doc = withCrop(DOC, crop);
    expect(JSON.parse(JSON.stringify(doc)).crop).toEqual({
      rect: { left: 0, top: 0, width: 1, height: 1 },
      angle: -2.5,
      flip_horizontal: false,
      flip_vertical: false,
      aspect: { ratio: { width: 3, height: 2 } },
    });
    expect(doc.tone).toBe(DOC.tone);
  });

  it('removes the crop for the crop mode preview', () => {
    expect(withoutCrop(DOC)).toBe(DOC);
    const doc = withCrop(DOC, { ...CROP_DEFAULTS, angle: 4 });
    expect(withoutCrop(doc)).toEqual(DOC);
  });

  it('knows the identity crop', () => {
    expect(isIdentityCrop(CROP_DEFAULTS)).toBe(true);
    expect(isIdentityCrop({ ...CROP_DEFAULTS, aspect: 'free' })).toBe(true);
    expect(isIdentityCrop({ ...CROP_DEFAULTS, flip_vertical: true })).toBe(false);
    expect(resetCrop()).toEqual(CROP_DEFAULTS);
  });
});

describe('aspects', () => {
  it('has the presets in both orientations', () => {
    const ids = ASPECT_PRESETS.map((p) => p.id);
    expect(ids).toEqual([
      'free',
      'original',
      '1:1',
      '4:5',
      '5:4',
      '5:7',
      '7:5',
      '2:3',
      '3:2',
      '3:4',
      '4:3',
      '9:16',
      '16:9',
    ]);
    for (const preset of ASPECT_PRESETS) {
      expect(aspectFromId(preset.id)).toEqual(preset.aspect);
      expect(aspectId(preset.aspect)).toBe(preset.id);
    }
    expect(aspectFromId('0:3')).toBeNull();
    expect(aspectFromId('wide')).toBeNull();
  });

  it('gives the pixel ratio', () => {
    expect(aspectRatio('free', SIZE)).toBeNull();
    expect(aspectRatio('original', SIZE)).toBe(1.5);
    expect(aspectRatio({ ratio: { width: 4, height: 5 } }, SIZE)).toBe(0.8);
    expect(integerRatio(3000, 2000)).toEqual({ ratio: { width: 3, height: 2 } });
    expect(integerRatio(0.2, 7)).toEqual({ ratio: { width: 1, height: 7 } });
  });
});

describe('crop edits', () => {
  it('an angle change keeps the rect inside and clamps the angle', () => {
    const c = setCropAngle(CROP_DEFAULTS, SIZE, 10);
    expect(c.angle).toBe(10);
    expect(c.rect).toEqual(maxInscribedRect(300, 200, 10, 1.5));
    expect(setCropAngle(CROP_DEFAULTS, SIZE, 90).angle).toBe(45);
    expect(setCropAngle(CROP_DEFAULTS, SIZE, Number.NaN)).toBe(CROP_DEFAULTS);
  });

  it('an aspect change gives the largest rect of the ratio', () => {
    const c = setCropAspect(CROP_DEFAULTS, SIZE, { ratio: { width: 1, height: 1 } });
    expect(pixelAspect(SIZE, c.rect)).toBeCloseTo(1, 9);
    expect(c.rect.height).toBeCloseTo(1, 9);
    const free = setCropAspect(c, SIZE, 'free');
    expect(free.rect).toBe(c.rect);
  });

  it('the lock keeps the shape of the rect', () => {
    const free = { ...CROP_DEFAULTS, aspect: 'free' as const, rect: { left: 0, top: 0, width: 0.5, height: 1 } };
    expect(setCropLock(free, SIZE, true).aspect).toEqual({ ratio: { width: 3, height: 4 } });
    expect(setCropLock(CROP_DEFAULTS, SIZE, false).aspect).toBe('free');
    expect(setCropLock(CROP_DEFAULTS, SIZE, true)).toBe(CROP_DEFAULTS);
  });

  it('swaps the orientation of the aspect and the rect', () => {
    const c = setCropAspect(CROP_DEFAULTS, SIZE, { ratio: { width: 3, height: 2 } });
    const swapped = swapCropOrientation(c, SIZE);
    expect(swapped.aspect).toEqual({ ratio: { width: 2, height: 3 } });
    expect(pixelAspect(SIZE, swapped.rect)).toBeCloseTo(2 / 3, 9);
    expect(rectIsInside(300, 200, 0, swapped.rect)).toBe(true);
    // Original becomes the swapped integer ratio of the session.
    expect(swapCropOrientation(CROP_DEFAULTS, SIZE).aspect).toEqual({ ratio: { width: 2, height: 3 } });
    // Swapped back at an angle: the ratio is kept.
    const tilted = swapCropOrientation(setCropAngle(swapped, SIZE, 12), SIZE);
    expect(pixelAspect(SIZE, tilted.rect)).toBeCloseTo(1.5, 6);
    expect(rectIsInside(300, 200, 12, tilted.rect)).toBe(true);
  });

  it('a flip mirrors the result: flag, angle sign, and rect', () => {
    const c: CropSettings = { ...CROP_DEFAULTS, angle: 5, rect: { left: 0.1, top: 0.2, width: 0.3, height: 0.4 } };
    const h = flipCrop(c, 'horizontal');
    expect(h.flip_horizontal).toBe(true);
    expect(h.angle).toBe(-5);
    expect(h.rect.left).toBeCloseTo(0.6, 12);
    const v = flipCrop(CROP_DEFAULTS, 'vertical');
    expect(v.flip_vertical).toBe(true);
    expect(Object.is(v.angle, 0)).toBe(true);
    expect(flipCrop(h, 'horizontal').flip_horizontal).toBe(false);
  });

  it('a rect is fitted into the rotated image', () => {
    const c = { ...CROP_DEFAULTS, angle: 20 };
    const out = setCropRect(c, SIZE, { left: 0, top: 0, width: 1, height: 1 });
    expect(rectIsInside(300, 200, 20, out.rect)).toBe(true);
  });
});
