/** Public API of the keyboard shortcut system. */
export { isEditableTarget } from './editable';
export {
  chordFromEvent,
  chordId,
  detectPlatform,
  formatChord,
  KeyBindingParseError,
  normalizeKey,
  parseBinding,
  type KeyChord,
  type KeyEventLike,
  type Platform,
} from './keys';
export {
  ShortcutConflictError,
  ShortcutRegistry,
  ShortcutRegistryError,
  type BindingInfo,
  type BindingOverrides,
  type ShortcutAction,
  type ShortcutKeyEvent,
  type ShortcutRegistryOptions,
  type ShortcutScope,
} from './registry';
