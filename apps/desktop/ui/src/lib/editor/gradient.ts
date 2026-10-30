/**
 * Settings, stop logic, and the edit builder of the gradient tool. All
 * functions here are pure: they return new arrays and do not change input.
 */
import type { BlendModeId, EditDto } from './api';
import { clampUnit, rgbToHex, type Rgb } from './brush';
import type { Point } from './view';

/**
 * The most stops in one gradient. Copy of `MAX_GRADIENT_STOPS` in
 * crates/expose-editor/src/gradient.rs. That file does not exist on every
 * base yet, so `lib/rustConstants.test.ts` does not check this copy yet.
 */
export const MAX_GRADIENT_STOPS = 32;
/** The fewest stops in one gradient. */
export const MIN_GRADIENT_STOPS = 2;

export type GradientShape = 'linear' | 'radial' | 'angle' | 'reflected' | 'diamond';

/** A color stop. `position` is 0..1, `color` is gamma-encoded sRGB 0..1, `opacity` is 0..1. */
export interface GradientStop {
  position: number;
  color: Rgb;
  opacity: number;
}

export type GradientPresetId = 'foregroundToBackground' | 'foregroundToTransparent' | 'blackToWhite' | 'custom';

export const GRADIENT_SHAPES: ReadonlyArray<{ id: GradientShape; label: string; title: string }> = [
  { id: 'linear', label: 'Linear', title: 'Linear gradient' },
  { id: 'radial', label: 'Radial', title: 'Radial gradient from the center' },
  { id: 'angle', label: 'Angle', title: 'Angle gradient around the start point' },
  { id: 'reflected', label: 'Reflected', title: 'Reflected gradient (mirrored about the start point)' },
  { id: 'diamond', label: 'Diamond', title: 'Diamond gradient from the center' },
];

export const GRADIENT_PRESETS: ReadonlyArray<{ id: GradientPresetId; label: string }> = [
  { id: 'foregroundToBackground', label: 'Foreground to background' },
  { id: 'foregroundToTransparent', label: 'Foreground to transparent' },
  { id: 'blackToWhite', label: 'Black to white' },
  { id: 'custom', label: 'Custom' },
];

/** The settings of the gradient tool. */
export interface GradientSettings {
  shape: GradientShape;
  preset: GradientPresetId;
  /** The stops of the `custom` preset. The other presets build their stops from the colors. */
  stops: GradientStop[];
  /** 0..1. */
  opacity: number;
  blendMode: BlendModeId;
  reverse: boolean;
}

export function defaultGradient(): GradientSettings {
  return {
    shape: 'linear',
    preset: 'foregroundToBackground',
    stops: blackToWhite(),
    opacity: 1,
    blendMode: 'normal',
    reverse: false,
  };
}

// ---- Presets ----

export function foregroundToBackground(foreground: Rgb, background: Rgb): GradientStop[] {
  return [
    { position: 0, color: [...foreground], opacity: 1 },
    { position: 1, color: [...background], opacity: 1 },
  ];
}

export function foregroundToTransparent(foreground: Rgb): GradientStop[] {
  return [
    { position: 0, color: [...foreground], opacity: 1 },
    { position: 1, color: [...foreground], opacity: 0 },
  ];
}

export function blackToWhite(): GradientStop[] {
  return [
    { position: 0, color: [0, 0, 0], opacity: 1 },
    { position: 1, color: [1, 1, 1], opacity: 1 },
  ];
}

/** The stops of a preset. The foreground and background colors are read when this is called. */
export function presetStops(settings: GradientSettings, foreground: Rgb, background: Rgb): GradientStop[] {
  switch (settings.preset) {
    case 'foregroundToBackground':
      return foregroundToBackground(foreground, background);
    case 'foregroundToTransparent':
      return foregroundToTransparent(foreground);
    case 'blackToWhite':
      return blackToWhite();
    case 'custom':
      return settings.stops.map(copyStop);
  }
}

function copyStop(stop: GradientStop): GradientStop {
  return { position: stop.position, color: [...stop.color], opacity: stop.opacity };
}

// ---- Stop logic ----

/** Limits a stop position to 0..1. A value that is not a number gives 0. */
export function clampPosition(value: number): number {
  return Number.isFinite(value) ? clampUnit(value) : 0;
}

/** Sorts by position. The sort is stable: stops at one position keep their order. */
export function sortStops(stops: readonly GradientStop[]): GradientStop[] {
  return stops.map(copyStop).sort((a, b) => a.position - b.position);
}

/**
 * The color and opacity of the gradient at `position`, from linear
 * interpolation between the two stops around it (in gamma-encoded sRGB, the
 * same as the shell). Before the first stop and after the last stop the end
 * stop values apply.
 */
