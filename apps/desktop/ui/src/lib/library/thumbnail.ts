/**
 * The one place that decides the thumbnail image of a photo.
 *
 * The desktop shell serves thumbnails with the `thumb` URI scheme. The URL
 * form depends on the platform of the web view:
 *
 * - macOS and Linux: `thumb://localhost/<photo id>`
 * - Windows (and Android): `http://thumb.localhost/<photo id>`
 *
 * The query `v=<file modified ms>` changes when the file changes, so the
 * web view does not show an old image. The shell ignores the query. It
 * also sends an ETag, so a change of the develop settings gives a new
 * image at the next load. The query `e=<edit revision>` (only after an
 * edit in this run) makes the web view load the image again.
 */
import type { Photo } from '../catalog';
import { thumbnailRevision } from './thumbnailRevisions.svelte';

/** The name of the URI scheme of the shell. */
export const THUMBNAIL_SCHEME = 'thumb';

/**
 * Returns true if the web view uses the `http://<scheme>.localhost` form
 * for custom schemes (Windows and Android).
 */
export function usesHttpSchemeForm(userAgent: string): boolean {
  return /Windows|Android/i.test(userAgent);
}

/** The URL form of this web view. Computed once. */
const HTTP_FORM: boolean =
  typeof navigator !== 'undefined' && usesHttpSchemeForm(navigator.userAgent);

/**
 * Returns the thumbnail URL of a photo for the given URL form.
 * Exported for tests; use `thumbnailUrl` in components.
 */
export function thumbnailUrlFor(photo: Photo, httpForm: boolean, revision = 0): string {
  const base = httpForm ? `http://${THUMBNAIL_SCHEME}.localhost` : `${THUMBNAIL_SCHEME}://localhost`;
  const edits = revision > 0 ? `&e=${revision}` : '';
  return `${base}/${encodeURIComponent(String(photo.id))}?v=${encodeURIComponent(String(photo.fileModifiedMs))}${edits}`;
}

/**
 * Returns the URL of the thumbnail of a photo. The grid shows a neutral
 * placeholder tile while the image loads, and if the image fails to load
 * (for example, the file is offline or cannot be decoded).
 */
export function thumbnailUrl(photo: Photo): string | null {
  return thumbnailUrlFor(photo, HTTP_FORM, thumbnailRevision(photo.id));
}
