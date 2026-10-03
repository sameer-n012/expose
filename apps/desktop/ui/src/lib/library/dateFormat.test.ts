import { describe, expect, it } from 'vitest';
import { formatCaptureTime, NO_DATE_TEXT } from './dateFormat';

const EN_UTC = new Intl.DateTimeFormat('en-US', { dateStyle: 'medium', timeStyle: 'short', timeZone: 'UTC' });

describe('formatCaptureTime', () => {
  it('shows the stored wall-clock time without a time zone shift', () => {
    // 2024-03-09 23:45 camera time. A local zone would change the day or hour.
    const ms = Date.UTC(2024, 2, 9, 23, 45);
    expect(formatCaptureTime(ms, EN_UTC)).toBe('Mar 9, 2024, 11:45 PM');
  });

  it('shows nothing useful for null', () => {
    expect(formatCaptureTime(null)).toBe(NO_DATE_TEXT);
  });

  it('does not throw for a value that is not a valid time', () => {
    expect(formatCaptureTime(Number.NaN)).toBe(NO_DATE_TEXT);
    expect(formatCaptureTime(Number.POSITIVE_INFINITY)).toBe(NO_DATE_TEXT);
    expect(formatCaptureTime(9e15)).toBe(NO_DATE_TEXT);
  });

  it('formats with the default format', () => {
    expect(formatCaptureTime(Date.UTC(2024, 0, 1, 12, 0))).toContain('2024');
  });
});
