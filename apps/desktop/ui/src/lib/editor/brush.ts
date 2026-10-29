/**
 * Pure helpers of the brush and eraser tools: colors, brush settings, size
 * steps, and the stroke target that the shell receives.
 */
import type { BrushSettingsDto, StrokeTargetDto } from './api';

/** A color as gamma-encoded sRGB, each channel 0..1. */
export type Rgb = [number, number, number];

/** The tools that paint. */
export type PaintTool = 'brush' | 'eraser';

/** What a stroke changes in the selected layer. */
export type PaintTarget = 'pixels' | 'mask';

// The size and spacing limits are copies of `MIN_BRUSH_*` and `MAX_BRUSH_*` in
// crates/expose-editor/src/brush.rs. `lib/rustConstants.test.ts` checks them.
export const MIN_SIZE = 1;
export const MAX_SIZE = 5000;
export const MIN_SPACING = 0.01;
export const MAX_SPACING = 10;
/** The change of the hardness for `{` and `}`. */
export const HARDNESS_STEP = 0.25;

/** The settings of one brush. The UI shows opacity, flow, and hardness as percent; here they are 0..1. */
export type BrushSettings = BrushSettingsDto;

/** The start settings of the brush tool. */
export function defaultBrush(): BrushSettings {
  return { size: 30, hardness: 0.8, opacity: 1, flow: 1, blendMode: 'normal', spacing: 0.25, pressureSize: false, pressureOpacity: false };
}

/** The start settings of the eraser tool. */
export function defaultEraser(): BrushSettings {
  return { size: 50, hardness: 1, opacity: 1, flow: 1, blendMode: 'normal', spacing: 0.25, pressureSize: false, pressureOpacity: false };
}

function clamp(value: number, min: number, max: number, fallback: number): number {
  return Number.isFinite(value) ? Math.min(max, Math.max(min, value)) : fallback;
}

/** Limits a size to 1..5000. A value that is not a number gives 1. */
export function clampSize(size: number): number {
  return clamp(size, MIN_SIZE, MAX_SIZE, MIN_SIZE);
}

/** Limits a value to 0..1. A value that is not a number gives 0. */
export function clampUnit(value: number): number {
  return clamp(value, 0, 1, 0);
}

/** Limits a spacing to 0.01..10. A value that is not a number gives 0.25. */
export function clampSpacing(spacing: number): number {
  return clamp(spacing, MIN_SPACING, MAX_SPACING, 0.25);
}

/** The size slider is logarithmic: position 0..1 to a size. The size is a whole number. */
export function sliderToSize(position: number): number {
  const t = clampUnit(position);
  return clampSize(Math.round(Math.exp(Math.log(MIN_SIZE) + t * (Math.log(MAX_SIZE) - Math.log(MIN_SIZE)))));
}

/** The inverse of `sliderToSize`. */
export function sizeToSlider(size: number): number {
  return (Math.log(clampSize(size)) - Math.log(MIN_SIZE)) / (Math.log(MAX_SIZE) - Math.log(MIN_SIZE));
}

/** The change of the size for one press of `[` or `]`. It grows with the size. */
export function sizeStep(size: number): number {
  if (size < 10) {
    return 1;
  }
  if (size < 50) {
    return 5;
  }
  if (size < 100) {
    return 10;
  }
  if (size < 300) {
    return 25;
  }
  if (size < 1000) {
    return 50;
  }
  return 100;
}

/** The size after one press of `]` (direction 1) or `[` (direction -1). */
export function steppedSize(size: number, direction: 1 | -1): number {
  const current = clampSize(size);
  // Going down uses the step of the size just below, so up and down are inverse near the limits.
  const step = sizeStep(direction === 1 ? current : current - 1);
  return clampSize(Math.round(current + direction * step));
}

