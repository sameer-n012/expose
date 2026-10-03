import { describe, expect, it } from 'vitest';
import {
  clickSelect,
  EMPTY_SELECTION,
  moveSelection,
  selectAllLoaded,
  type IdAt,
  type SelectionState,
} from './selection';

// Position i holds photo id 100 + i. Positions from 8 on are not loaded.
const LOADED = 8;
const idAt: IdAt = (i) => (i >= 0 && i < LOADED ? 100 + i : undefined);
const NONE = { toggle: false, range: false };

function ids(state: SelectionState): number[] {
  return [...state.ids].sort((a, b) => a - b);
}

describe('clickSelect', () => {
  it('selects one photo with a plain click and replaces the old selection', () => {
    const first = clickSelect(EMPTY_SELECTION, 2, idAt, NONE);
    expect(ids(first)).toEqual([102]);
    const second = clickSelect(first, 4, idAt, NONE);
    expect(ids(second)).toEqual([104]);
    expect(second).toMatchObject({ anchor: 4, focus: 4 });
  });

  it('toggles a photo with Cmd or Ctrl', () => {
    let state = clickSelect(EMPTY_SELECTION, 1, idAt, NONE);
    state = clickSelect(state, 3, idAt, { toggle: true, range: false });
    expect(ids(state)).toEqual([101, 103]);
    state = clickSelect(state, 1, idAt, { toggle: true, range: false });
    expect(ids(state)).toEqual([103]);
  });

  it('selects a range from the anchor with Shift, in both directions', () => {
    const start = clickSelect(EMPTY_SELECTION, 5, idAt, NONE);
    expect(ids(clickSelect(start, 2, idAt, { toggle: false, range: true }))).toEqual([102, 103, 104, 105]);
    expect(ids(clickSelect(start, 7, idAt, { toggle: false, range: true }))).toEqual([105, 106, 107]);
  });

  it('keeps the anchor for several Shift clicks', () => {
    let state = clickSelect(EMPTY_SELECTION, 2, idAt, NONE);
    state = clickSelect(state, 5, idAt, { toggle: false, range: true });
    state = clickSelect(state, 3, idAt, { toggle: false, range: true });
    expect(ids(state)).toEqual([102, 103]);
  });

  it('adds the range to the selection with Shift and Cmd', () => {
    let state = clickSelect(EMPTY_SELECTION, 0, idAt, NONE);
    state = clickSelect(state, 4, idAt, { toggle: true, range: false });
    state = clickSelect(state, 6, idAt, { toggle: true, range: true });
    expect(ids(state)).toEqual([100, 104, 105, 106]);
  });

  it('starts the range at the clicked photo if there is no anchor', () => {
    expect(ids(clickSelect(EMPTY_SELECTION, 3, idAt, { toggle: false, range: true }))).toEqual([103]);
  });

  it('skips photos that are not loaded in a range', () => {
    const start = clickSelect(EMPTY_SELECTION, 6, idAt, NONE);
    // The click on position 10 is not loaded: no change.
    expect(clickSelect(start, 10, idAt, NONE)).toBe(start);
    // A range that reaches over the unloaded part holds only loaded ids.
    const sparse: IdAt = (i) => (i === 6 || i === 9 ? 100 + i : undefined);
    const state = clickSelect(clickSelect(EMPTY_SELECTION, 6, sparse, NONE), 9, sparse, { toggle: false, range: true });
    expect(ids(state)).toEqual([106, 109]);
  });
});

describe('moveSelection', () => {
  it('moves the focus and selects the photo there', () => {
    const state = clickSelect(EMPTY_SELECTION, 2, idAt, NONE);
    const next = moveSelection(state, 1, 20, idAt, false);
    expect(ids(next)).toEqual([103]);
    expect(next.focus).toBe(3);
  });

  it('stops at the first photo and at the last photo', () => {
    const first = clickSelect(EMPTY_SELECTION, 0, idAt, NONE);
    expect(moveSelection(first, -1, 20, idAt, false).focus).toBe(0);
    const last = clickSelect(EMPTY_SELECTION, 7, idAt, NONE);
    // Position 8 is loaded in a grid of 8 photos only.
    expect(moveSelection(last, 1, 8, idAt, false).focus).toBe(7);
  });

  it('starts at the first photo (forward) or the last photo (backward) without focus', () => {
    expect(moveSelection(EMPTY_SELECTION, 1, 8, idAt, false).focus).toBe(0);
    expect(moveSelection(EMPTY_SELECTION, -1, 8, idAt, false).focus).toBe(7);
  });

  it('does not change the state if the target is not loaded', () => {
    const state = clickSelect(EMPTY_SELECTION, 7, idAt, NONE);
    expect(moveSelection(state, 1, 20, idAt, false)).toBe(state);
  });

  it('does not change the state for an empty grid', () => {
    expect(moveSelection(EMPTY_SELECTION, 1, 0, idAt, false)).toBe(EMPTY_SELECTION);
  });

  it('extends the selection from the anchor', () => {
    let state = clickSelect(EMPTY_SELECTION, 2, idAt, NONE);
    state = moveSelection(state, 1, 20, idAt, true);
    state = moveSelection(state, 1, 20, idAt, true);
    expect(ids(state)).toEqual([102, 103, 104]);
    state = moveSelection(state, -3, 20, idAt, true);
    expect(ids(state)).toEqual([101, 102]);
    expect(state.anchor).toBe(2);
  });
});

describe('selectAllLoaded', () => {
  it('selects the given ids and keeps the focus', () => {
    const state = clickSelect(EMPTY_SELECTION, 2, idAt, NONE);
    const all = selectAllLoaded(state, [100, 101, 102]);
    expect(ids(all)).toEqual([100, 101, 102]);
    expect(all.focus).toBe(2);
  });
});
