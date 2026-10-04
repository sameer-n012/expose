/**
 * Edit revisions of the library thumbnails.
 *
 * The thumbnail URL has a query that the shell ignores (see `thumbnail.ts`).
 * When a photo gets new develop settings, the Develop module calls `bump`.
 * The URL of that photo then changes, so the grid loads the thumbnail
 * again. The shell sends an ETag that depends on the settings, so the
 * new request gets the new image.
 */
import { SvelteMap } from 'svelte/reactivity';
import type { PhotoId } from '../catalog';

const revisions = new SvelteMap<PhotoId, number>();

/** The number of saved edits of a photo in this run. 0 if none. */
export function thumbnailRevision(id: PhotoId): number {
  return revisions.get(id) ?? 0;
}

/** Records that the develop settings of a photo changed. */
export function bumpThumbnailRevision(id: PhotoId): void {
  revisions.set(id, (revisions.get(id) ?? 0) + 1);
}
