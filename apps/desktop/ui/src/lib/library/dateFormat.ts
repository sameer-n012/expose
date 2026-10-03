/**
 * Display format of the capture time.
 *
 * `captureTimeMs` is the wall-clock time of the camera, stored as if it were
 * UTC. The camera does not store a time zone. Thus the format must use the
 * UTC time zone. Any other zone would shift the shown time.
 */

/** Text for a photo that has no capture time. */
export const NO_DATE_TEXT = '—';

/** Largest absolute time value (ms) that a JavaScript `Date` can hold. */
const MAX_DATE_MS = 8.64e15;

const DEFAULT_FORMAT = new Intl.DateTimeFormat(undefined, {
  dateStyle: 'medium',
  timeStyle: 'short',
  timeZone: 'UTC',
});

/**
 * Formats a capture time for display.
 * @returns the text, or `NO_DATE_TEXT` for null and for a value that is not a valid time.
 */
export function formatCaptureTime(ms: number | null, format: Intl.DateTimeFormat = DEFAULT_FORMAT): string {
  if (ms === null || !Number.isFinite(ms) || Math.abs(ms) > MAX_DATE_MS) {
    return NO_DATE_TEXT;
  }
  return format.format(ms);
}
