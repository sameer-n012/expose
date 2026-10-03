/**
 * Central registry of keyboard shortcut actions.
 *
 * Each action has an id, a label, a scope, a default key binding, and a
 * handler. Bindings are data (portable strings, see `keys.ts`). The user
 * can remap them later through `setBinding` and `applyOverrides`; the
 * result of `getOverrides` can be stored as user preferences.
 *
 * Scopes and precedence:
 * - A `global` action is active in all modules.
 * - A module action (scope `library`, `develop`, or `editor`) is active only
 *   while that module is active. It takes precedence over a global action
 *   with the same key binding.
 *
 * Conflicts: two actions in the same scope must not have the same key
 * binding. The registry checks this for macOS and for other platforms at
 * each change, because `Mod` means a different physical key on each. A
 * change that causes a conflict throws `ShortcutConflictError`, and the
 * registry stays unchanged.
 *
 * This module is pure logic. It does not add event listeners. The app
 * calls `handleKeyDown` from its own `keydown` listener.
 */
import type { ModuleId } from '../modules';
import { isEditableTarget } from './editable';
import {
  chordFromEvent,
  chordId,
  formatChord,
  parseBinding,
  type KeyChord,
  type KeyEventLike,
  type Platform,
} from './keys';

/** Where an action is active. */
export type ShortcutScope = 'global' | ModuleId;

/** The definition of one shortcut action. */
export interface ShortcutAction {
  /** Unique, stable id, for example "module.develop". Stored in user preferences. */
  readonly id: string;
  /** Human-readable label for help and preference screens. */
  readonly label: string;
  readonly scope: ShortcutScope;
  /** The default key binding, or null for an action with no default key. */
  readonly defaultBinding: string | null;
  /** If true, the action also fires while focus is in a text field. Default: false. */
  readonly allowInEditable?: boolean;
  /** If true, the action fires again on key auto-repeat. Default: false. */
  readonly allowRepeat?: boolean;
  /** Runs the action. */
  readonly run: () => void;
}

/** One row of the binding list, for a help or preferences screen. */
export interface BindingInfo {
  readonly id: string;
  readonly label: string;
  readonly scope: ShortcutScope;
  /** The current binding (portable string), or null if the action has no key. */
  readonly binding: string | null;
  readonly defaultBinding: string | null;
  /** The current binding formatted for the platform, or null. */
  readonly display: string | null;
  /** True if the user changed the binding from the default. */
  readonly isCustom: boolean;
}

/** User binding changes: action id -> binding string, or null for "no key". */
export type BindingOverrides = Readonly<Record<string, string | null>>;

/** The fields of a `KeyboardEvent` that `handleKeyDown` reads. */
export interface ShortcutKeyEvent extends KeyEventLike {
  readonly target: unknown;
  readonly repeat?: boolean;
  readonly isComposing?: boolean;
  readonly defaultPrevented?: boolean;
}

/** Error for two actions with the same key binding in the same scope. */
export class ShortcutConflictError extends Error {
  constructor(
    readonly actionId: string,
    readonly conflictingActionId: string,
    readonly scope: ShortcutScope,
    readonly binding: string,
    readonly platform: Platform,
  ) {
    super(
      `Shortcut conflict in scope "${scope}": action "${actionId}" and action ` +
        `"${conflictingActionId}" both use "${binding}" (on platform "${platform}").`,
    );
    this.name = 'ShortcutConflictError';
  }
}

/** Error for an incorrect use of the registry (unknown or duplicate id). */
export class ShortcutRegistryError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'ShortcutRegistryError';
  }
}

/** Options for the registry. */
export interface ShortcutRegistryOptions {
  /**
   * Receives errors that action handlers throw. If not set, the error is
   * thrown again from `handleKeyDown`.
   */
  readonly onError?: (error: unknown, action: ShortcutAction) => void;
}

interface Entry {
  readonly action: ShortcutAction;
  readonly binding: string | null;
}

/** Lookup table: scope -> chord id -> action id. */
type ChordIndex = Map<ShortcutScope, Map<string, string>>;

const ALL_PLATFORMS: readonly Platform[] = ['mac', 'other'];

export class ShortcutRegistry {
  readonly platform: Platform;
  private readonly onError: ShortcutRegistryOptions['onError'];
  private entries = new Map<string, Entry>();
  private index: ChordIndex = new Map();

  constructor(platform: Platform, options: ShortcutRegistryOptions = {}) {
    this.platform = platform;
    this.onError = options.onError;
  }

  /**
   * Adds an action with its default binding.
   * @returns a function that removes the action again.
   * @throws ShortcutRegistryError if the id is already registered.
   * @throws ShortcutConflictError if the binding conflicts with another action.
   * @throws KeyBindingParseError if the default binding is not valid.
   */
  register(action: ShortcutAction): () => void {
    if (this.entries.has(action.id)) {
      throw new ShortcutRegistryError(`Shortcut action "${action.id}" is already registered.`);
    }
    const next = new Map(this.entries);
    next.set(action.id, { action, binding: action.defaultBinding });
    this.commit(next);
    return () => {
      // Remove only this exact action, not a later action with the same id.
      if (this.entries.get(action.id)?.action === action) {
        this.unregister(action.id);
      }
    };
  }

  /** Removes an action. Does nothing if the id is not registered. */
  unregister(id: string): void {
    if (!this.entries.has(id)) {
      return;
    }
    const next = new Map(this.entries);
    next.delete(id);
    this.commit(next);
  }

  /** True if an action with this id is registered. */
  has(id: string): boolean {
    return this.entries.has(id);
  }

