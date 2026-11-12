import { describe, expect, it, vi } from 'vitest';
import type { DocumentInfo, FilterDto } from './api';
import {
  FilterClient,
  clampAmountPercent,
  clampRadius,
  defaultFilterValues,
  filterDto,
  radiusToSlider,
  sliderToRadius,
} from './filter';

const info = (revision: number) => ({ documentId: 1, revision }) as unknown as DocumentInfo;
const blur = (radius: number): FilterDto => ({ type: 'gaussianBlur', radius });

/** A fake API whose preview calls finish when the test says so. */
function setup() {
  const pending: { filter: FilterDto; resolve: () => void; reject: (e: unknown) => void }[] = [];
  let revision = 0;
  const api = {
    filterPreview: vi.fn((_doc: number, _layer: number, _target: string, filter: FilterDto) => {
      return new Promise<DocumentInfo>((resolve, reject) => {
        pending.push({ filter, resolve: () => resolve(info(++revision)), reject });
      });
    }),
    filterCommit: vi.fn(async (_doc: number) => info(++revision)),
    filterCancel: vi.fn(async (_doc: number) => info(++revision)),
  };
  const onInfo = vi.fn();
  const onError = vi.fn();
  const client = new FilterClient({ api, documentId: 1, layerId: 7, target: 'pixels', onInfo, onError });
  return { api, onInfo, onError, client, pending };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

describe('filterDto', () => {
  it('converts the amount from percent to a fraction', () => {
    const values = defaultFilterValues();
    values.unsharpMask = { amountPercent: 150, radius: 2.5, threshold: 4 };
    expect(filterDto('unsharpMask', values)).toEqual({ type: 'unsharpMask', amount: 1.5, radius: 2.5, threshold: 4 });
  });

  it('clamps the values to the limits', () => {
    const values = defaultFilterValues();
    values.unsharpMask = { amountPercent: 9000, radius: 0, threshold: 999 };
    expect(filterDto('unsharpMask', values)).toEqual({ type: 'unsharpMask', amount: 5, radius: 0.1, threshold: 255 });
    expect(clampAmountPercent(0)).toBe(1);
    expect(clampRadius(5000)).toBe(1000);
  });
});

describe('radius slider', () => {
  it('maps the ends and the middle on a log scale', () => {
    expect(sliderToRadius(0)).toBe(0.1);
    expect(sliderToRadius(1)).toBe(1000);
    expect(sliderToRadius(0.5)).toBe(10);
    expect(radiusToSlider(10)).toBeCloseTo(0.5, 6);
  });
});

describe('FilterClient preview', () => {
  it('keeps one call in flight and sends only the newest waiting value', async () => {
    const { api, client, pending, onInfo } = setup();
    client.preview(blur(1));
    client.preview(blur(2));
    client.preview(blur(3));
    expect(api.filterPreview).toHaveBeenCalledTimes(1);
    pending[0]?.resolve();
    await tick();
    expect(api.filterPreview).toHaveBeenCalledTimes(2);
    expect(pending[1]?.filter).toEqual(blur(3));
    pending[1]?.resolve();
    await tick();
    expect(onInfo).toHaveBeenCalledTimes(2);
    expect(api.filterPreview).toHaveBeenCalledTimes(2);
  });

  it('sends the document, layer, and target', async () => {
    const { api, client } = setup();
    client.preview(blur(1));
    expect(api.filterPreview).toHaveBeenCalledWith(1, 7, 'pixels', blur(1));
  });

  it('reports an error once and drops the waiting value', async () => {
    const { client, pending, onError, api } = setup();
    client.preview(blur(1));
    client.preview(blur(2));
    pending[0]?.reject(new Error('bad'));
    await tick();
    expect(onError).toHaveBeenCalledTimes(1);
    expect(api.filterPreview).toHaveBeenCalledTimes(1);
  });
});

describe('FilterClient end paths', () => {
  it('OK commits when the last preview shows the current values', async () => {
    const { api, client, pending } = setup();
    client.preview(blur(4));
    pending[0]?.resolve();
    await tick();
    expect(await client.commit(blur(4))).toBe(true);
    expect(api.filterPreview).toHaveBeenCalledTimes(1);
    expect(api.filterCommit).toHaveBeenCalledWith(1);
  });

  it('OK waits for the call in flight, then previews the current values first', async () => {
    const { api, client, pending } = setup();
    client.preview(blur(4));
    const done = client.commit(blur(5));
    pending[0]?.resolve();
    await tick();
    expect(pending[1]?.filter).toEqual(blur(5));
    pending[1]?.resolve();
    expect(await done).toBe(true);
    expect(api.filterCommit).toHaveBeenCalledTimes(1);
  });

  it('OK with the preview off previews, then commits', async () => {
    const { api, client, pending } = setup();
    client.preview(blur(4));
    pending[0]?.resolve();
    await tick();
    await client.cancelSession();
    expect(api.filterCancel).toHaveBeenCalledTimes(1);
    const done = client.commit(blur(4));
    await tick();
    expect(api.filterPreview).toHaveBeenCalledTimes(2);
    pending[1]?.resolve();
    expect(await done).toBe(true);
    expect(api.filterCommit).toHaveBeenCalledTimes(1);
  });

  it('Cancel without a session makes no call', async () => {
    const { api, client } = setup();
    await client.dispose();
    expect(api.filterCancel).not.toHaveBeenCalled();
  });

  it('Cancel after a preview restores the layer and stops later previews', async () => {
    const { api, client, pending } = setup();
    client.preview(blur(4));
    pending[0]?.resolve();
    await tick();
    await client.dispose();
    expect(api.filterCancel).toHaveBeenCalledTimes(1);
    client.preview(blur(9));
    expect(api.filterPreview).toHaveBeenCalledTimes(1);
  });

  it('the preview can start again after the session was canceled', async () => {
    const { api, client, pending } = setup();
    client.preview(blur(4));
    pending[0]?.resolve();
    await tick();
    await client.cancelSession();
    client.preview(blur(6));
    expect(api.filterPreview).toHaveBeenCalledTimes(2);
    expect(pending[1]?.filter).toEqual(blur(6));
  });

  it('a failed commit reports the error and lets the owner cancel', async () => {
    const { api, client, pending, onError } = setup();
    api.filterCommit.mockRejectedValueOnce(new Error('no'));
    client.preview(blur(4));
    pending[0]?.resolve();
    await tick();
    expect(await client.commit(blur(4))).toBe(false);
    expect(onError).toHaveBeenCalledTimes(1);
    await client.dispose();
    expect(api.filterCancel).toHaveBeenCalledTimes(1);
  });

  it('abandon makes no cancel call', async () => {
    const { api, client, pending } = setup();
    client.preview(blur(4));
    pending[0]?.resolve();
    await tick();
    client.abandon();
    await client.dispose();
    expect(api.filterCancel).not.toHaveBeenCalled();
  });
});
