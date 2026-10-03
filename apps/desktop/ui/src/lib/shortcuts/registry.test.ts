import { describe, expect, it, vi } from 'vitest';
import { KeyBindingParseError } from './keys';
import {
  ShortcutConflictError,
  ShortcutRegistry,
  ShortcutRegistryError,
  type ShortcutAction,
  type ShortcutKeyEvent,
  type ShortcutScope,
} from './registry';

function action(
  id: string,
  defaultBinding: string | null,
  scope: ShortcutScope = 'global',
  extra: Partial<ShortcutAction> = {},
): ShortcutAction & { run: ReturnType<typeof vi.fn> } {
  const run = vi.fn();
  return { id, label: id, scope, defaultBinding, ...extra, run };
}

function key(k: string, init: Partial<ShortcutKeyEvent> = {}): ShortcutKeyEvent {
  return {
    key: k,
    code: init.code ?? '',
    ctrlKey: init.ctrlKey ?? false,
    altKey: init.altKey ?? false,
    shiftKey: init.shiftKey ?? false,
    metaKey: init.metaKey ?? false,
    target: init.target ?? null,
    ...(init.repeat === undefined ? {} : { repeat: init.repeat }),
    ...(init.isComposing === undefined ? {} : { isComposing: init.isComposing }),
    ...(init.defaultPrevented === undefined ? {} : { defaultPrevented: init.defaultPrevented }),
  };
}

const TEXT_INPUT = { tagName: 'INPUT', type: 'text' };

describe('ShortcutRegistry registration', () => {
  it('rejects a duplicate id', () => {
    const reg = new ShortcutRegistry('mac');
    reg.register(action('a', 'G'));
    expect(() => reg.register(action('a', 'H'))).toThrow(ShortcutRegistryError);
  });

  it('rejects a conflicting binding in the same scope with a clear error', () => {
    const reg = new ShortcutRegistry('mac');
    reg.register(action('first', 'Mod+Z'));
    let caught: unknown;
    try {
      reg.register(action('second', 'cmd+z'));
    } catch (error: unknown) {
      caught = error;
    }
    expect(caught).toBeInstanceOf(ShortcutConflictError);
    const conflict = caught as ShortcutConflictError;
    expect(conflict.actionId).toBe('second');
    expect(conflict.conflictingActionId).toBe('first');
    expect(conflict.scope).toBe('global');
    expect(conflict.message).toContain('"first"');
    expect(conflict.message).toContain('"second"');
    // The registry is unchanged after the error.
    expect(reg.has('second')).toBe(false);
  });

  it('detects a conflict that occurs only on another platform', () => {
    // On macOS "Mod+Z" is Cmd+Z and "Ctrl+Z" is Control+Z, but on other
    // platforms both are Ctrl+Z.
    const reg = new ShortcutRegistry('mac');
    reg.register(action('undo', 'Mod+Z'));
    expect(() => reg.register(action('other', 'Ctrl+Z'))).toThrow(/platform "other"/);
  });

  it('allows the same binding in different scopes', () => {
    const reg = new ShortcutRegistry('mac');
    reg.register(action('lib', 'R', 'library'));
    reg.register(action('dev', 'R', 'develop'));
    reg.register(action('glob', 'R', 'global'));
    expect(reg.listBindings()).toHaveLength(3);
  });

  it('allows several actions without a binding', () => {
    const reg = new ShortcutRegistry('mac');
    reg.register(action('a', null));
    reg.register(action('b', null));
    expect(reg.listBindings().map((b) => b.binding)).toEqual([null, null]);
  });

  it('rejects a binding that cannot be parsed', () => {
    const reg = new ShortcutRegistry('mac');
    expect(() => reg.register(action('a', 'Hyper+Q'))).toThrow(KeyBindingParseError);
    expect(reg.has('a')).toBe(false);
  });

  it('unregisters with the returned function', () => {
    const reg = new ShortcutRegistry('mac');
    const off = reg.register(action('a', 'G'));
    off();
    expect(reg.has('a')).toBe(false);
    // The key is free again.
    reg.register(action('b', 'G'));
    expect(reg.has('b')).toBe(true);
  });

  it('does not remove a newer action with the same id', () => {
    const reg = new ShortcutRegistry('mac');
    const off = reg.register(action('a', 'G'));
    reg.unregister('a');
    const newer = action('a', 'G');
    reg.register(newer);
    off();
    expect(reg.has('a')).toBe(true);
  });
});

