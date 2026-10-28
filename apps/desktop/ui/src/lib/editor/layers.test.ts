import { describe, expect, it } from 'vitest';
import type { LayerInfo } from './api';
import {
  BLEND_GROUPS,
  NO_LOCKS,
  defaultAdjustment,
  effectiveLocks,
  hasOwnLock,
  layerRows,
  maskChipTitle,
  maskCommands,
  moveStep,
  newLayerIds,
  positionOf,
} from './layers';

function layer(id: number, extra: Partial<LayerInfo> = {}): LayerInfo {
  return {
    id,
    name: `L${id}`,
    kind: 'pixel',
    visible: true,
    opacity: 1,
    fillOpacity: 1,
    blendMode: 'normal',
    hasMask: false,
    maskEnabled: false,
    adjustment: null,
    isolated: null,
    children: null,
    locks: { transparency: false, pixels: false, position: false, all: false },
    ...extra,
  };
}

// Bottom-first: 1, 2, group 3 [4, 5].
const tree: LayerInfo[] = [layer(1), layer(2), layer(3, { kind: 'group', isolated: true, children: [layer(4), layer(5)] })];

describe('maskCommands', () => {
  const NONE_LOCKS = { transparency: false, pixels: false, position: false, all: false };
  const masked = { hasMask: true, maskEnabled: true };

  it('allows add on a layer without a mask', () => {
    expect(maskCommands([layer(1)], 1)).toEqual({ canAdd: true, canDelete: false, canToggle: false });
  });

  it('allows delete and toggle on a layer with a mask', () => {
    expect(maskCommands([layer(1, masked)], 1)).toEqual({ canAdd: false, canDelete: true, canToggle: true });
  });

  it('lets the pixels lock stop add and delete but not the switch', () => {
    const locks = { ...NONE_LOCKS, pixels: true };
    expect(maskCommands([layer(1, { locks })], 1).canAdd).toBe(false);
    expect(maskCommands([layer(1, { ...masked, locks })], 1)).toEqual({ canAdd: false, canDelete: false, canToggle: true });
  });

  it('lets the all lock of a parent group stop everything', () => {
    const group = layer(3, { kind: 'group', isolated: true, locks: { ...NONE_LOCKS, all: true }, children: [layer(4, masked), layer(5)] });
    expect(maskCommands([group], 4)).toEqual({ canAdd: false, canDelete: false, canToggle: false });
    expect(maskCommands([group], 5).canAdd).toBe(false);
  });

  it('has no commands without a layer', () => {
    expect(maskCommands([layer(1)], null)).toEqual({ canAdd: false, canDelete: false, canToggle: false });
    expect(maskCommands([layer(1)], 9)).toEqual({ canAdd: false, canDelete: false, canToggle: false });
  });

  it('names the chip action', () => {
    expect(maskChipTitle(layer(1, masked))).toContain('turn it off');
    expect(maskChipTitle(layer(1, { hasMask: true, maskEnabled: false }))).toContain('turn it on');
  });
});

describe('layerRows', () => {
  it('lists the top layer first and hides collapsed children', () => {
    expect(layerRows(tree, new Set()).map((r) => r.layer.id)).toEqual([3, 2, 1]);
  });

  it('lists the children of an expanded group, reversed, with depth', () => {
    const rows = layerRows(tree, new Set([3]));
    expect(rows.map((r) => [r.layer.id, r.depth])).toEqual([
      [3, 0],
      [5, 1],
      [4, 1],
      [2, 0],
      [1, 0],
    ]);
  });
});

describe('moveStep', () => {
  it('moves up by one place in the bottom-first list', () => {
    expect(moveStep(tree, 1, 'up')).toEqual({ type: 'move', layerId: 1, parentId: null, index: 1 });
  });

  it('moves down', () => {
    expect(moveStep(tree, 2, 'down')).toEqual({ type: 'move', layerId: 2, parentId: null, index: 0 });
  });

  it('stops at the ends', () => {
    expect(moveStep(tree, 3, 'up')).toBeNull();
    expect(moveStep(tree, 1, 'down')).toBeNull();
  });

  it('uses the parent group for a child', () => {
    expect(moveStep(tree, 4, 'up')).toEqual({ type: 'move', layerId: 4, parentId: 3, index: 1 });
    expect(positionOf(tree, 5)).toEqual({ parentId: 3, index: 1, siblings: 2 });
  });

  it('returns null for an unknown layer', () => {
    expect(moveStep(tree, 99, 'up')).toBeNull();
  });
});

describe('menus and defaults', () => {
  it('has all 27 blend modes once, in 6 groups', () => {
    const ids = BLEND_GROUPS.flat().map((m) => m.id);
    expect(BLEND_GROUPS).toHaveLength(6);
    expect(ids).toHaveLength(27);
    expect(new Set(ids).size).toBe(27);
  });

  it('gives the no-effect defaults', () => {
    expect(defaultAdjustment('brightnessContrast')).toEqual({ type: 'brightnessContrast', brightness: 0, contrast: 0 });
    expect(defaultAdjustment('hueSaturation')).toEqual({ type: 'hueSaturation', hue: 0, saturation: 0, lightness: 0 });
    const levels = defaultAdjustment('levels');
    expect(levels.type === 'levels' && levels.red).toEqual({ inputBlack: 0, inputWhite: 1, gamma: 1, outputBlack: 0, outputWhite: 1 });
    const curves = defaultAdjustment('curves');
    expect(curves.type === 'curves' && curves.blue).toEqual([
      { x: 0, y: 0 },
      { x: 1, y: 1 },
    ]);
  });

  it('finds new layer ids', () => {
    expect(newLayerIds(new Set([1, 2, 3, 4, 5]), [...tree, layer(6)])).toEqual([6]);
  });
});

describe('effectiveLocks', () => {
  const lockAll = { transparency: false, pixels: false, position: false, all: true };
  const lockPos = { transparency: false, pixels: false, position: true, all: false };

  it('returns the own locks of a root layer', () => {
    expect(effectiveLocks([layer(1, { locks: lockPos })], 1)).toEqual(lockPos);
  });

  it('adds the locks of the ancestor groups', () => {
    const layers = [layer(3, { kind: 'group', isolated: true, locks: lockAll, children: [layer(4, { locks: lockPos })] })];
    expect(effectiveLocks(layers, 4)).toEqual({ transparency: false, pixels: false, position: true, all: true });
    expect(effectiveLocks(layers, 3).position).toBe(false);
  });

  it('returns no locks for a missing layer', () => {
    expect(effectiveLocks(tree, 99)).toEqual(NO_LOCKS);
  });

  it('hasOwnLock looks at the own locks only', () => {
    expect(hasOwnLock(layer(1, { locks: lockPos }))).toBe(true);
    expect(hasOwnLock(layer(1))).toBe(false);
  });
});