  /**
   * Changes the binding of one action. Use null to remove the key.
   * @throws ShortcutRegistryError if the id is not registered.
   * @throws ShortcutConflictError or KeyBindingParseError; the registry stays unchanged.
   */
  setBinding(id: string, binding: string | null): void {
    this.applyOverrides({ [id]: binding });
  }

  /** Sets the binding of one action back to its default. */
  resetBinding(id: string): void {
    const entry = this.requireEntry(id);
    this.setBinding(id, entry.action.defaultBinding);
  }

  /** Sets all bindings back to their defaults. */
  resetAll(): void {
    const next = new Map<string, Entry>();
    for (const [id, entry] of this.entries) {
      next.set(id, { action: entry.action, binding: entry.action.defaultBinding });
    }
    this.commit(next);
  }

  /**
   * Applies several binding changes together. If one change is not valid,
   * no change is applied.
   * @throws ShortcutRegistryError if an id is not registered.
   * @throws ShortcutConflictError or KeyBindingParseError.
   */
  applyOverrides(overrides: BindingOverrides): void {
    const next = new Map(this.entries);
    for (const [id, binding] of Object.entries(overrides)) {
      const entry = this.requireEntry(id);
      next.set(id, { action: entry.action, binding });
    }
    this.commit(next);
  }

  /** Returns the bindings that differ from the defaults (data for user preferences). */
  getOverrides(): BindingOverrides {
    const result: Record<string, string | null> = {};
    for (const [id, entry] of this.entries) {
      if (entry.binding !== entry.action.defaultBinding) {
        result[id] = entry.binding;
      }
    }
    return result;
  }

  /** Lists all actions with their bindings, sorted by scope and then by label. */
  listBindings(): BindingInfo[] {
    const scopeOrder = (scope: ShortcutScope): number => (scope === 'global' ? 0 : 1);
    return [...this.entries.values()]
      .map(({ action, binding }): BindingInfo => ({
        id: action.id,
        label: action.label,
        scope: action.scope,
        binding,
        defaultBinding: action.defaultBinding,
        display: binding === null ? null : formatChord(parseBinding(binding, this.platform), this.platform),
        isCustom: binding !== action.defaultBinding,
      }))
      .sort(
        (a, b) =>
          scopeOrder(a.scope) - scopeOrder(b.scope) ||
          a.scope.localeCompare(b.scope) ||
          a.label.localeCompare(b.label),
      );
  }

  /** Returns the current binding of an action, formatted for the platform. */
  displayBinding(id: string): string | null {
    const binding = this.requireEntry(id).binding;
    return binding === null ? null : formatChord(parseBinding(binding, this.platform), this.platform);
  }

  /**
   * Finds the action for a chord in the active module. A module action has
   * precedence over a global action. Returns null if no action matches.
   */
  resolve(chord: KeyChord, activeModule: ModuleId): ShortcutAction | null {
    const key = chordId(chord);
    const id = this.index.get(activeModule)?.get(key) ?? this.index.get('global')?.get(key);
    return id === undefined ? null : (this.entries.get(id)?.action ?? null);
  }

  /**
   * Handles a key event: finds the matching action and runs it.
   * @returns true if an action ran. The caller must then call
   *   `preventDefault()` on the event.
   */
  handleKeyDown(event: ShortcutKeyEvent, activeModule: ModuleId): boolean {
    if (event.defaultPrevented === true || event.isComposing === true) {
      return false;
    }
    const chord = chordFromEvent(event);
    if (chord === null) {
      return false;
    }
    const action = this.resolve(chord, activeModule);
    if (action === null) {
      return false;
    }
    // Check the text field first: a key that the user types must reach the field.
    if (isEditableTarget(event.target) && action.allowInEditable !== true) {
      return false;
    }
    if (event.repeat === true && action.allowRepeat !== true) {
      // Consume the repeat, so that the browser does not act on the key either.
      return true;
    }
    if (this.onError === undefined) {
      action.run();
    } else {
      try {
        action.run();
      } catch (error: unknown) {
        this.onError(error, action);
      }
    }
    return true;
  }

  private requireEntry(id: string): Entry {
    const entry = this.entries.get(id);
    if (entry === undefined) {
      throw new ShortcutRegistryError(`Shortcut action "${id}" is not registered.`);
    }
    return entry;
  }

  /**
   * Validates a full set of entries and makes it the current state.
   * Throws (and keeps the old state) if a binding is not valid or conflicts.
   */
  private commit(next: Map<string, Entry>): void {
    let current: ChordIndex | null = null;
    // Check all platforms, so that a conflict that occurs only on another
    // platform is found during development too.
    for (const platform of ALL_PLATFORMS) {
      const index = buildIndex(next, platform);
      if (platform === this.platform) {
        current = index;
      }
    }
    if (current === null) {
      // Cannot occur: this.platform is always one of ALL_PLATFORMS.
      throw new ShortcutRegistryError(`Unknown platform "${this.platform}".`);
    }
    this.entries = next;
    this.index = current;
  }
}

function buildIndex(entries: ReadonlyMap<string, Entry>, platform: Platform): ChordIndex {
  const index: ChordIndex = new Map();
  for (const [id, { action, binding }] of entries) {
    if (binding === null) {
      continue;
    }
    const key = chordId(parseBinding(binding, platform));
    let scopeMap = index.get(action.scope);
    if (scopeMap === undefined) {
      scopeMap = new Map();
      index.set(action.scope, scopeMap);
    }
    const existing = scopeMap.get(key);
    if (existing !== undefined) {
      throw new ShortcutConflictError(id, existing, action.scope, binding, platform);
    }
    scopeMap.set(key, id);
  }
  return index;
}
