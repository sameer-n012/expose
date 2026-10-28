/**
 * Pure helpers for the layer tree: display order, lookup, reorder, blend
 * mode menu, and default adjustments.
 */
import type { AdjustmentDto, BlendModeId, EditDto, LayerInfo, LocksDto } from './api';

/** One row of the layers panel. */
export interface LayerRow {
  readonly layer: LayerInfo;
  /** Nesting depth (0 = root). */
  readonly depth: number;
}

/**
 * The rows of the panel, top layer first. The backend lists layers
 * bottom-first, so each list is reversed. A group shows its children only
 * if its id is in `expanded`.
 */
export function layerRows(layers: readonly LayerInfo[], expanded: ReadonlySet<number>): LayerRow[] {
  const rows: LayerRow[] = [];
  const walk = (list: readonly LayerInfo[], depth: number): void => {
    for (let i = list.length - 1; i >= 0; i -= 1) {
      const layer = list[i];
      if (layer === undefined) {
        continue;
      }
      rows.push({ layer, depth });
      if (layer.kind === 'group' && layer.children !== null && expanded.has(layer.id)) {
        walk(layer.children, depth + 1);
      }
    }
  };
  walk(layers, 0);
  return rows;
}

/** Finds a layer by id anywhere in the tree. */
export function findLayer(layers: readonly LayerInfo[], id: number): LayerInfo | null {
  for (const layer of layers) {
    if (layer.id === id) {
      return layer;
    }
    if (layer.children !== null) {
      const found = findLayer(layer.children, id);
      if (found !== null) {
        return found;
      }
    }
  }
  return null;
}

/** All layer ids in the tree. */
export function allLayerIds(layers: readonly LayerInfo[]): Set<number> {
  const ids = new Set<number>();
  const walk = (list: readonly LayerInfo[]): void => {
    for (const layer of list) {
      ids.add(layer.id);
      if (layer.children !== null) {
        walk(layer.children);
      }
    }
  };
  walk(layers);
  return ids;
}

/** The position of a layer: its parent group id (null = root) and its index in the bottom-first list. */
export interface LayerPosition {
  readonly parentId: number | null;
  readonly index: number;
  readonly siblings: number;
}

export function positionOf(layers: readonly LayerInfo[], id: number, parentId: number | null = null): LayerPosition | null {
  for (let i = 0; i < layers.length; i += 1) {
    const layer = layers[i];
    if (layer === undefined) {
      continue;
    }
    if (layer.id === id) {
      return { parentId, index: i, siblings: layers.length };
    }
    if (layer.children !== null) {
      const found = positionOf(layer.children, id, layer.id);
      if (found !== null) {
        return found;
      }
    }
  }
  return null;
}

/**
 * The move edit that moves a layer one step up (towards the top of the
 * panel) or down inside its parent. Returns null at the end of the list.
 */
export function moveStep(layers: readonly LayerInfo[], id: number, direction: 'up' | 'down'): EditDto | null {
  const pos = positionOf(layers, id);
  if (pos === null) {
    return null;
  }
  // `index` of the edit counts after the layer is removed. Up: one place
  // after the next layer in the list (a larger index is higher).
  const index = direction === 'up' ? pos.index + 1 : pos.index - 1;
  if (index < 0 || index >= pos.siblings) {
    return null;
  }
  return { type: 'move', layerId: id, parentId: pos.parentId, index };
}

/** Menu groups of the blend modes, in menu order. */
export const BLEND_GROUPS: ReadonlyArray<ReadonlyArray<{ id: BlendModeId; label: string }>> = [
  [
    { id: 'normal', label: 'Normal' },
    { id: 'dissolve', label: 'Dissolve' },
  ],
  [
    { id: 'darken', label: 'Darken' },
    { id: 'multiply', label: 'Multiply' },
    { id: 'colorBurn', label: 'Color burn' },
    { id: 'linearBurn', label: 'Linear burn' },
    { id: 'darkerColor', label: 'Darker color' },
  ],
  [
    { id: 'lighten', label: 'Lighten' },
    { id: 'screen', label: 'Screen' },
    { id: 'colorDodge', label: 'Color dodge' },
    { id: 'linearDodge', label: 'Linear dodge' },
    { id: 'lighterColor', label: 'Lighter color' },
  ],
  [
    { id: 'overlay', label: 'Overlay' },
    { id: 'softLight', label: 'Soft light' },
    { id: 'hardLight', label: 'Hard light' },
    { id: 'vividLight', label: 'Vivid light' },
    { id: 'linearLight', label: 'Linear light' },
    { id: 'pinLight', label: 'Pin light' },
    { id: 'hardMix', label: 'Hard mix' },
  ],
  [
    { id: 'difference', label: 'Difference' },
    { id: 'exclusion', label: 'Exclusion' },
    { id: 'subtract', label: 'Subtract' },
    { id: 'divide', label: 'Divide' },
  ],
  [
    { id: 'hue', label: 'Hue' },
    { id: 'saturation', label: 'Saturation' },
    { id: 'color', label: 'Color' },
    { id: 'luminosity', label: 'Luminosity' },
  ],
];

