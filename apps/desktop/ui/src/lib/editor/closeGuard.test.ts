import { describe, expect, it, vi } from 'vitest';
import { guardWindowClose, type CloseEvent, type CloseListener } from './closeGuard';

const tick = () => new Promise((r) => setTimeout(r, 0));

function fakeWindow() {
  let handler: ((event: CloseEvent) => Promise<void>) | null = null;
  const unlisten = vi.fn(() => {
    handler = null;
  });
  const listen: CloseListener = async (h) => {
    handler = h;
    return unlisten;
  };
  /** Sends a close request; returns true if the window stays open. */
  async function close(): Promise<boolean> {
    let prevented = false;
    await handler?.({ preventDefault: () => (prevented = true) });
    return prevented;
  }
  return { listen, unlisten, close, installed: () => handler !== null };
}

describe('guardWindowClose', () => {
  it('keeps the window open when the user cancels, and closes it otherwise', async () => {
    const win = fakeWindow();
    const confirm = vi.fn(async () => false);
    guardWindowClose(win.listen, confirm, vi.fn());
    await tick();
    expect(await win.close()).toBe(true);
    confirm.mockResolvedValueOnce(true);
    expect(await win.close()).toBe(false);
    expect(confirm).toHaveBeenCalledTimes(2);
  });

  it('removes the listener, also when it is removed before the install ends', async () => {
    const win = fakeWindow();
    const remove = guardWindowClose(win.listen, vi.fn(async () => true), vi.fn());
    remove();
    await tick();
    expect(win.unlisten).toHaveBeenCalledOnce();
    expect(win.installed()).toBe(false);
  });

  it('reports an install failure', async () => {
    const onError = vi.fn();
    guardWindowClose(() => Promise.reject(new Error('no window')), vi.fn(async () => true), onError);
    await tick();
    expect(onError).toHaveBeenCalledOnce();
  });
});