export function interpolateStop(stops: readonly GradientStop[], position: number): GradientStop {
  const p = clampPosition(position);
  const sorted = sortStops(stops);
  const first = sorted[0];
  const last = sorted[sorted.length - 1];
  if (first === undefined || last === undefined) {
    return { position: p, color: [0, 0, 0], opacity: 1 };
  }
  if (p <= first.position) {
    return { ...copyStop(first), position: p };
  }
  if (p >= last.position) {
    return { ...copyStop(last), position: p };
  }
  for (let i = 1; i < sorted.length; i += 1) {
    const a = sorted[i - 1];
    const b = sorted[i];
    if (a === undefined || b === undefined || p > b.position) {
      continue;
    }
    const span = b.position - a.position;
    const t = span <= 0 ? 1 : (p - a.position) / span;
    const mix = (x: number, y: number): number => x + (y - x) * t;
    return {
      position: p,
      color: [mix(a.color[0], b.color[0]), mix(a.color[1], b.color[1]), mix(a.color[2], b.color[2])],
      opacity: mix(a.opacity, b.opacity),
    };
  }
  return { ...copyStop(last), position: p };
}

/** The result of a stop change: the new list and the index of the stop that the change is about. */
export interface StopChange {
  stops: GradientStop[];
  index: number;
}

/** Adds a stop at `position` with the interpolated values. Returns null at the maximum. */
export function addStop(stops: readonly GradientStop[], position: number): StopChange | null {
  if (stops.length >= MAX_GRADIENT_STOPS) {
    return null;
  }
  const added = interpolateStop(stops, position);
  const list = stops.map(copyStop);
  // The new stop goes after the stops at the same or an earlier position.
  let index = list.findIndex((s) => s.position > added.position);
  if (index < 0) {
    index = list.length;
  }
  list.splice(index, 0, added);
  return { stops: list, index };
}

/** Sets the position of a stop and sorts the list. Returns null for an index out of range. */
export function moveStop(stops: readonly GradientStop[], index: number, position: number): StopChange | null {
  const stop = stops[index];
  if (stop === undefined) {
    return null;
  }
  const moved = { ...copyStop(stop), position: clampPosition(position) };
  const rest = stops.filter((_, i) => i !== index).map(copyStop);
  // Insert after the stops at the same position, so the order is stable.
  let at = rest.findIndex((s) => s.position > moved.position);
  if (at < 0) {
    at = rest.length;
  }
  rest.splice(at, 0, moved);
  return { stops: rest, index: at };
}

/** Changes the color and/or the opacity of a stop. Returns null for an index out of range. */
export function updateStop(stops: readonly GradientStop[], index: number, change: Partial<Pick<GradientStop, 'color' | 'opacity'>>): GradientStop[] | null {
  const stop = stops[index];
  if (stop === undefined) {
    return null;
  }
  const list = stops.map(copyStop);
  list[index] = {
    position: stop.position,
    color: change.color === undefined ? [...stop.color] : [...change.color],
    opacity: change.opacity === undefined ? stop.opacity : clampUnit(change.opacity),
  };
  return list;
}

/** Removes a stop. Returns null if only the minimum is left or the index is out of range. */
export function removeStop(stops: readonly GradientStop[], index: number): GradientStop[] | null {
  if (stops.length <= MIN_GRADIENT_STOPS || index < 0 || index >= stops.length) {
    return null;
  }
  return stops.filter((_, i) => i !== index).map(copyStop);
}

/** A CSS `linear-gradient` of the stops, for the preview swatch. The opacity shows as alpha. */
export function cssGradient(stops: readonly GradientStop[], reverse = false): string {
  const list = reverse ? sortStops(stops.map((s) => ({ ...s, position: 1 - s.position })).reverse()) : sortStops(stops);
  const parts = list.map((s) => {
    const hex = rgbToHex(s.color);
    const alpha = Math.round(clampUnit(s.opacity) * 255)
      .toString(16)
      .padStart(2, '0');
    return `${hex}${alpha} ${Math.round(clampPosition(s.position) * 1000) / 10}%`;
  });
  return `linear-gradient(to right, ${parts.join(', ')})`;
}

// ---- Drag ----

/**
 * Snaps the direction from `start` to `end` to a multiple of 45°. The length
 * stays the same.
 */
export function snapAngle(start: Point, end: Point): Point {
  const dx = end.x - start.x;
  const dy = end.y - start.y;
  const length = Math.hypot(dx, dy);
  if (length === 0) {
    return { x: end.x, y: end.y };
  }
  const step = Math.PI / 4;
  const angle = Math.round(Math.atan2(dy, dx) / step) * step;
  return { x: start.x + Math.cos(angle) * length, y: start.y + Math.sin(angle) * length };
}

// ---- Edit ----

/**
 * The gradient edit. `stops` are the resolved stops (see `presetStops`).
 * `start` and `end` are canvas pixels and are not rounded. On the mask
 * target the blend mode is `normal`: a mask has no blend mode.
 */
export function gradientEdit(
  layerId: number,
  target: 'pixels' | 'mask',
  start: Point,
  end: Point,
  stops: readonly GradientStop[],
  settings: GradientSettings,
): EditDto {
  return {
    type: 'gradient',
    layerId,
    target,
    start: [start.x, start.y],
    end: [end.x, end.y],
    shape: settings.shape,
    stops: sortStops(stops).map((s) => ({ position: clampPosition(s.position), color: [...s.color], opacity: clampUnit(s.opacity) })),
    opacity: clampUnit(settings.opacity),
    blendMode: target === 'mask' ? 'normal' : settings.blendMode,
    reverse: settings.reverse,
  };
}
