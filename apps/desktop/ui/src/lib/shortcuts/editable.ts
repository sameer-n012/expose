/**
 * Detects event targets where the user types text.
 *
 * Shortcuts must not fire while the user types in such an element, unless
 * the binding allows it. The check uses duck typing (no `instanceof
 * HTMLElement`), so that it works in tests without a DOM.
 */

/** The element fields that the check reads. */
export interface EditableCandidate {
  readonly tagName?: string;
  readonly type?: string;
  readonly isContentEditable?: boolean;
  readonly readOnly?: boolean;
}

/** Input types that do not accept typed text. Keys in these inputs can fire shortcuts. */
const NON_TEXT_INPUT_TYPES: ReadonlySet<string> = new Set([
  'button',
  'checkbox',
  'color',
  'file',
  'hidden',
  'image',
  'radio',
  'range',
  'reset',
  'submit',
]);

/** True if the target is an element that accepts typed text. */
export function isEditableTarget(target: unknown): boolean {
  if (typeof target !== 'object' || target === null) {
    return false;
  }
  const el = target as EditableCandidate;
  if (el.isContentEditable === true) {
    return true;
  }
  const tag = el.tagName?.toUpperCase();
  if (tag === 'TEXTAREA') {
    return el.readOnly !== true;
  }
  if (tag === 'SELECT') {
    // Letter keys select options in a focused select element.
    return true;
  }
  if (tag === 'INPUT') {
    const type = (el.type ?? 'text').toLowerCase();
    return !NON_TEXT_INPUT_TYPES.has(type) && el.readOnly !== true;
  }
  return false;
}
