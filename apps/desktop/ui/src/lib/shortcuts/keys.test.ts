import { describe, expect, it } from 'vitest';
import {
  chordFromEvent,
  chordId,
  detectPlatform,
  formatChord,
  KeyBindingParseError,
  normalizeKey,
  parseBinding,
  type KeyEventLike,
} from './keys';

function ev(key: string, init: Partial<KeyEventLike> = {}): KeyEventLike {
  return {
    key,
    code: init.code ?? '',
    ctrlKey: init.ctrlKey ?? false,
    altKey: init.altKey ?? false,
    shiftKey: init.shiftKey ?? false,
    metaKey: init.metaKey ?? false,
  };
}

describe('normalizeKey', () => {
  it('lowercases single characters', () => {
    expect(normalizeKey('G')).toBe('g');
    expect(normalizeKey('?')).toBe('?');
  });

  it('gives named keys their canonical case', () => {
    expect(normalizeKey('tab')).toBe('Tab');
    expect(normalizeKey('ARROWUP')).toBe('ArrowUp');
    expect(normalizeKey('f12')).toBe('F12');
  });

  it('resolves aliases', () => {
    expect(normalizeKey('Esc')).toBe('Escape');
    expect(normalizeKey('Space')).toBe(' ');
    expect(normalizeKey('Plus')).toBe('+');
  });

  it('returns null for unknown names', () => {
    expect(normalizeKey('Banana')).toBeNull();
    expect(normalizeKey('Dead')).toBeNull();
    expect(normalizeKey('')).toBeNull();
  });
});

describe('parseBinding', () => {
  it('maps Mod to Meta on macOS', () => {
    expect(parseBinding('Mod+Z', 'mac')).toEqual({ ctrl: false, alt: false, shift: false, meta: true, key: 'z' });
  });

  it('maps Mod to Ctrl on other platforms', () => {
    expect(parseBinding('Mod+Z', 'other')).toEqual({ ctrl: true, alt: false, shift: false, meta: false, key: 'z' });
  });

  it('accepts modifiers in any order and case', () => {
    expect(chordId(parseBinding('shift+mod+alt+z', 'other'))).toBe('Ctrl+Alt+Shift+z');
    expect(chordId(parseBinding('Alt+Shift+Mod+Z', 'other'))).toBe('Ctrl+Alt+Shift+z');
  });

  it('accepts modifier aliases', () => {
    expect(chordId(parseBinding('Cmd+Option+K', 'mac'))).toBe('Alt+Meta+k');
  });

  it('ignores Shift for symbol keys', () => {
    expect(chordId(parseBinding('Shift+?', 'mac'))).toBe('?');
    expect(chordId(parseBinding('?', 'mac'))).toBe('?');
  });

  it('keeps Shift for letters and named keys', () => {
    expect(chordId(parseBinding('Shift+G', 'mac'))).toBe('Shift+g');
    expect(chordId(parseBinding('Shift+Tab', 'mac'))).toBe('Shift+Tab');
  });

  it('accepts "+" as the key', () => {
    expect(chordId(parseBinding('Mod++', 'other'))).toBe('Ctrl++');
    expect(chordId(parseBinding('Mod+Plus', 'other'))).toBe('Ctrl++');
    expect(chordId(parseBinding('+', 'other'))).toBe('+');
  });

  it('accepts Space', () => {
    expect(chordId(parseBinding('Space', 'mac'))).toBe('Space');
  });

  it.each([
    ['', 'the key is missing'],
    ['Mod+', 'the key is missing'],
    ['Hyper+Z', 'not a modifier'],
    ['Shift+Shift+Z', 'more than once'],
    ['Mod+Banana', 'not a known key'],
  ])('rejects %j', (binding, reason) => {
    expect(() => parseBinding(binding, 'mac')).toThrow(KeyBindingParseError);
    expect(() => parseBinding(binding, 'mac')).toThrow(reason);
  });

  it('rejects Mod together with the same physical modifier', () => {
    expect(() => parseBinding('Mod+Meta+Z', 'mac')).toThrow(KeyBindingParseError);
    expect(() => parseBinding('Mod+Ctrl+Z', 'other')).toThrow(KeyBindingParseError);
    // On the other platform these are two different physical keys.
    expect(chordId(parseBinding('Mod+Meta+Z', 'other'))).toBe('Ctrl+Meta+z');
    expect(chordId(parseBinding('Mod+Ctrl+Z', 'mac'))).toBe('Ctrl+Meta+z');
  });
});

