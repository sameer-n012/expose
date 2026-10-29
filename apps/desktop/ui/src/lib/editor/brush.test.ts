import { describe, expect, it } from 'vitest';
import {
  MAX_SIZE,
  clampSize,
  defaultBrush,
  hexToRgb,
  luminance,
  normalizeBrush,
  pointerPressure,
  rgbToHex,
  sizeToSlider,
  sliderToSize,
  steppedHardness,
  steppedSize,
  strokeBrush,
  strokeTarget,
} from './brush';

describe('colors', () => {
  it('converts hex to sRGB and back', () => {
    expect(hexToRgb('#000000')).toEqual([0, 0, 0]);
    expect(hexToRgb('#FFffFF')).toEqual([1, 1, 1]);
    expect(hexToRgb('#ff0000')).toEqual([1, 0, 0]);
    expect(rgbToHex([1, 0.5, 0])).toBe('#ff8000');
    expect(rgbToHex(hexToRgb('#12ab9f') ?? [0, 0, 0])).toBe('#12ab9f');
  });

  it('rejects text that is not a 6-digit hex color', () => {
    expect(hexToRgb('red')).toBeNull();
    expect(hexToRgb('#fff')).toBeNull();
    expect(hexToRgb('#gg0000')).toBeNull();
  });

  it('limits values out of range when it makes hex', () => {
    expect(rgbToHex([2, -1, Number.NaN])).toBe('#ff0000');
  });

  it('uses the Rec. 709 weights for the luminance', () => {
    expect(luminance([0, 0, 0])).toBe(0);
    expect(luminance([1, 1, 1])).toBeCloseTo(1, 12);
    expect(luminance([1, 0, 0])).toBeCloseTo(0.2126, 12);
    expect(luminance([0, 1, 0])).toBeCloseTo(0.7152, 12);
    expect(luminance([0, 0, 1])).toBeCloseTo(0.0722, 12);
    expect(luminance([0.5, 0.5, 0.5])).toBeCloseTo(0.5, 12);
  });
});

describe('stroke target', () => {
  it('paints the foreground and erases on pixels', () => {
    expect(strokeTarget('brush', 'pixels', '#ff0000', '#ffffff')).toEqual({ type: 'paint', color: [1, 0, 0] });
    expect(strokeTarget('eraser', 'pixels', '#ff0000', '#ffffff')).toEqual({ type: 'erase' });
  });

  it('uses the luminance of the foreground (brush) or background (eraser) on a mask', () => {
    expect(strokeTarget('brush', 'mask', '#000000', '#ffffff')).toEqual({ type: 'mask', value: 0 });
    const eraser = strokeTarget('eraser', 'mask', '#000000', '#ffffff');
    expect(eraser?.type).toBe('mask');
    expect(eraser?.type === 'mask' ? eraser.value : -1).toBeCloseTo(1, 12);
  });

  it('gives null for a color that is not valid', () => {
    expect(strokeTarget('brush', 'pixels', 'x', '#ffffff')).toBeNull();
  });
});

describe('size', () => {
  it('limits the size', () => {
    expect(clampSize(0)).toBe(1);
    expect(clampSize(99999)).toBe(MAX_SIZE);
    expect(clampSize(Number.NaN)).toBe(1);
  });

  it('steps with a size that grows', () => {
    expect(steppedSize(5, 1)).toBe(6);
    expect(steppedSize(30, 1)).toBe(35);
    expect(steppedSize(100, 1)).toBe(125);
    expect(steppedSize(500, 1)).toBe(550);
    expect(steppedSize(2000, 1)).toBe(2100);
    expect(steppedSize(100, -1)).toBe(90);
  });

  it('keeps up and down inverse at a step border', () => {
    expect(steppedSize(steppedSize(50, 1), -1)).toBe(50);
    expect(steppedSize(steppedSize(50, -1), 1)).toBe(50);
  });

  it('stays in the range', () => {
    expect(steppedSize(1, -1)).toBe(1);
    expect(steppedSize(4990, 1)).toBe(5000);
    expect(steppedSize(5000, 1)).toBe(5000);
  });

  it('maps the logarithmic slider both ways', () => {
    expect(sliderToSize(0)).toBe(1);
    expect(sliderToSize(1)).toBe(5000);
    for (const size of [1, 7, 30, 250, 1200, 5000]) {
      expect(sliderToSize(sizeToSlider(size))).toBe(size);
    }
    expect(sliderToSize(0.5)).toBe(71);
  });
});

describe('hardness and settings', () => {
  it('steps the hardness by 25 percent', () => {
    expect(steppedHardness(0.5, 1)).toBe(0.75);
    expect(steppedHardness(0.8, 1)).toBe(1);
    expect(steppedHardness(0.1, -1)).toBe(0);
  });

  it('limits every value of the settings', () => {
    const out = normalizeBrush({ ...defaultBrush(), size: 0, hardness: 2, opacity: -1, flow: Number.NaN, spacing: 100 });
    expect(out).toMatchObject({ size: 1, hardness: 1, opacity: 0, flow: 0, spacing: 10 });
  });

  it('keeps a valid blend mode and replaces dissolve with normal', () => {
    expect(defaultBrush().blendMode).toBe('normal');
    expect(normalizeBrush({ ...defaultBrush(), blendMode: 'multiply' }).blendMode).toBe('multiply');
    expect(normalizeBrush({ ...defaultBrush(), blendMode: 'dissolve' }).blendMode).toBe('normal');
  });

  it('sends normal for the eraser and for mask paint', () => {
    const brush = { ...defaultBrush(), blendMode: 'screen' as const };
    expect(strokeBrush('brush', 'pixels', brush).blendMode).toBe('screen');
    expect(strokeBrush('brush', 'mask', brush).blendMode).toBe('normal');
    expect(strokeBrush('eraser', 'pixels', brush).blendMode).toBe('normal');
  });
});

describe('pressure', () => {
  it('uses the pen pressure and 1 for other pointers', () => {
    expect(pointerPressure({ pointerType: 'pen', pressure: 0.4 })).toBe(0.4);
    expect(pointerPressure({ pointerType: 'mouse', pressure: 0.5 })).toBe(1);
    expect(pointerPressure({ pointerType: 'touch', pressure: 0 })).toBe(1);
  });
});
