import { describe, expect, it, vi } from 'vitest';
import { appActions } from '../appActions';
import { ShortcutRegistry, type ShortcutKeyEvent } from '../shortcuts/registry';
import { CROP_ACTION, cropModeActions, cropToggleAction, registerActions, type CropCommands } from './cropActions';

function commands(): CropCommands {
  return {
    toggleCrop: vi.fn(),
    commitCrop: vi.fn(),
    cancelCrop: vi.fn(),
    cycleCropOverlay: vi.fn(),
    swapCropOrientation: vi.fn(),
  };
}

const key = (k: string): ShortcutKeyEvent => ({
  key: k,
  code: '',
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  target: null,
});

describe('crop actions', () => {
  it('registers next to the shell actions without a conflict', () => {
    const registry = new ShortcutRegistry('mac');
    const fake = { switchTo: vi.fn(), showLibraryGrid: vi.fn() };
    const layout = { toggleSides: vi.fn(), toggleAll: vi.fn() };
    for (const action of appActions({ modules: fake as never, layout: layout as never })) {
      registry.register(action);
    }
    const c = commands();
    registry.register(cropToggleAction(c));
    const remove = registerActions(registry, cropModeActions(c));
    expect(registry.displayBinding(CROP_ACTION.toggle)).toBe('R');

    expect(registry.handleKeyDown(key('r'), 'develop')).toBe(true);
    expect(c.toggleCrop).toHaveBeenCalledOnce();
    // Not active in the Library.
    expect(registry.handleKeyDown(key('r'), 'library')).toBe(false);
    registry.handleKeyDown(key('Enter'), 'develop');
    registry.handleKeyDown(key('Escape'), 'develop');
    registry.handleKeyDown(key('o'), 'develop');
    registry.handleKeyDown(key('x'), 'develop');
    expect(c.commitCrop).toHaveBeenCalledOnce();
    expect(c.cancelCrop).toHaveBeenCalledOnce();
    expect(c.cycleCropOverlay).toHaveBeenCalledOnce();
    expect(c.swapCropOrientation).toHaveBeenCalledOnce();

    remove();
    expect(registry.has(CROP_ACTION.commit)).toBe(false);
    expect(registry.handleKeyDown(key('Enter'), 'develop')).toBe(false);
  });

  it('registers none of the actions when one conflicts', () => {
    const registry = new ShortcutRegistry('mac');
    registry.register({ id: 'other', label: 'Other', scope: 'develop', defaultBinding: 'X', run: vi.fn() });
    expect(() => registerActions(registry, cropModeActions(commands()))).toThrow();
    expect(registry.has(CROP_ACTION.commit)).toBe(false);
  });
});
