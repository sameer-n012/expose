import { describe, expect, it } from 'vitest';
import {
  MAX_GRADIENT_STOPS,
  addStop,
  blackToWhite,
  cssGradient,
  defaultGradient,
  foregroundToBackground,
  foregroundToTransparent,
  gradientEdit,
  interpolateStop,
  moveStop,
  presetStops,
  removeStop,
  snapAngle,
  sortStops,
  updateStop,
  type GradientStop,
} from './gradient';

const stop = (position: number, color: [number, number, number] = [0, 0, 0], opacity = 1): GradientStop => ({ position, color, opacity });

describe('presets', () => {
  it('build the stops from the colors', () => {
    const settings = defaultGradient();
    expect(presetStops(settings, [1, 0, 0], [0, 0, 1])).toEqual(foregroundToBackground([1, 0, 0], [0, 0, 1]));
    settings.preset = 'foregroundToTransparent';
    expect(presetStops(settings, [1, 0, 0], [0, 0, 1])).toEqual([stop(0, [1, 0, 0], 1), stop(1, [1, 0, 0], 0)]);
    expect(foregroundToTransparent([0.5, 0.5, 0.5])[1]?.opacity).toBe(0);
    settings.preset = 'blackToWhite';
    expect(presetStops(settings, [1, 0, 0], [0, 0, 1])).toEqual(blackToWhite());
  });

  it('custom copies its own stops', () => {
    const settings = { ...defaultGradient(), preset: 'custom' as const, stops: [stop(0, [1, 1, 0]), stop(1, [0, 1, 1])] };
    const out = presetStops(settings, [0, 0, 0], [1, 1, 1]);
    expect(out).toEqual(settings.stops);
    expect(out[0]).not.toBe(settings.stops[0]);
  });
});

describe('stops', () => {
  it('sort by position and keep the order of equal positions', () => {
    const sorted = sortStops([stop(0.5, [1, 0, 0]), stop(0, [0, 1, 0]), stop(0.5, [0, 0, 1])]);
    expect(sorted.map((s) => s.position)).toEqual([0, 0.5, 0.5]);
    expect(sorted[1]?.color).toEqual([1, 0, 0]);
    expect(sorted[2]?.color).toEqual([0, 0, 1]);
  });

  it('interpolate color and opacity', () => {
    const stops = [stop(0, [0, 0, 0], 1), stop(1, [1, 0.5, 1], 0)];
    expect(interpolateStop(stops, 0.5)).toEqual({ position: 0.5, color: [0.5, 0.25, 0.5], opacity: 0.5 });
    const outer = [stop(0.2, [1, 0, 0]), stop(0.8, [0, 0, 1])];
    expect(interpolateStop(outer, 0).color).toEqual([1, 0, 0]);
    expect(interpolateStop(outer, 1).color).toEqual([0, 0, 1]);
  });

  it('add inserts a sorted stop with the interpolated values', () => {
    const result = addStop(blackToWhite(), 0.25);
    expect(result?.index).toBe(1);
    expect(result?.stops.map((s) => s.position)).toEqual([0, 0.25, 1]);
    expect(result?.stops[1]?.color).toEqual([0.25, 0.25, 0.25]);
  });

  it('add stops at the maximum', () => {
    let stops = blackToWhite();
    for (let i = 2; i < MAX_GRADIENT_STOPS; i += 1) {
      const added = addStop(stops, i / 100);
      expect(added).not.toBeNull();
      stops = added?.stops ?? stops;
    }
    expect(stops).toHaveLength(MAX_GRADIENT_STOPS);
    expect(addStop(stops, 0.5)).toBeNull();
  });

  it('move re-sorts, tracks the index, and allows equal positions', () => {
    const stops = [stop(0, [1, 0, 0]), stop(0.5, [0, 1, 0]), stop(1, [0, 0, 1])];
    const moved = moveStop(stops, 0, 0.8);
    expect(moved?.index).toBe(1);
    expect(moved?.stops.map((s) => s.color)).toEqual([[0, 1, 0], [1, 0, 0], [0, 0, 1]]);
    const equal = moveStop(stops, 0, 0.5);
    expect(equal?.stops.map((s) => s.position)).toEqual([0.5, 0.5, 1]);
    expect(moveStop(stops, 0, 5)?.stops[2]?.position).toBe(1);
    expect(moveStop(stops, 9, 0.5)).toBeNull();
  });

  it('remove keeps at least 2 stops', () => {
    const three = [stop(0), stop(0.5), stop(1)];
    expect(removeStop(three, 1)?.map((s) => s.position)).toEqual([0, 1]);
    expect(removeStop(blackToWhite(), 0)).toBeNull();
    expect(removeStop(three, 7)).toBeNull();
  });

  it('update limits the opacity', () => {
    const out = updateStop(blackToWhite(), 0, { opacity: 3, color: [1, 0, 0] });
    expect(out?.[0]).toEqual(stop(0, [1, 0, 0], 1));
    expect(updateStop(blackToWhite(), 5, {})).toBeNull();
  });

  it('css gradient shows the opacity as alpha and reverses', () => {
    expect(cssGradient([stop(0, [1, 0, 0], 1), stop(1, [1, 0, 0], 0)])).toBe('linear-gradient(to right, #ff0000ff 0%, #ff000000 100%)');
    expect(cssGradient([stop(0, [1, 0, 0], 1), stop(1, [1, 0, 0], 0)], true)).toBe('linear-gradient(to right, #ff000000 0%, #ff0000ff 100%)');
  });
});

describe('snapAngle', () => {
  it('snaps to 45 degree steps and keeps the length', () => {
    const a = snapAngle({ x: 10, y: 10 }, { x: 30, y: 12 });
    expect(a.x).toBeCloseTo(10 + Math.hypot(20, 2), 6);
    expect(a.y).toBeCloseTo(10, 6);
    const b = snapAngle({ x: 0, y: 0 }, { x: 10, y: 8 });
    expect(b.x).toBeCloseTo(b.y, 6);
    expect(Math.hypot(b.x, b.y)).toBeCloseTo(Math.hypot(10, 8), 6);
    expect(snapAngle({ x: 1, y: 1 }, { x: 1, y: 1 })).toEqual({ x: 1, y: 1 });
  });
});

describe('gradientEdit', () => {
  it('builds the contract fields, not rounded', () => {
    const settings = { ...defaultGradient(), shape: 'radial' as const, opacity: 0.4, blendMode: 'multiply' as const, reverse: true };
    expect(gradientEdit(3, 'pixels', { x: 1.5, y: 2.25 }, { x: 30.75, y: 40 }, blackToWhite(), settings)).toEqual({
      type: 'gradient',
      layerId: 3,
      target: 'pixels',
      start: [1.5, 2.25],
      end: [30.75, 40],
      shape: 'radial',
      stops: [
        { position: 0, color: [0, 0, 0], opacity: 1 },
        { position: 1, color: [1, 1, 1], opacity: 1 },
      ],
      opacity: 0.4,
      blendMode: 'multiply',
      reverse: true,
    });
  });

  it('uses normal on the mask target', () => {
    const settings = { ...defaultGradient(), blendMode: 'multiply' as const };
    const edit = gradientEdit(1, 'mask', { x: 0, y: 0 }, { x: 5, y: 5 }, blackToWhite(), settings);
    expect(edit).toMatchObject({ target: 'mask', blendMode: 'normal' });
  });
});