/** The kinds of adjustment that the "Add adjustment" menu offers. */
export type AdjustmentType = AdjustmentDto['type'];

export const ADJUSTMENT_LABELS: Readonly<Record<AdjustmentType, string>> = {
  brightnessContrast: 'Brightness/Contrast',
  levels: 'Levels',
  curves: 'Curves',
  hueSaturation: 'Hue/Saturation',
};

/** The menu order. */
export const ADJUSTMENT_TYPES: readonly AdjustmentType[] = ['brightnessContrast', 'levels', 'curves', 'hueSaturation'];

/** The default (no-effect) parameters of an adjustment. */
export function defaultAdjustment(type: AdjustmentType): AdjustmentDto {
  const channel = () => ({ inputBlack: 0, inputWhite: 1, gamma: 1, outputBlack: 0, outputWhite: 1 });
  const line = () => [
    { x: 0, y: 0 },
    { x: 1, y: 1 },
  ];
  switch (type) {
    case 'brightnessContrast':
      return { type, brightness: 0, contrast: 0 };
    case 'levels':
      return { type, master: channel(), red: channel(), green: channel(), blue: channel() };
    case 'curves':
      return { type, master: line(), red: line(), green: line(), blue: line() };
    case 'hueSaturation':
      return { type, hue: 0, saturation: 0, lightness: 0 };
  }
}

/** Layer ids in `after` that are not in `before` (for example the layer that a duplicate made). */
export function newLayerIds(before: ReadonlySet<number>, after: readonly LayerInfo[]): number[] {
  return [...allLayerIds(after)].filter((id) => !before.has(id));
}

/** No lock set. */
export const NO_LOCKS: LocksDto = { transparency: false, pixels: false, position: false, all: false };

/**
 * The locks that apply to a layer: the union of its own locks and the locks
 * of all its ancestor groups. A missing layer has no locks.
 */
export function effectiveLocks(layers: readonly LayerInfo[], id: number): LocksDto {
  const walk = (list: readonly LayerInfo[], inherited: LocksDto): LocksDto | null => {
    for (const layer of list) {
      const own = {
        transparency: inherited.transparency || layer.locks.transparency,
        pixels: inherited.pixels || layer.locks.pixels,
        position: inherited.position || layer.locks.position,
        all: inherited.all || layer.locks.all,
      };
      if (layer.id === id) {
        return own;
      }
      if (layer.children !== null) {
        const found = walk(layer.children, own);
        if (found !== null) {
          return found;
        }
      }
    }
    return null;
  };
  return walk(layers, NO_LOCKS) ?? NO_LOCKS;
}

/** Which mask commands are available for a layer. */
export interface MaskCommands {
  /** A layer without a mask, with no pixels lock. */
  readonly canAdd: boolean;
  /** A layer with a mask, with no pixels lock. */
  readonly canDelete: boolean;
  /** A layer with a mask, with no `all` lock. */
  readonly canToggle: boolean;
}

/**
 * The mask commands of the layer `id`. The locks are the effective locks
 * (own and from ancestor groups). The pixels lock (and `all`) stops add
 * and delete. Only `all` stops the enable switch. A missing layer has no
 * commands.
 */
export function maskCommands(layers: readonly LayerInfo[], id: number | null): MaskCommands {
  const layer = id === null ? null : findLayer(layers, id);
  if (id === null || layer === null) {
    return { canAdd: false, canDelete: false, canToggle: false };
  }
  const locks = effectiveLocks(layers, id);
  const pixelsFree = !locks.pixels && !locks.all;
  return {
    canAdd: !layer.hasMask && pixelsFree,
    canDelete: layer.hasMask && pixelsFree,
    canToggle: layer.hasMask && !locks.all,
  };
}

/** The tooltip of the mask chip in a layer row. */
export function maskChipTitle(layer: LayerInfo): string {
  const toggle = layer.maskEnabled ? 'Shift+click to turn it off.' : 'Shift+click to turn it on.';
  return `Layer mask ${layer.maskEnabled ? 'on' : 'off'}. ${toggle} Alt+click to show it on the canvas.`;
}

/** True if the layer has any own lock. */
export function hasOwnLock(layer: LayerInfo): boolean {
  const l = layer.locks;
  return l.transparency || l.pixels || l.position || l.all;
}
