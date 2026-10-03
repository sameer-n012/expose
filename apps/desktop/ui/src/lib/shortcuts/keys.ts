/**
 * Key binding parser and key event normalizer.
 *
 * A key binding is a portable string, for example "Mod+Shift+Z", "G", or
 * "Shift+Tab". Tokens are separated by "+". The last token is the key. The
 * other tokens are modifiers:
 *
 * - `Mod`   the primary modifier of the platform: Cmd on macOS, Ctrl elsewhere.
 * - `Ctrl`  the physical Control key.
 * - `Alt`   Alt (Option on macOS).
 * - `Shift` Shift.
 * - `Meta`  Cmd on macOS, the Windows key elsewhere.
 *
 * A binding is resolved for one platform into a `KeyChord` (physical
 * modifiers + normalized key). Two bindings conflict on a platform when their
 * chords have the same canonical string.
 *
 * Rules for the key token:
 * - Letters are case-insensitive ("g" and "G" are the same key).
 * - Named keys use the `KeyboardEvent.key` names (Tab, Enter, Escape,
 *   ArrowUp, F1, ...). Some aliases are accepted (Esc, Space, Plus, Del).
 * - For a printable symbol that is not a letter (for example "?" or "["),
 *   Shift is ignored. The symbol itself already tells which key the user
 *   pressed, and the Shift state for a symbol depends on the keyboard layout.
 *   Digits are symbols too: bind "!" (not "Shift+1") for Shift and the 1 key
 *   on a US layout.
 */

/** The platform family. It decides what `Mod` means and how keys display. */
export type Platform = 'mac' | 'other';

/** A key binding resolved to physical modifiers for one platform. */
export interface KeyChord {
  readonly ctrl: boolean;
  readonly alt: boolean;
  readonly shift: boolean;
  readonly meta: boolean;
  /** Normalized key: a lowercase single character or a named key. */
  readonly key: string;
}

/** The fields of a `KeyboardEvent` that the normalizer reads. */
export interface KeyEventLike {
  readonly key: string;
  readonly code: string;
  readonly ctrlKey: boolean;
  readonly altKey: boolean;
  readonly shiftKey: boolean;
  readonly metaKey: boolean;
}

/** Error for a key binding string that cannot be parsed. */
export class KeyBindingParseError extends Error {
  constructor(
    readonly binding: string,
    reason: string,
  ) {
    super(`Invalid key binding "${binding}": ${reason}`);
    this.name = 'KeyBindingParseError';
  }
}

type Modifier = 'mod' | 'ctrl' | 'alt' | 'shift' | 'meta';

const MODIFIER_ALIASES: Readonly<Record<string, Modifier>> = {
  mod: 'mod',
  ctrl: 'ctrl',
  control: 'ctrl',
  alt: 'alt',
  option: 'alt',
  opt: 'alt',
  shift: 'shift',
  meta: 'meta',
  cmd: 'meta',
  command: 'meta',
};

/** Named keys, keyed by lowercase name. Values are `KeyboardEvent.key` names. */
const NAMED_KEYS: ReadonlyMap<string, string> = new Map(
  [
    'Tab',
    'Enter',
    'Escape',
    'Backspace',
    'Delete',
    'Insert',
    'Home',
    'End',
    'PageUp',
    'PageDown',
    'ArrowUp',
    'ArrowDown',
    'ArrowLeft',
    'ArrowRight',
    ...Array.from({ length: 24 }, (_, i) => `F${i + 1}`),
  ].map((name) => [name.toLowerCase(), name]),
);

const KEY_ALIASES: Readonly<Record<string, string>> = {
  esc: 'Escape',
  return: 'Enter',
  del: 'Delete',
  space: ' ',
  spacebar: ' ',
  plus: '+',
  up: 'ArrowUp',
  down: 'ArrowDown',
  left: 'ArrowLeft',
  right: 'ArrowRight',
};

/** Keys from `KeyboardEvent.key` that are modifiers alone. They never form a chord. */
const MODIFIER_KEY_NAMES: ReadonlySet<string> = new Set([
  'Shift',
  'Control',
  'Alt',
  'AltGraph',
  'Meta',
  'OS',
  'CapsLock',
  'Fn',
]);

/**
 * Normalizes a key name. Returns null if the name is not a known key.
 * Single characters are lowercased. Named keys get their canonical case.
 */
export function normalizeKey(raw: string): string | null {
  // Do not use `raw.length`: it counts UTF-16 units, not characters.
  if ([...raw].length === 1) {
    return raw.toLowerCase();
  }
  const lower = raw.toLowerCase();
  return KEY_ALIASES[lower] ?? NAMED_KEYS.get(lower) ?? null;
}

/** True if the normalized key is one printable character that is not a letter. */
function isSymbolKey(key: string): boolean {
  return [...key].length === 1 && key.toLowerCase() === key.toUpperCase() && key !== ' ';
}

/**
 * Parses a binding string and resolves it for a platform.
 * @throws KeyBindingParseError if the string is not a valid binding.
 */