describe('ShortcutRegistry.handleKeyDown', () => {
  it('runs a global action and reports that it handled the event', () => {
    const reg = new ShortcutRegistry('mac');
    const a = action('a', 'G');
    reg.register(a);
    expect(reg.handleKeyDown(key('g'), 'develop')).toBe(true);
    expect(a.run).toHaveBeenCalledOnce();
  });

  it('returns false when no action matches', () => {
    const reg = new ShortcutRegistry('mac');
    reg.register(action('a', 'G'));
    expect(reg.handleKeyDown(key('h'), 'library')).toBe(false);
    expect(reg.handleKeyDown(key('Shift', { shiftKey: true }), 'library')).toBe(false);
  });

  it('distinguishes modifiers', () => {
    const reg = new ShortcutRegistry('mac');
    const plain = action('plain', 'G');
    const shifted = action('shifted', 'Shift+G');
    reg.register(plain);
    reg.register(shifted);
    reg.handleKeyDown(key('G', { shiftKey: true }), 'library');
    expect(shifted.run).toHaveBeenCalledOnce();
    expect(plain.run).not.toHaveBeenCalled();
    expect(reg.handleKeyDown(key('g', { ctrlKey: true }), 'library')).toBe(false);
  });

  it('resolves Mod to Cmd on macOS and to Ctrl elsewhere', () => {
    const mac = new ShortcutRegistry('mac');
    const macUndo = action('undo', 'Mod+Z');
    mac.register(macUndo);
    expect(mac.handleKeyDown(key('z', { ctrlKey: true }), 'editor')).toBe(false);
    expect(mac.handleKeyDown(key('z', { metaKey: true }), 'editor')).toBe(true);

    const other = new ShortcutRegistry('other');
    const otherUndo = action('undo', 'Mod+Z');
    other.register(otherUndo);
    expect(other.handleKeyDown(key('z', { metaKey: true }), 'editor')).toBe(false);
    expect(other.handleKeyDown(key('z', { ctrlKey: true }), 'editor')).toBe(true);
  });

  it('runs module actions only in their module', () => {
    const reg = new ShortcutRegistry('mac');
    const crop = action('develop.crop', 'R', 'develop');
    reg.register(crop);
    expect(reg.handleKeyDown(key('r'), 'library')).toBe(false);
    expect(reg.handleKeyDown(key('r'), 'develop')).toBe(true);
    expect(crop.run).toHaveBeenCalledOnce();
  });

  it('gives module actions precedence over global actions', () => {
    const reg = new ShortcutRegistry('mac');
    const glob = action('glob', 'R');
    const dev = action('dev', 'R', 'develop');
    reg.register(glob);
    reg.register(dev);
    reg.handleKeyDown(key('r'), 'develop');
    expect(dev.run).toHaveBeenCalledOnce();
    expect(glob.run).not.toHaveBeenCalled();
    reg.handleKeyDown(key('r'), 'library');
    expect(glob.run).toHaveBeenCalledOnce();
  });

  it('ignores keys typed in a text field', () => {
    const reg = new ShortcutRegistry('mac');
    const a = action('a', 'G');
    reg.register(a);
    expect(reg.handleKeyDown(key('g', { target: TEXT_INPUT }), 'library')).toBe(false);
    // Also for auto-repeat, so that the field gets the repeated characters.
    expect(reg.handleKeyDown(key('g', { target: TEXT_INPUT, repeat: true }), 'library')).toBe(false);
    expect(a.run).not.toHaveBeenCalled();
  });

  it('runs an action in a text field if the binding allows it', () => {
    const reg = new ShortcutRegistry('mac');
    const save = action('save', 'Mod+S', 'global', { allowInEditable: true });
    reg.register(save);
    expect(reg.handleKeyDown(key('s', { metaKey: true, target: TEXT_INPUT }), 'library')).toBe(true);
    expect(save.run).toHaveBeenCalledOnce();
  });

  it('consumes auto-repeat without running the action, unless allowed', () => {
    const reg = new ShortcutRegistry('mac');
    const toggle = action('toggle', 'Tab');
    const zoom = action('zoom', 'Mod+=', 'global', { allowRepeat: true });
    reg.register(toggle);
    reg.register(zoom);
    expect(reg.handleKeyDown(key('Tab', { repeat: true }), 'library')).toBe(true);
    expect(toggle.run).not.toHaveBeenCalled();
    reg.handleKeyDown(key('=', { metaKey: true, repeat: true }), 'library');
    expect(zoom.run).toHaveBeenCalledOnce();
  });

  it('ignores composing and already handled events', () => {
    const reg = new ShortcutRegistry('mac');
    const a = action('a', 'G');
    reg.register(a);
    expect(reg.handleKeyDown(key('g', { isComposing: true }), 'library')).toBe(false);
    expect(reg.handleKeyDown(key('g', { defaultPrevented: true }), 'library')).toBe(false);
    expect(a.run).not.toHaveBeenCalled();
  });

  it('sends handler errors to onError', () => {
    const onError = vi.fn();
    const reg = new ShortcutRegistry('mac', { onError });
    const failing = action('a', 'G');
    failing.run.mockImplementation(() => {
      throw new Error('boom');
    });
    reg.register(failing);
    expect(reg.handleKeyDown(key('g'), 'library')).toBe(true);
    expect(onError).toHaveBeenCalledWith(expect.objectContaining({ message: 'boom' }), failing);
  });

  it('throws handler errors when there is no onError', () => {
    const reg = new ShortcutRegistry('mac');
    const failing = action('a', 'G');
    failing.run.mockImplementation(() => {
      throw new Error('boom');
    });
    reg.register(failing);
    expect(() => reg.handleKeyDown(key('g'), 'library')).toThrow('boom');
  });
});

