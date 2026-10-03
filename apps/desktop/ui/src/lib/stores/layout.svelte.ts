/**
 * State of the main window layout: which panels are visible and how wide
 * the side panels are.
 *
 * The size values must match the tokens in `src/styles/tokens.css`
 * (--panel-left-width, --panel-right-width, --panel-min-width,
 * --panel-max-width). A unit test checks this.
 */

/** A side panel. */
export type Side = 'left' | 'right';

/** A region that the user can hide. */
export type Region = Side | 'topbar' | 'filmstrip';

/** Width limits of the side panels, in CSS pixels. */
export const PANEL_LIMITS = { min: 180, max: 480 } as const;

/** Default widths of the side panels, in CSS pixels. */
export const DEFAULT_PANEL_WIDTH: Readonly<Record<Side, number>> = { left: 260, right: 300 };

/** Step for a keyboard resize (arrow key on the resize handle), in CSS pixels. */
export const RESIZE_STEP = 16;

/** Limits a panel width to PANEL_LIMITS. A non-finite value gives the minimum. */
export function clampPanelWidth(width: number): number {
  if (!Number.isFinite(width)) {
    return PANEL_LIMITS.min;
  }
  return Math.round(Math.min(PANEL_LIMITS.max, Math.max(PANEL_LIMITS.min, width)));
}

const REGIONS: readonly Region[] = ['topbar', 'left', 'right', 'filmstrip'];

export class LayoutStore {
  /** Visibility of each region. */
  visible = $state<Record<Region, boolean>>({ topbar: true, left: true, right: true, filmstrip: true });
  /** Width of each side panel, in CSS pixels. Always inside PANEL_LIMITS. */
  width = $state<Record<Side, number>>({ ...DEFAULT_PANEL_WIDTH });

  /** Shows or hides one region. */
  toggle(region: Region): void {
    this.visible[region] = !this.visible[region];
  }

  /**
   * Toggles both side panels together. If one or both are visible, both
   * become hidden. If both are hidden, both become visible.
   */
  toggleSides(): void {
    const show = !this.visible.left && !this.visible.right;
    this.visible.left = show;
    this.visible.right = show;
  }

  /**
   * Toggles all panels (top bar, side panels, filmstrip) together. If one or
   * more are visible, all become hidden. If all are hidden, all become visible.
   */
  toggleAll(): void {
    const show = REGIONS.every((region) => !this.visible[region]);
    for (const region of REGIONS) {
      this.visible[region] = show;
    }
  }

  /** Sets the width of a side panel. The value is limited to PANEL_LIMITS. */
  setWidth(side: Side, width: number): void {
    this.width[side] = clampPanelWidth(width);
  }

  /** Sets the width of a side panel back to its default. */
  resetWidth(side: Side): void {
    this.width[side] = DEFAULT_PANEL_WIDTH[side];
  }
}
