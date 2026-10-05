/** DOM tests of the histogram view: empty state, shapes, and clip indicators. */
import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import { computeHistogram } from '../../lib/develop/histogram';
import HistogramView from './HistogramView.svelte';

describe('HistogramView', () => {
  it('shows no plot without a histogram', () => {
    render(HistogramView, { histogram: null });
    expect(screen.queryByTestId('histogram')).toBeNull();
  });

  it('draws three channel areas', () => {
    const rgba = new Uint8ClampedArray([10, 100, 200, 255, 20, 110, 210, 255]);
    const { container } = render(HistogramView, { histogram: computeHistogram(rgba, 2, 1, 1) });
    expect(screen.getByTestId('histogram')).toBeTruthy();
    expect(container.querySelectorAll('polygon').length).toBe(3);
    expect(screen.getByTestId('clip-shadows').classList.contains('lit')).toBe(false);
    expect(screen.getByTestId('clip-highlights').classList.contains('lit')).toBe(false);
  });

  it('lights the clip indicators', () => {
    // Black and white pixels clip both ends.
    const rgba = new Uint8ClampedArray([0, 0, 0, 255, 255, 255, 255, 255]);
    render(HistogramView, { histogram: computeHistogram(rgba, 2, 1, 1) });
    expect(screen.getByTestId('clip-shadows').classList.contains('lit')).toBe(true);
    expect(screen.getByTestId('clip-highlights').classList.contains('lit')).toBe(true);
  });
});
