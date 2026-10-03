import { describe, expect, it } from 'vitest';
import type { Photo } from '../catalog';
import { thumbnailUrl, thumbnailUrlFor, usesHttpSchemeForm } from './thumbnail';

const photo: Photo = {
  id: 42,
  folderId: 1,
  fileName: 'a.nef',
  fileSize: 1000,
  fileModifiedMs: 1_700_000_000_123,
  importedMs: 0,
  captureTimeMs: null,
  width: null,
  height: null,
  rating: 0,
  flag: 'none',
  colorLabel: 'none',
};

describe('thumbnailUrlFor', () => {
  it('uses the custom scheme form on macOS and Linux', () => {
    expect(thumbnailUrlFor(photo, false)).toBe('thumb://localhost/42?v=1700000000123');
  });

  it('uses the http form on Windows', () => {
    expect(thumbnailUrlFor(photo, true)).toBe('http://thumb.localhost/42?v=1700000000123');
  });

  it('changes when the file changes', () => {
    const changed = { ...photo, fileModifiedMs: photo.fileModifiedMs + 1 };
    expect(thumbnailUrlFor(changed, false)).not.toBe(thumbnailUrlFor(photo, false));
  });
});

describe('usesHttpSchemeForm', () => {
  it('detects the platform from the user agent', () => {
    expect(usesHttpSchemeForm('Mozilla/5.0 (Windows NT 10.0; Win64; x64) Edg/120')).toBe(true);
    expect(usesHttpSchemeForm('Mozilla/5.0 (Linux; Android 14)')).toBe(true);
    expect(usesHttpSchemeForm('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit')).toBe(false);
    expect(usesHttpSchemeForm('Mozilla/5.0 (X11; Linux x86_64) AppleWebKit')).toBe(false);
  });
});

describe('thumbnailUrl', () => {
  it('returns a URL for a photo', () => {
    const url = thumbnailUrl(photo);
    expect(url).not.toBeNull();
    expect(url).toContain('/42?v=1700000000123');
  });
});
