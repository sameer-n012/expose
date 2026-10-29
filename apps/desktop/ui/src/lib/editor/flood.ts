/**
 * Settings and edit builders of the two flood tools: the magic wand and the
 * paint bucket. Both pick the pixels that are like the pixel under the click.
 */
import type { BlendModeId, EditDto, SelectionModeId } from './api';
import { clampUnit, type Rgb } from './brush';

/**
 * The largest tolerance. Copy of `MAX_FLOOD_TOLERANCE` in
 * crates/expose-editor/src/flood.rs. That file does not exist on every base
 * yet, so `lib/rustConstants.test.ts` does not check this copy yet.
 */
export const MAX_FLOOD_TOLERANCE = 255;

/** The settings that the magic wand and the paint bucket share. */
export interface FloodSettings {
  /** 0..=255. */
  tolerance: number;
  /** True: only the pixels that touch the click. False: all similar pixels. */
  contiguous: boolean;
  antiAlias: boolean;
  /** True: compare the composite. False: compare the selected layer. */
  sampleAllLayers: boolean;
}

/** The settings of the paint bucket. */
export interface PaintBucketSettings extends FloodSettings {
  /** 0..1. */
  opacity: number;
  blendMode: BlendModeId;
}

export function defaultWand(): FloodSettings {
  return { tolerance: 32, contiguous: true, antiAlias: true, sampleAllLayers: false };
}

export function defaultBucket(): PaintBucketSettings {
  return { ...defaultWand(), opacity: 1, blendMode: 'normal' };
}

/** Rounds a tolerance to a whole number in 0..=MAX_FLOOD_TOLERANCE. A value that is not a number gives 0. */
export function clampTolerance(value: number): number {
  return Number.isFinite(value) ? Math.min(MAX_FLOOD_TOLERANCE, Math.max(0, Math.round(value))) : 0;
}

/** The magic wand edit. `layerId` is the selected layer, or null. */
export function magicWandEdit(x: number, y: number, mode: SelectionModeId, settings: FloodSettings, layerId: number | null): EditDto {
  return {
    type: 'magicWand',
    x,
    y,
    mode,
    tolerance: clampTolerance(settings.tolerance),
    contiguous: settings.contiguous,
    antiAlias: settings.antiAlias,
    sampleAllLayers: settings.sampleAllLayers,
    layerId,
  };
}

/** The fill edit. `color` is gamma-encoded sRGB, 0..1. */
export function fillEdit(layerId: number, x: number, y: number, color: Rgb, settings: PaintBucketSettings): EditDto {
  return {
    type: 'fill',
    layerId,
    x,
    y,
    color,
    opacity: clampUnit(settings.opacity),
    blendMode: settings.blendMode,
    tolerance: clampTolerance(settings.tolerance),
    contiguous: settings.contiguous,
    antiAlias: settings.antiAlias,
    sampleAllLayers: settings.sampleAllLayers,
  };
}
