/**
 * Shortcut actions of the Library module: rating, flag, color label,
 * selection movement, and select all.
 *
 * The actions have scope `library`. Thus the registry runs them only while
 * the Library module is active, and not while focus is in a text field.
 */
import type { ColorLabel, Flag } from '../catalog';
import type { ShortcutAction, ShortcutRegistry } from '../shortcuts/registry';

/** What the actions do. The library store implements this. */
export interface LibraryCommands {
  setRating(rating: number): void;
  setFlag(flag: Flag): void;
  setColorLabel(label: ColorLabel): void;
  selectAll(): void;
  /** Moves the selection by `dx` photos and `dy` rows. With `extend`, grows the selection. */
  moveSelection(dx: number, dy: number, extend: boolean): void;
}

/** Id prefix of all library actions. */
export const LIBRARY_ACTION_PREFIX = 'library.';

/** Keys of the color labels. Purple has no key. Use the toolbar for it. */
export const COLOR_LABEL_KEYS: ReadonlyArray<readonly [string, ColorLabel]> = [
  ['6', 'red'],
  ['7', 'yellow'],
  ['8', 'green'],
  ['9', 'blue'],
];

const FLAG_KEYS: ReadonlyArray<readonly [string, Flag, string]> = [
  ['P', 'pick', 'Flag as pick'],
  ['X', 'reject', 'Flag as reject'],
  ['U', 'none', 'Remove flag'],
];

const MOVES: ReadonlyArray<readonly [string, number, number]> = [
  ['ArrowLeft', -1, 0],
  ['ArrowRight', 1, 0],
  ['ArrowUp', 0, -1],
  ['ArrowDown', 0, 1],
];

const LABEL_NAMES: Readonly<Record<ColorLabel, string>> = {
  none: 'no label',
  red: 'red',
  yellow: 'yellow',
  green: 'green',
  blue: 'blue',
  purple: 'purple',
};

/** Creates the action definitions. */
export function libraryActions(commands: LibraryCommands): ShortcutAction[] {
  const actions: ShortcutAction[] = [];
  for (let rating = 0; rating <= 5; rating++) {
    actions.push({
      id: `${LIBRARY_ACTION_PREFIX}rating.${rating}`,
      label: rating === 0 ? 'Remove rating' : `Rate ${rating} star${rating === 1 ? '' : 's'}`,
      scope: 'library',
      defaultBinding: String(rating),
      run: () => commands.setRating(rating),
    });
  }
  for (const [key, flag, label] of FLAG_KEYS) {
    actions.push({
      id: `${LIBRARY_ACTION_PREFIX}flag.${flag}`,
      label,
      scope: 'library',
      defaultBinding: key,
      run: () => commands.setFlag(flag),
    });
  }
  for (const [key, color] of COLOR_LABEL_KEYS) {
    actions.push({
      id: `${LIBRARY_ACTION_PREFIX}label.${color}`,
      label: `Label ${LABEL_NAMES[color]}`,
      scope: 'library',
      defaultBinding: key,
      run: () => commands.setColorLabel(color),
    });
  }
  actions.push({
    id: `${LIBRARY_ACTION_PREFIX}selectAll`,
    label: 'Select all loaded photos',
    scope: 'library',
    defaultBinding: 'Mod+A',
    run: () => commands.selectAll(),
  });
  for (const [key, dx, dy] of MOVES) {
    for (const extend of [false, true]) {
      actions.push({
        id: `${LIBRARY_ACTION_PREFIX}move.${key}${extend ? '.extend' : ''}`,
        label: `${extend ? 'Extend selection' : 'Move selection'}: ${key.replace('Arrow', '').toLowerCase()}`,
        scope: 'library',
        defaultBinding: extend ? `Shift+${key}` : key,
        allowRepeat: true,
        run: () => commands.moveSelection(dx, dy, extend),
      });
    }
  }
  return actions;
}

/**
 * Registers the library actions.
 * @returns a function that removes them again.
 */
export function registerLibraryActions(registry: ShortcutRegistry, commands: LibraryCommands): () => void {
  const removers: Array<() => void> = [];
  const removeAll = (): void => {
    for (const remove of removers) {
      remove();
    }
  };
  try {
    for (const action of libraryActions(commands)) {
      removers.push(registry.register(action));
    }
  } catch (error: unknown) {
    // Do not leave a part of the actions registered.
    removeAll();
    throw error;
  }
  return removeAll;
}
