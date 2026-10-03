/**
 * Flat list of the visible rows of the folder tree.
 */
import type { FolderId, FolderNode } from '../catalog';

export interface FolderRow {
  readonly node: FolderNode;
  /** Nesting level, starting at 0 for a root folder. */
  readonly depth: number;
  readonly expandable: boolean;
  readonly expanded: boolean;
}

/** Lists the rows in display order. Children of a collapsed folder are left out. */
export function flattenTree(nodes: readonly FolderNode[], expanded: ReadonlySet<FolderId>): FolderRow[] {
  const rows: FolderRow[] = [];
  const visit = (list: readonly FolderNode[], depth: number): void => {
    for (const node of list) {
      const expandable = node.children.length > 0;
      const isExpanded = expandable && expanded.has(node.folder.id);
      rows.push({ node, depth, expandable, expanded: isExpanded });
      if (isExpanded) {
        visit(node.children, depth + 1);
      }
    }
  };
  visit(nodes, 0);
  return rows;
}

/** Collects the ids of all folders in the tree, and those that have children. */
export function collectFolders(nodes: readonly FolderNode[]): { all: FolderId[]; withChildren: FolderId[] } {
  const all: FolderId[] = [];
  const withChildren: FolderId[] = [];
  const stack = [...nodes];
  while (stack.length > 0) {
    const node = stack.pop();
    if (node === undefined) {
      break;
    }
    all.push(node.folder.id);
    if (node.children.length > 0) {
      withChildren.push(node.folder.id);
      stack.push(...node.children);
    }
  }
  return { all, withChildren };
}

/** Returns the ids of the ancestors of a folder, or an empty list if it is not in the tree. */
export function ancestorIds(nodes: readonly FolderNode[], target: FolderId): FolderId[] {
  const path: FolderId[] = [];
  const search = (list: readonly FolderNode[]): boolean => {
    for (const node of list) {
      if (node.folder.id === target) {
        return true;
      }
      path.push(node.folder.id);
      if (search(node.children)) {
        return true;
      }
      path.pop();
    }
    return false;
  };
  return search(nodes) ? path : [];
}
