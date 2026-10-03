/**
 * Selection model of the library grid. Pure functions on an immutable state.
 *
 * The grid is indexed by position (0 .. total-1). The photos load in pages,
 * so a position can be not loaded yet. `idAt` returns undefined for such a
 * position. The selection holds only ids of loaded photos.
 */
import type { PhotoId } from '../catalog';

export interface SelectionState {
  /** The selected photo ids. */
  readonly ids: ReadonlySet<PhotoId>;
  /** The position where a range selection starts. Null if none. */
  readonly anchor: number | null;
  /** The position of the keyboard focus (the last clicked or moved to). Null if none. */
  readonly focus: number | null;
}

export const EMPTY_SELECTION: SelectionState = { ids: new Set(), anchor: null, focus: null };

/** Modifier keys of a pointer click. `toggle` is Cmd or Ctrl. */
export interface ClickModifiers {
  readonly toggle: boolean;
  readonly range: boolean;
}

/** Returns the id at a position, or undefined if the position is not loaded. */
export type IdAt = (index: number) => PhotoId | undefined;

/** Collects the loaded ids of the positions from `a` to `b` (both included, any order). */
function idsInRange(a: number, b: number, idAt: IdAt): PhotoId[] {
  const result: PhotoId[] = [];
  for (let i = Math.min(a, b); i <= Math.max(a, b); i++) {
    const id = idAt(i);
    if (id !== undefined) {
      result.push(id);
    }
  }
  return result;
}

/**
 * Applies a click on the photo at `index`.
 * - Plain click: select only this photo.
 * - Toggle (Cmd/Ctrl): add or remove this photo. The anchor moves here.
 * - Range (Shift): select from the anchor to this photo. With toggle too,
 *   the range is added to the current selection.
 * A click on a photo that is not loaded does not change the state.
 */
export function clickSelect(state: SelectionState, index: number, idAt: IdAt, mods: ClickModifiers): SelectionState {
  const id = idAt(index);
  if (id === undefined) {
    return state;
  }
  if (mods.range) {
    const anchor = state.anchor ?? index;
    const range = idsInRange(anchor, index, idAt);
    const ids = mods.toggle ? new Set([...state.ids, ...range]) : new Set(range);
    return { ids, anchor, focus: index };
  }
  if (mods.toggle) {
    const ids = new Set(state.ids);
    if (!ids.delete(id)) {
      ids.add(id);
    }
    return { ids, anchor: index, focus: index };
  }
  return { ids: new Set([id]), anchor: index, focus: index };
}

/**
 * Moves the focus by `delta` positions (clamped to the grid) and selects the
 * photo there. With `extend`, the selection grows from the anchor to the new
 * focus (range selection with the keyboard).
 *
 * If nothing has focus yet, the move goes to the first photo for a forward
 * move, and to the last photo for a backward move.
 * If the target photo is not loaded, the state does not change. The grid
 * loads the page when it scrolls there.
 */
export function moveSelection(
  state: SelectionState,
  delta: number,
  total: number,
  idAt: IdAt,
  extend: boolean,
): SelectionState {
  if (total <= 0) {
    return state;
  }
  let target: number;
  if (state.focus !== null) {
    target = Math.min(Math.max(state.focus + delta, 0), total - 1);
  } else {
    target = delta >= 0 ? 0 : total - 1;
  }
  const id = idAt(target);
  if (id === undefined) {
    return state;
  }
  if (extend) {
    const anchor = state.anchor ?? state.focus ?? target;
    return { ids: new Set(idsInRange(anchor, target, idAt)), anchor, focus: target };
  }
  return { ids: new Set([id]), anchor: target, focus: target };
}

/** Selects all loaded photos. The anchor and focus stay as they are. */
export function selectAllLoaded(state: SelectionState, loadedIds: readonly PhotoId[]): SelectionState {
  return { ...state, ids: new Set(loadedIds) };
}

/** Removes the selection. */
export function clearSelection(): SelectionState {
  return EMPTY_SELECTION;
}