/** The hardness after one press of `}` (direction 1) or `{` (direction -1). */
export function steppedHardness(hardness: number, direction: 1 | -1): number {
  return clampUnit(Math.round((hardness + direction * HARDNESS_STEP) * 100) / 100);
}

/** Returns a copy of the settings with every value in its range. */
export function normalizeBrush(brush: BrushSettings): BrushSettings {
  return {
    size: clampSize(brush.size),
    hardness: clampUnit(brush.hardness),
    opacity: clampUnit(brush.opacity),
    flow: clampUnit(brush.flow),
    spacing: clampSpacing(brush.spacing),
    // The shell refuses dissolve for strokes.
    blendMode: brush.blendMode === 'dissolve' ? 'normal' : brush.blendMode,
    pressureSize: brush.pressureSize,
    pressureOpacity: brush.pressureOpacity,
  };
}

// ---- Colors ----

/** Converts "#rrggbb" to gamma-encoded sRGB 0..1. Returns null for text in another form. */
export function hexToRgb(hex: string): Rgb | null {
  const match = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
  if (match === null) {
    return null;
  }
  return [parseInt(match[1] ?? '0', 16) / 255, parseInt(match[2] ?? '0', 16) / 255, parseInt(match[3] ?? '0', 16) / 255];
}

/** Converts sRGB 0..1 to "#rrggbb" (lowercase). Values out of range are limited. */
export function rgbToHex(color: Rgb): string {
  const part = (v: number): string =>
    Math.round(clampUnit(v) * 255)
      .toString(16)
      .padStart(2, '0');
  return `#${part(color[0])}${part(color[1])}${part(color[2])}`;
}

/**
 * The luminance of a color: the Rec. 709 weights (0.2126, 0.7152, 0.0722)
 * applied to the gamma-encoded values. The mask is also gamma-encoded, so
 * a gray swatch gives the same gray value in the mask. The result is 0..1.
 */
export function luminance(color: Rgb): number {
  return clampUnit(0.2126 * color[0] + 0.7152 * color[1] + 0.0722 * color[2]);
}

/** The start colors. */
export const DEFAULT_FOREGROUND = '#000000';
export const DEFAULT_BACKGROUND = '#ffffff';

/**
 * The target that the shell receives for a stroke.
 *
 * - Pixels: the brush paints the foreground color, the eraser erases.
 * - Mask: the brush paints the luminance of the foreground color, the
 *   eraser paints the luminance of the background color.
 *
 * Returns null if a color is not valid hex text.
 */
export function strokeTarget(tool: PaintTool, target: PaintTarget, foreground: string, background: string): StrokeTargetDto | null {
  const fg = hexToRgb(foreground);
  const bg = hexToRgb(background);
  if (fg === null || bg === null) {
    return null;
  }
  if (target === 'mask') {
    return { type: 'mask', value: luminance(tool === 'brush' ? fg : bg) };
  }
  return tool === 'brush' ? { type: 'paint', color: fg } : { type: 'erase' };
}

/**
 * The brush that a stroke sends. Only paint strokes of the brush tool use
 * the blend mode; the eraser and mask paint always send 'normal'.
 */
export function strokeBrush(tool: PaintTool, target: PaintTarget, brush: BrushSettings): BrushSettings {
  const normal = tool === 'eraser' || target === 'mask';
  return { ...brush, blendMode: normal ? 'normal' : brush.blendMode };
}

/** The pressure of a pointer event: the pen pressure for a pen, else 1. */
export function pointerPressure(event: { pointerType: string; pressure: number }): number {
  return event.pointerType === 'pen' ? clampUnit(event.pressure) : 1;
}

// ---- Cursor ----

/** A cursor circle smaller than this (screen pixels) also shows a crosshair. */
export const CROSSHAIR_BELOW = 4;

/** The diameter of the cursor circle on the screen, for a brush size and a zoom. */
export function cursorDiameter(size: number, zoom: number): number {
  return clampSize(size) * zoom;
}
