import { describe, expect, it } from 'vitest';
import { makeFolder } from '../../test/fakeCatalog';
import { ancestorIds, collectFolders, flattenTree } from './folderRows';

const tree = [
  makeFolder(1, 'a', 3, [makeFolder(2, 'b', 2, [makeFolder(3, 'c', 1)]), makeFolder(4, 'd', 0)]),
  makeFolder(5, 'e', 7),
];

describe('flattenTree', () => {
  it('lists only the roots if nothing is expanded', () => {
    const rows = flattenTree(tree, new Set());
    expect(rows.map((r) => r.node.folder.id)).toEqual([1, 5]);
    expect(rows[0]).toMatchObject({ depth: 0, expandable: true, expanded: false });
    expect(rows[1]).toMatchObject({ expandable: false, expanded: false });
  });

  it('lists the children of expanded folders in order, with depth', () => {
    const rows = flattenTree(tree, new Set([1, 2]));
    expect(rows.map((r) => [r.node.folder.id, r.depth])).toEqual([
      [1, 0],
      [2, 1],
      [3, 2],
      [4, 1],
      [5, 0],
    ]);
  });

  it('hides the children of a collapsed folder, also if the grandparent is open', () => {
    const rows = flattenTree(tree, new Set([1]));
    expect(rows.map((r) => r.node.folder.id)).toEqual([1, 2, 4, 5]);
  });

  it('ignores an expanded id of a folder without children', () => {
    expect(flattenTree(tree, new Set([5]))[1]?.expanded).toBe(false);
  });
});

describe('collectFolders', () => {
  it('lists all folders and those with children', () => {
    const { all, withChildren } = collectFolders(tree);
    expect(all.sort()).toEqual([1, 2, 3, 4, 5]);
    expect(withChildren.sort()).toEqual([1, 2]);
  });
});

describe('ancestorIds', () => {
  it('returns the path to a folder', () => {
    expect(ancestorIds(tree, 3)).toEqual([1, 2]);
    expect(ancestorIds(tree, 1)).toEqual([]);
  });

  it('returns an empty list for an unknown folder', () => {
    expect(ancestorIds(tree, 99)).toEqual([]);
  });
});
