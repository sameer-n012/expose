import { describe, expect, it } from 'vitest';
import { contentHeight, GRID_CONFIG, gridMetrics, rowCount, scrollTopToReveal, visibleRows } from './gridLayout';

describe('gridMetrics', () => {
  it('uses at least one column, also for a tiny or zero width', () => {
    expect(gridMetrics(0).columns).toBe(1);
    expect(gridMetrics(50).columns).toBe(1);
  });

  it('fits as many columns as the minimum cell width allows', () => {
    const { minCellWidth, gap } = GRID_CONFIG;
    const width = 2 * gap + 4 * minCellWidth + 3 * gap;
    expect(gridMetrics(width).columns).toBe(4);
    expect(gridMetrics(width - 1).columns).toBe(3);
  });

  it('makes the cells fill the row', () => {
    const m = gridMetrics(1000);
    const used = 2 * GRID_CONFIG.gap + m.columns * m.cellWidth + (m.columns - 1) * GRID_CONFIG.gap;
    expect(used).toBeCloseTo(1000);
    expect(m.rowHeight).toBe(m.cellHeight + GRID_CONFIG.gap);
  });
});

describe('rowCount and contentHeight', () => {
  it('rounds the row count up', () => {
    expect(rowCount(0, 4)).toBe(0);
    expect(rowCount(1, 4)).toBe(1);
    expect(rowCount(8, 4)).toBe(2);
    expect(rowCount(9, 4)).toBe(3);
  });

  it('stays small enough for 50 000 photos', () => {
    const m = gridMetrics(1000);
    expect(contentHeight(50_000, m)).toBeLessThan(10_000_000);
    expect(contentHeight(50_000, m)).toBe(rowCount(50_000, m.columns) * m.rowHeight + GRID_CONFIG.gap);
  });
});

describe('visibleRows', () => {
  const m = gridMetrics(1000);

  it('returns null for no photos or a hidden viewport', () => {
    expect(visibleRows(0, 600, 0, m)).toBeNull();
    expect(visibleRows(0, 0, 100, m)).toBeNull();
  });

  it('covers the viewport at the top, with overscan below', () => {
    const rows = visibleRows(0, 600, 1000, m, 1);
    expect(rows?.first).toBe(0);
    expect(rows?.last).toBeGreaterThanOrEqual(Math.floor(600 / m.rowHeight));
    expect(rows?.last).toBeLessThan(10);
  });

  it('renders only a small window of 50 000 photos', () => {
    const total = 50_000;
    const scrollTop = contentHeight(total, m) / 2;
    const rows = visibleRows(scrollTop, 600, total, m);
    expect(rows).not.toBeNull();
    if (rows !== null) {
      expect((rows.last - rows.first + 1) * m.columns).toBeLessThan(60);
      expect(rows.first * m.columns).toBeLessThanOrEqual(total / 2);
      expect((rows.last + 1) * m.columns).toBeGreaterThan(total / 2);
    }
  });

  it('limits the rows at the end of the grid', () => {
    const total = 10;
    const rows = visibleRows(1e9, 600, total, m);
    expect(rows?.last).toBe(rowCount(total, m.columns) - 1);
    expect(rows?.first).toBeGreaterThanOrEqual(0);
  });
});

describe('scrollTopToReveal', () => {
  const m = gridMetrics(1000);

  it('keeps the scroll position if the row is visible', () => {
    expect(scrollTopToReveal(0, 0, 600, m)).toBe(0);
  });

  it('scrolls down until the row is visible at the bottom', () => {
    const next = scrollTopToReveal(10, 0, 600, m);
    const rowBottom = GRID_CONFIG.gap + 10 * m.rowHeight + m.cellHeight;
    expect(next + 600).toBeGreaterThanOrEqual(rowBottom);
  });

  it('scrolls up until the row is visible at the top', () => {
    const next = scrollTopToReveal(1, 5000, 600, m);
    expect(next).toBeLessThanOrEqual(GRID_CONFIG.gap + m.rowHeight);
    expect(next).toBeGreaterThanOrEqual(0);
  });
});