describe('chordFromEvent', () => {
  it('normalizes a plain letter', () => {
    expect(chordId(chordFromEvent(ev('g')) ?? fail())).toBe('g');
  });

  it('keeps Shift for an uppercase letter', () => {
    expect(chordId(chordFromEvent(ev('G', { shiftKey: true })) ?? fail())).toBe('Shift+g');
  });

  it('drops Shift for a shifted symbol', () => {
    expect(chordId(chordFromEvent(ev('?', { shiftKey: true })) ?? fail())).toBe('?');
  });

  it('uses the physical code when Alt changes the character', () => {
    // Option+E on a macOS US layout gives a dead key.
    expect(chordId(chordFromEvent(ev('Dead', { altKey: true, code: 'KeyE' })) ?? fail())).toBe('Alt+e');
    // Option+G gives the copyright sign.
    expect(chordId(chordFromEvent(ev('©', { altKey: true, code: 'KeyG' })) ?? fail())).toBe('Alt+g');
    expect(chordId(chordFromEvent(ev('¡', { altKey: true, code: 'Digit1' })) ?? fail())).toBe('Alt+1');
  });

  it('returns null for a modifier key alone', () => {
    expect(chordFromEvent(ev('Shift', { shiftKey: true }))).toBeNull();
    expect(chordFromEvent(ev('Meta', { metaKey: true }))).toBeNull();
    expect(chordFromEvent(ev('Alt', { altKey: true, code: 'AltLeft' }))).toBeNull();
  });

  it('returns null for a dead key without Alt', () => {
    expect(chordFromEvent(ev('Dead'))).toBeNull();
  });

  it('matches the chord of the binding', () => {
    const fromEvent = chordFromEvent(ev('z', { metaKey: true, shiftKey: true }));
    expect(chordId(fromEvent ?? fail())).toBe(chordId(parseBinding('Mod+Shift+Z', 'mac')));
  });
});

describe('formatChord', () => {
  it('uses symbols on macOS', () => {
    expect(formatChord(parseBinding('Mod+Shift+Z', 'mac'), 'mac')).toBe('⇧⌘Z');
    expect(formatChord(parseBinding('Ctrl+Alt+Tab', 'mac'), 'mac')).toBe('⌃⌥⇥');
  });

  it('uses names on other platforms', () => {
    expect(formatChord(parseBinding('Mod+Shift+Z', 'other'), 'other')).toBe('Ctrl+Shift+Z');
    expect(formatChord(parseBinding('Space', 'other'), 'other')).toBe('Space');
    expect(formatChord(parseBinding('G', 'other'), 'other')).toBe('G');
  });
});

describe('detectPlatform', () => {
  it('detects macOS', () => {
    expect(detectPlatform({ platform: 'MacIntel', userAgent: '' })).toBe('mac');
    expect(detectPlatform({ userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)' })).toBe('mac');
  });

  it('uses "other" for other platforms and for no navigator', () => {
    expect(detectPlatform({ platform: 'Win32', userAgent: 'Windows NT 10.0' })).toBe('other');
    expect(detectPlatform({ platform: 'Linux x86_64' })).toBe('other');
    expect(detectPlatform(undefined)).toBe('other');
  });
});

function fail(): never {
  throw new Error('Expected a chord, got null.');
}
