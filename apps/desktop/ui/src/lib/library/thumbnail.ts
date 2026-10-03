/**
 * The one place that decides the thumbnail image of a photo.
 */
import type { Photo } from '../catalog';

/**
 * Returns the URL of the thumbnail of a photo, or null if there is none.
 * A thumbnail cache does not exist yet, so this always returns null. The
 * grid then shows a neutral placeholder tile.
 */
export function thumbnailUrl(_photo: Photo): string | null {
  return null;
}
