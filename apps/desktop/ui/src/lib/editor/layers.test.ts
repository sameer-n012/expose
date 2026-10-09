import { describe, expect, it } from 'vitest';
import type { LayerInfo } from './api';
import { BLEND_GROUPS, defaultAdjustment, layerRows, moveStep, newLayerIds, positionOf } from './layers';

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
    adjustment: null,
    isolated: null,
    children: null,
    ...extra,
  };
}

// Bottom-first: 1, 2, group 3 [4, 5].
const tree: LayerInfo[] = [layer(1), layer(2), layer(3, { kind: 'group', isolated: true, children: [layer(4), layer(5)] })];

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
