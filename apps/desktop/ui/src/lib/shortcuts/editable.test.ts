import { describe, expect, it } from 'vitest';
import { isEditableTarget } from './editable';

describe('isEditableTarget', () => {
  it('detects text inputs and text areas', () => {
    expect(isEditableTarget({ tagName: 'INPUT', type: 'text' })).toBe(true);
    expect(isEditableTarget({ tagName: 'INPUT' })).toBe(true);
    expect(isEditableTarget({ tagName: 'input', type: 'search' })).toBe(true);
    expect(isEditableTarget({ tagName: 'INPUT', type: 'number' })).toBe(true);
    expect(isEditableTarget({ tagName: 'TEXTAREA' })).toBe(true);
    expect(isEditableTarget({ tagName: 'SELECT' })).toBe(true);
  });

  it('detects content-editable elements', () => {
    expect(isEditableTarget({ tagName: 'DIV', isContentEditable: true })).toBe(true);
  });

  it('ignores inputs that do not accept text', () => {
    for (const type of ['checkbox', 'radio', 'range', 'button', 'color']) {
      expect(isEditableTarget({ tagName: 'INPUT', type })).toBe(false);
    }
  });

  it('ignores read-only fields', () => {
    expect(isEditableTarget({ tagName: 'INPUT', type: 'text', readOnly: true })).toBe(false);
    expect(isEditableTarget({ tagName: 'TEXTAREA', readOnly: true })).toBe(false);
  });

  it('ignores other elements and non-elements', () => {
    expect(isEditableTarget({ tagName: 'BUTTON' })).toBe(false);
    expect(isEditableTarget({ tagName: 'BODY', isContentEditable: false })).toBe(false);
    expect(isEditableTarget(null)).toBe(false);
    expect(isEditableTarget(undefined)).toBe(false);
    expect(isEditableTarget('INPUT')).toBe(false);
  });
});