export function parseBinding(binding: string, platform: Platform): KeyChord {
  // "+" can be the key itself ("Mod++" or "Mod+Plus"), so do not split blindly.
  let tokens: string[];
  if (binding === '+') {
    tokens = ['+'];
  } else if (binding.endsWith('++')) {
    tokens = [...binding.slice(0, -2).split('+'), '+'];
  } else {
    tokens = binding.split('+');
  }
  const trimmed = tokens.map((t) => t.trim());
  const keyToken = trimmed.pop();
  if (keyToken === undefined || keyToken === '') {
    throw new KeyBindingParseError(binding, 'the key is missing');
  }

  const mods = new Set<Modifier>();
  for (const token of trimmed) {
    const modifier = MODIFIER_ALIASES[token.toLowerCase()];
    if (modifier === undefined) {
      throw new KeyBindingParseError(binding, `"${token}" is not a modifier`);
    }
    if (mods.has(modifier)) {
      throw new KeyBindingParseError(binding, `modifier "${token}" occurs more than once`);
    }
    mods.add(modifier);
  }

  const key = normalizeKey(keyToken);
  if (key === null) {
    throw new KeyBindingParseError(binding, `"${keyToken}" is not a known key`);
  }

  const mod = mods.has('mod');
  const ctrl = mods.has('ctrl') || (mod && platform === 'other');
  const meta = mods.has('meta') || (mod && platform === 'mac');
  if (mod && ((platform === 'other' && mods.has('ctrl')) || (platform === 'mac' && mods.has('meta')))) {
    throw new KeyBindingParseError(binding, '"Mod" and the same physical modifier occur together');
  }

  return {
    ctrl,
    alt: mods.has('alt'),
    shift: mods.has('shift') && !isSymbolKey(key),
    meta,
    key,
  };
}

/**
 * Converts a keyboard event into a chord. Returns null for an event that
 * is a modifier key alone, or that has no usable key.
 */
export function chordFromEvent(event: KeyEventLike): KeyChord | null {
  if (MODIFIER_KEY_NAMES.has(event.key)) {
    return null;
  }
  // On macOS, Option changes letters into other characters ("å") or dead
  // keys. For letters and digits, the physical code is then the reliable
  // source for the key.
  let key = event.altKey ? keyFromCode(event.code) : null;
  // "Dead" and "Unidentified" are not known keys, so this gives null for them.
  key ??= normalizeKey(event.key);
  if (key === null) {
    return null;
  }

  return {
    ctrl: event.ctrlKey,
    alt: event.altKey,
    shift: event.shiftKey && !isSymbolKey(key),
    meta: event.metaKey,
    key,
  };
}

/** Maps a `KeyboardEvent.code` for a letter or digit to its key. Returns null otherwise. */
function keyFromCode(code: string): string | null {
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter?.[1] !== undefined) {
    return letter[1].toLowerCase();
  }
  const digit = /^Digit([0-9])$/.exec(code);
  if (digit?.[1] !== undefined) {
    return digit[1];
  }
  return null;
}

/** Canonical string of a chord. Equal strings mean the same physical key press. */
export function chordId(chord: KeyChord): string {
  return [
    chord.ctrl ? 'Ctrl' : '',
    chord.alt ? 'Alt' : '',
    chord.shift ? 'Shift' : '',
    chord.meta ? 'Meta' : '',
    chord.key === ' ' ? 'Space' : chord.key,
  ]
    .filter((part) => part !== '')
    .join('+');
}

const MAC_KEY_SYMBOLS: Readonly<Record<string, string>> = {
  Enter: '↩',
  Escape: '⎋',
  Backspace: '⌫',
  Delete: '⌦',
  Tab: '⇥',
  ArrowUp: '↑',
  ArrowDown: '↓',
  ArrowLeft: '←',
  ArrowRight: '→',
  ' ': 'Space',
};

function displayKey(key: string, platform: Platform): string {
  if (platform === 'mac') {
    const symbol = MAC_KEY_SYMBOLS[key];
    if (symbol !== undefined) {
      return symbol;
    }
  }
  if (key === ' ') {
    return 'Space';
  }
  return [...key].length === 1 ? key.toUpperCase() : key;
}

/**
 * Formats a chord for display in the UI, in the convention of the platform.
 * macOS: "⇧⌘Z". Other platforms: "Ctrl+Shift+Z".
 */
export function formatChord(chord: KeyChord, platform: Platform): string {
  const key = displayKey(chord.key, platform);
  if (platform === 'mac') {
    // macOS order: Control, Option, Shift, Command.
    return `${chord.ctrl ? '⌃' : ''}${chord.alt ? '⌥' : ''}${chord.shift ? '⇧' : ''}${chord.meta ? '⌘' : ''}${key}`;
  }
  return [chord.ctrl ? 'Ctrl' : '', chord.alt ? 'Alt' : '', chord.shift ? 'Shift' : '', chord.meta ? 'Win' : '', key]
    .filter((part) => part !== '')
    .join('+');
}

/** Detects the platform family from the browser environment. */
export function detectPlatform(nav: { platform?: string; userAgent?: string } | undefined): Platform {
  if (nav === undefined) {
    return 'other';
  }
  const source = `${nav.platform ?? ''} ${nav.userAgent ?? ''}`;
  return /Mac|iPhone|iPad|iPod/i.test(source) ? 'mac' : 'other';
}
