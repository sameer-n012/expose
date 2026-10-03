/**
 * Layout math of the virtualized library grid.
 *
 * Every row has the same height. Thus the visible rows follow from the
 * scroll position, and the grid renders only those rows.
 */

export interface GridConfig {
  /** Minimum cell width, in CSS pixels. */
  readonly minCellWidth: number;
  /** Space between cells and around the grid, in CSS pixels. */
  readonly gap: number;
  /** Height of the text area under the thumbnail, in CSS pixels. */
  readonly infoHeight: number;
  /** Width / height of the thumbnail area. */
  readonly thumbAspect: number;
}

export const GRID_CONFIG: GridConfig = { minCellWidth: 170, gap: 8, infoHeight: 62, thumbAspect: 3 / 2 };

export interface GridMetrics {
  readonly columns: number;
  readonly cellWidth: number;
  readonly cellHeight: number;
  /** Distance between the tops of two rows: cell height + gap. */
  readonly rowHeight: number;
}

/** Computes the column count and the cell size for a grid of `width` pixels. */
export function gridMetrics(width: number, config: GridConfig = GRID_CONFIG): GridMetrics {
  const inner = Math.max(width - 2 * config.gap, 0);
  const columns = Math.max(1, Math.floor((inner + config.gap) / (config.minCellWidth + config.gap)));
  const cellWidth = Math.max((inner - config.gap * (columns - 1)) / columns, 1);
  const cellHeight = Math.round(cellWidth / config.thumbAspect + config.infoHeight);
  return { columns, cellWidth, cellHeight, rowHeight: cellHeight + config.gap };
}

/** Number of rows for `total` photos. */
export function rowCount(total: number, columns: number): number {
  return Math.ceil(Math.max(total, 0) / columns);
}

/** Total height of the scrollable content, in CSS pixels. */
export function contentHeight(total: number, metrics: GridMetrics, gap: number = GRID_CONFIG.gap): number {
  return rowCount(total, metrics.columns) * metrics.rowHeight + gap;
}

/**
 * The rows that intersect the viewport, plus `overscan` rows on each side.
 * @returns first and last row (both included), or null if there is no row.
 */
export function visibleRows(
  scrollTop: number,
  viewportHeight: number,
  total: number,
  metrics: GridMetrics,
  overscan = 1,
  gap: number = GRID_CONFIG.gap,
): { first: number; last: number } | null {
  const rows = rowCount(total, metrics.columns);
  if (rows === 0 || viewportHeight <= 0) {
    return null;
  }
  const top = Math.max(scrollTop - gap, 0);
  const first = Math.min(Math.floor(top / metrics.rowHeight), rows - 1);
  const last = Math.min(Math.floor((scrollTop + viewportHeight - gap) / metrics.rowHeight), rows - 1);
  return { first: Math.max(first - overscan, 0), last: Math.min(Math.max(last, first) + overscan, rows - 1) };
}

/**
 * The scroll position that makes row `row` fully visible with the least
 * movement. Returns `scrollTop` if the row is visible already.
 */
export function scrollTopToReveal(
  row: number,
  scrollTop: number,
  viewportHeight: number,
  metrics: GridMetrics,
  gap: number = GRID_CONFIG.gap,
): number {
  const rowTop = gap + row * metrics.rowHeight;
  const rowBottom = rowTop + metrics.cellHeight;
  if (rowTop < scrollTop) {
    return Math.max(rowTop - gap, 0);
  }
  if (rowBottom > scrollTop + viewportHeight) {
    return rowBottom - viewportHeight + gap;
  }
  return scrollTop;
}
