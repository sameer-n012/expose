/** Tests of the export store: dialog, start, events, cancel, and failures. */
import { describe, expect, it, vi } from 'vitest';
import { defaultForm } from '../export/options';
import type { ExportEvent, ExportRequest } from '../export/types';
import { ExportStore, type ExportApi } from './export.svelte';
import { ToastStore } from './toasts.svelte';

interface Harness {
  store: ExportStore;
  toasts: ToastStore;
  api: { start: ReturnType<typeof vi.fn>; cancel: ReturnType<typeof vi.fn> };
  /** Sends an event as the backend. */
  emit: (event: ExportEvent) => void;
  save: ReturnType<typeof vi.fn>;
}

function setup(startImpl?: () => Promise<number>): Harness {
  let sink: (event: ExportEvent) => void = () => undefined;
  const api = {
    start: vi.fn(async (_request: ExportRequest, onEvent: (event: ExportEvent) => void) => {
      sink = onEvent;
      return startImpl === undefined ? 7 : startImpl();
    }),
    cancel: vi.fn(async () => undefined),
  };
  const toasts = new ToastStore();
  const save = vi.fn();
  const store = new ExportStore({
    toasts,
    api: api as unknown as ExportApi,
    loadForm: () => ({ ...defaultForm(), destination: '/out' }),
    saveForm: save,
  });
  return { store, toasts, api, emit: (e) => sink(e), save };
}

const started = (total: number, jobId = 7): ExportEvent => ({ kind: 'started', jobId, total });
const done = (photoId: number, index: number, total: number, jobId = 7): ExportEvent => ({
  kind: 'photoDone',
  jobId,
  photoId,
  outputPath: `/out/${photoId}.jpg`,
  index,
  total,
});

describe('ExportStore', () => {
  it('shows a toast and stays closed when no photo is selected', () => {
    const h = setup();
    h.store.openDialog([]);
    expect(h.store.isOpen).toBe(false);
    expect(h.toasts.items).toHaveLength(1);
  });

  it('starts a job with the request, saves the form, and closes the dialog', async () => {
    const h = setup();
    h.store.openDialog([1, 2, 3]);
    await h.store.submit();
    const request = h.api.start.mock.calls[0]?.[0] as ExportRequest;
    expect(request.photoIds).toEqual([1, 2, 3]);
    expect(request.destination).toBe('/out');
    expect(h.store.jobId).toBe(7);
    expect(h.store.isOpen).toBe(false);
    expect(h.save).toHaveBeenCalledOnce();
  });

  it('does not start an invalid form', async () => {
    const h = setup();
    h.store.openDialog([1]);
    h.store.form.destination = '';
    await h.store.submit();
    expect(h.api.start).not.toHaveBeenCalled();
    expect(h.store.isOpen).toBe(true);
  });

  it('follows the events to a summary', async () => {
    const h = setup();
    h.store.openDialog([1, 2, 3]);
    await h.store.submit();
    h.emit(started(3));
    expect(h.store.phase).toBe('running');
    h.emit(done(1, 1, 3));
    h.emit({ kind: 'photoSkipped', jobId: 7, photoId: 2, outputPath: '/out/2.jpg', reason: 'exists' });
    h.emit({ kind: 'photoFailed', jobId: 7, photoId: 3, message: 'decode error' });
    expect(h.store.done).toBe(3);
    expect(h.store.failures).toEqual([{ photoId: 3, message: 'decode error' }]);
    h.emit({ kind: 'finished', jobId: 7, written: 1, skipped: 1, failed: 1, cancelled: false });
    expect(h.store.phase).toBe('finished');
    expect(h.store.cancelled).toBe(false);
    h.store.dismiss();
    expect(h.store.phase).toBe('idle');
  });

  it('ignores events of another job', async () => {
    const h = setup();
    h.store.openDialog([1, 2]);
    await h.store.submit();
    h.emit(started(2));
    h.emit(done(1, 1, 2, 99));
    expect(h.store.written).toBe(0);
  });

  it('handles events that come before the start call answers', async () => {
    let resolve: (id: number) => void = () => undefined;
    const h = setup(() => new Promise<number>((r) => (resolve = r)));
    h.store.openDialog([1]);
    const pending = h.store.submit();
    await Promise.resolve();
    h.emit(started(1));
    h.emit(done(1, 1, 1));
    resolve(7);
    await pending;
    expect(h.store.written).toBe(1);
    expect(h.store.jobId).toBe(7);
  });

  it('cancels with the job id and reports the cancelled end', async () => {
    const h = setup();
    h.store.openDialog([1, 2]);
    await h.store.submit();
    h.emit(started(2));
    await h.store.cancel();
    expect(h.api.cancel).toHaveBeenCalledWith(7);
    expect(h.store.cancelling).toBe(true);
    h.emit({ kind: 'finished', jobId: 7, written: 0, skipped: 0, failed: 0, cancelled: true });
    expect(h.store.cancelled).toBe(true);
    expect(h.store.cancelling).toBe(false);
  });

  it('shows the message of a rejected start and keeps the dialog open', async () => {
    const h = setup(() => Promise.reject({ kind: 'unavailable', message: 'An export is running.' }));
    h.store.openDialog([1]);
    await h.store.submit();
    expect(h.store.phase).toBe('idle');
    expect(h.store.isOpen).toBe(true);
    expect(h.toasts.items[0]?.message).toContain('An export is running.');
    expect(h.save).not.toHaveBeenCalled();
  });

  it('refuses to open the dialog while a job runs', async () => {
    const h = setup();
    h.store.openDialog([1]);
    await h.store.submit();
    h.store.openDialog([2]);
    expect(h.store.isOpen).toBe(false);
  });

  it('sets the destination from the folder dialog', async () => {
    const toasts = new ToastStore();
    const store = new ExportStore({ toasts, loadForm: defaultForm, pickFolder: async () => '/picked' });
    await store.chooseDestination();
    expect(store.form.destination).toBe('/picked');
  });
});