describe('ShortcutRegistry remapping', () => {
  it('changes a binding and frees the old key', () => {
    const reg = new ShortcutRegistry('mac');
    const a = action('a', 'G');
    reg.register(a);
    reg.setBinding('a', 'H');
    expect(reg.handleKeyDown(key('g'), 'library')).toBe(false);
    expect(reg.handleKeyDown(key('h'), 'library')).toBe(true);
    expect(a.run).toHaveBeenCalledOnce();
  });

  it('removes a binding with null', () => {
    const reg = new ShortcutRegistry('mac');
    reg.register(action('a', 'G'));
    reg.setBinding('a', null);
    expect(reg.handleKeyDown(key('g'), 'library')).toBe(false);
    expect(reg.displayBinding('a')).toBeNull();
  });

  it('rejects a conflicting remap and keeps the old state', () => {
    const reg = new ShortcutRegistry('mac');
    reg.register(action('a', 'G'));
    reg.register(action('b', 'H'));
    expect(() => reg.setBinding('b', 'G')).toThrow(ShortcutConflictError);
    expect(reg.getOverrides()).toEqual({});
    expect(reg.handleKeyDown(key('h'), 'library')).toBe(true);
  });

  it('rejects an unknown id', () => {
    const reg = new ShortcutRegistry('mac');
    expect(() => reg.setBinding('missing', 'G')).toThrow(ShortcutRegistryError);
    expect(() => reg.applyOverrides({ missing: 'G' })).toThrow(ShortcutRegistryError);
  });

  it('applies overrides together, so that a swap of two keys works', () => {
    const reg = new ShortcutRegistry('mac');
    reg.register(action('a', 'G'));
    reg.register(action('b', 'H'));
    reg.applyOverrides({ a: 'H', b: 'G' });
    expect(reg.getOverrides()).toEqual({ a: 'H', b: 'G' });
  });

  it('applies no override if one override is not valid', () => {
    const reg = new ShortcutRegistry('mac');
    reg.register(action('a', 'G'));
    reg.register(action('b', 'H'));
    expect(() => reg.applyOverrides({ a: 'J', b: 'Bad+Key' })).toThrow(KeyBindingParseError);
    expect(reg.getOverrides()).toEqual({});
  });

  it('round-trips overrides as data', () => {
    const reg = new ShortcutRegistry('mac');
    reg.register(action('a', 'G'));
    reg.register(action('b', 'H'));
    reg.setBinding('a', 'Mod+G');
    reg.setBinding('b', null);
    const saved = JSON.parse(JSON.stringify(reg.getOverrides())) as Record<string, string | null>;

    const fresh = new ShortcutRegistry('other');
    fresh.register(action('a', 'G'));
    fresh.register(action('b', 'H'));
    fresh.applyOverrides(saved);
    expect(fresh.displayBinding('a')).toBe('Ctrl+G');
    expect(fresh.displayBinding('b')).toBeNull();
  });

  it('resets one binding or all bindings to the defaults', () => {
    const reg = new ShortcutRegistry('mac');
    reg.register(action('a', 'G'));
    reg.register(action('b', 'H'));
    reg.applyOverrides({ a: 'J', b: 'K' });
    reg.resetBinding('a');
    expect(reg.getOverrides()).toEqual({ b: 'K' });
    reg.resetAll();
    expect(reg.getOverrides()).toEqual({});
  });
});

describe('ShortcutRegistry.listBindings', () => {
  it('lists all actions with display strings, global scope first', () => {
    const reg = new ShortcutRegistry('mac');
    reg.register(action('dev.crop', 'R', 'develop', { label: 'Crop' }));
    reg.register(action('undo', 'Mod+Z', 'global', { label: 'Undo' }));
    reg.register(action('about', null, 'global', { label: 'About' }));
    reg.setBinding('dev.crop', 'Shift+R');

    expect(reg.listBindings()).toEqual([
      {
        id: 'about',
        label: 'About',
        scope: 'global',
        binding: null,
        defaultBinding: null,
        display: null,
        isCustom: false,
      },
      {
        id: 'undo',
        label: 'Undo',
        scope: 'global',
        binding: 'Mod+Z',
        defaultBinding: 'Mod+Z',
        display: '⌘Z',
        isCustom: false,
      },
      {
        id: 'dev.crop',
        label: 'Crop',
        scope: 'develop',
        binding: 'Shift+R',
        defaultBinding: 'R',
        display: '⇧R',
        isCustom: true,
      },
    ]);
  });
});
