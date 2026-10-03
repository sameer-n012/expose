import { describe, expect, it } from 'vitest';
import type { ShortcutKeyEvent } from '../shortcuts/registry';
import { ShortcutRegistry } from '../shortcuts/registry';
import { registerLibraryActions, type LibraryCommands } from './libraryActions';

function setup() {
  const calls: string[] = [];
  const commands: LibraryCommands = {
    setRating: (n) => calls.push(`rating:${n}`),
    setFlag: (f) => calls.push(`flag:${f}`),
    setColorLabel: (l) => calls.push(`label:${l}`),
    selectAll: () => calls.push('selectAll'),
    moveSelection: (dx, dy, extend) => calls.push(`move:${dx},${dy},${extend}`),
  };
  const registry = new ShortcutRegistry('mac');
  registerLibraryActions(registry, commands);
  return { calls, registry };
}

function key(k: string, init: Partial<ShortcutKeyEvent> = {}): ShortcutKeyEvent {
  return {
    key: k,
    code: '',
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    target: null,
    ...init,
  };
}

describe('library shortcut actions', () => {
  it('maps 0 to 5 to ratings', () => {
    const { calls, registry } = setup();
    for (const n of ['0', '1', '2', '3', '4', '5']) {
      expect(registry.handleKeyDown(key(n), 'library')).toBe(true);
    }
    expect(calls).toEqual(['rating:0', 'rating:1', 'rating:2', 'rating:3', 'rating:4', 'rating:5']);
  });

  it('maps P, X, and U to flags', () => {
    const { calls, registry } = setup();
    for (const k of ['p', 'x', 'u']) {
      registry.handleKeyDown(key(k), 'library');
    }
    expect(calls).toEqual(['flag:pick', 'flag:reject', 'flag:none']);
  });

  it('maps 6 to 9 to color labels, and gives purple no key', () => {
    const { calls, registry } = setup();
    for (const k of ['6', '7', '8', '9']) {
      registry.handleKeyDown(key(k), 'library');
    }
    expect(calls).toEqual(['label:red', 'label:yellow', 'label:green', 'label:blue']);
    expect(registry.listBindings().some((b) => b.id.endsWith('label.purple'))).toBe(false);
  });

  it('maps Cmd+A to select all on macOS, and Ctrl+A elsewhere', () => {
    const mac = setup();
    mac.registry.handleKeyDown(key('a', { metaKey: true }), 'library');
    expect(mac.calls).toEqual(['selectAll']);

    const calls: string[] = [];
    const other = new ShortcutRegistry('other');
    registerLibraryActions(other, {
      setRating: () => undefined,
      setFlag: () => undefined,
      setColorLabel: () => undefined,
      selectAll: () => calls.push('selectAll'),
      moveSelection: () => undefined,
    });
    other.handleKeyDown(key('a', { ctrlKey: true }), 'library');
    expect(calls).toEqual(['selectAll']);
  });

  it('maps the arrow keys to moves, and Shift+arrow to extend', () => {
    const { calls, registry } = setup();
    registry.handleKeyDown(key('ArrowLeft'), 'library');
    registry.handleKeyDown(key('ArrowDown'), 'library');
    registry.handleKeyDown(key('ArrowRight', { shiftKey: true }), 'library');
    registry.handleKeyDown(key('ArrowUp', { shiftKey: true }), 'library');
    expect(calls).toEqual(['move:-1,0,false', 'move:0,1,false', 'move:1,0,true', 'move:0,-1,true']);
  });

  it('lets a held arrow key repeat, but not a held rating key', () => {
    const { calls, registry } = setup();
    registry.handleKeyDown(key('ArrowRight', { repeat: true }), 'library');
    registry.handleKeyDown(key('3', { repeat: true }), 'library');
    expect(calls).toEqual(['move:1,0,false']);
  });

  it('does nothing in the other modules', () => {
    const { calls, registry } = setup();
    expect(registry.handleKeyDown(key('3'), 'develop')).toBe(false);
    expect(registry.handleKeyDown(key('p'), 'editor')).toBe(false);
    expect(calls).toEqual([]);
  });

  it('does nothing while the user types in a text field', () => {
    const { calls, registry } = setup();
    const input = { tagName: 'INPUT', type: 'search' };
    expect(registry.handleKeyDown(key('3', { target: input }), 'library')).toBe(false);
    expect(registry.handleKeyDown(key('a', { metaKey: true, target: input }), 'library')).toBe(false);
    expect(calls).toEqual([]);
  });

  it('removes all actions with the returned function', () => {
    const registry = new ShortcutRegistry('mac');
    const remove = registerLibraryActions(registry, {
      setRating: () => undefined,
      setFlag: () => undefined,
      setColorLabel: () => undefined,
      selectAll: () => undefined,
      moveSelection: () => undefined,
    });
    expect(registry.listBindings().length).toBeGreaterThan(0);
    remove();
    expect(registry.listBindings()).toEqual([]);
  });
});
