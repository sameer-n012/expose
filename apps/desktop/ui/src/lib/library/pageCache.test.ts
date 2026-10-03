import { describe, expect, it } from 'vitest';
import { makePhoto } from '../../test/fakeCatalog';
import { PhotoCache } from './pageCache';

function pageOf(page: number, size: number, count = size) {
  return Array.from({ length: count }, (_, i) => makePhoto(page * size + i));
}

describe('PhotoCache', () => {
  it('rejects a page size that is not a positive integer', () => {
    expect(() => new PhotoCache(0)).toThrow(RangeError);
    expect(() => new PhotoCache(1.5)).toThrow(RangeError);
  });

  it('returns undefined for a photo that is not loaded', () => {
    const cache = new PhotoCache(10);
    expect(cache.get(0)).toBeUndefined();
    expect(cache.get(-1)).toBeUndefined();
    expect(cache.get(1.5)).toBeUndefined();
  });

  it('returns the photos of a loaded page by grid position', () => {
    const cache = new PhotoCache(10);
    cache.setPage(1, pageOf(1, 10), 25);
    expect(cache.get(10)?.id).toBe(10);
    expect(cache.get(19)?.id).toBe(19);
    expect(cache.get(9)).toBeUndefined();
    expect(cache.total).toBe(25);
  });

  it('claims each missing page once', () => {
    const cache = new PhotoCache(10);
    expect(cache.claimMissing(5, 25)).toEqual([0, 1, 2]);
    expect(cache.claimMissing(5, 25)).toEqual([]);
    cache.setPage(0, pageOf(0, 10), 100);
    expect(cache.claimMissing(0, 35)).toEqual([3]);
  });

  it('limits the claim to the known total', () => {
    const cache = new PhotoCache(10);
    cache.setPage(0, pageOf(0, 10, 5), 5);
    expect(cache.claimMissing(0, 1000)).toEqual([]);
    expect(cache.claimMissing(-20, -5)).toEqual([]);
  });

  it('clears pages, claims, and total', () => {
    const cache = new PhotoCache(10);
    cache.setPage(0, pageOf(0, 10), 10);
    cache.clear();
    expect(cache.get(0)).toBeUndefined();
    expect(cache.total).toBeNull();
    expect(cache.claimMissing(0, 0)).toEqual([0]);
  });

  it('lists the loaded ids in grid order', () => {
    const cache = new PhotoCache(2);
    cache.setPage(2, pageOf(2, 2), 6);
    cache.setPage(0, pageOf(0, 2), 6);
    expect(cache.loadedIds()).toEqual([0, 1, 4, 5]);
  });

  it('updates photos by id and restores them', () => {
    const cache = new PhotoCache(10);
    cache.setPage(0, pageOf(0, 10), 10);
    const previous = cache.update([2, 3, 99], (photo) => ({ ...photo, rating: 4 }));
    expect(previous.map((p) => p.id).sort()).toEqual([2, 3]);
    expect(cache.get(2)?.rating).toBe(4);
    expect(cache.get(4)?.rating).toBe(0);
    cache.restore(previous);
    expect(cache.get(2)?.rating).toBe(0);
    expect(cache.get(3)?.rating).toBe(0);
  });
});
