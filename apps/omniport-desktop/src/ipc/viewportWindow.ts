/**
 * Pure viewport-window math for the virtualized serial monitor.
 *
 * Single source of truth for pixel → row-range conversion: the monitor canvas,
 * the fetch window (with overscan) and the synchronized logic strip all derive
 * their ranges from `computeWindows`. No React, DOM or Tauri imports, so the
 * math is exercised directly by node:test.
 */

export interface WindowRange {
  /** First row index included in the window. */
  start: number;
  /** Number of rows in the window (clamped to the available rows). */
  count: number;
}

export interface WindowCalc {
  /** Exact rows visible in the viewport (no overscan). */
  visible: WindowRange;
  /** Rows to fetch/paint: visible window plus overscan on both sides. */
  render: WindowRange;
  /** Maximum legal scroll offset in pixels. */
  maxScrollPx: number;
  /** Pixels the window is scrolled into the first visible row [0, rowHeight). */
  offsetInsideRowPx: number;
}

export const EMPTY_WINDOW: WindowCalc = {
  visible: { start: 0, count: 0 },
  render: { start: 0, count: 0 },
  maxScrollPx: 0,
  offsetInsideRowPx: 0,
};

/** Total height in pixels that the virtualized row list occupies. */
export function computeContentHeight(totalRows: number, rowHeightPx: number): number {
  if (totalRows <= 0 || rowHeightPx <= 0) return 0;
  return totalRows * rowHeightPx;
}

/** Largest legal scroll offset: content height minus the viewport height. */
export function computeScrollMax(
  totalRows: number,
  rowHeightPx: number,
  viewportHeightPx: number
): number {
  if (totalRows <= 0 || rowHeightPx <= 0) return 0;
  const contentHeight = totalRows * rowHeightPx;
  const viewport = Math.max(0, viewportHeightPx);
  return Math.max(0, contentHeight - viewport);
}

/** Clamps a scroll offset into the legal [0, maxScroll] range. */
export function clampScrollTop(
  scrollTopPx: number,
  viewportHeightPx: number,
  rowHeightPx: number,
  totalRows: number
): number {
  if (!Number.isFinite(scrollTopPx)) return 0;
  const maxScroll = computeScrollMax(totalRows, rowHeightPx, viewportHeightPx);
  return Math.min(maxScroll, Math.max(0, scrollTopPx));
}

/**
 * Computes the visible and render windows for a scroll offset.
 *
 * Compatible with the legacy `calculateViewportRange`:
 *   visible.start = floor(scrollTop / rowHeight)
 *   visible.count = ceil(viewportHeight / rowHeight), clamped to available rows
 *   render.start  = max(0, visible.start - overscan)
 *   render.end    = min(totalRows, visible.start + visible.count + overscan)
 */
export function computeWindows(
  scrollTopPx: number,
  viewportHeightPx: number,
  rowHeightPx: number,
  totalRows: number,
  overscanRows: number
): WindowCalc {
  if (totalRows <= 0 || rowHeightPx <= 0) {
    return {
      visible: { start: 0, count: 0 },
      render: { start: 0, count: 0 },
      maxScrollPx: 0,
      offsetInsideRowPx: 0,
    };
  }

  const viewport = Math.max(0, viewportHeightPx);
  const overscan = Math.max(0, Math.floor(overscanRows));
  const maxScrollPx = computeScrollMax(totalRows, rowHeightPx, viewport);
  const clampedScrollTop = clampScrollTop(scrollTopPx, viewport, rowHeightPx, totalRows);

  const visibleStart = Math.max(0, Math.floor(clampedScrollTop / rowHeightPx));
  const visibleCount = Math.min(
    Math.max(0, Math.ceil(viewport / rowHeightPx)),
    Math.max(0, totalRows - visibleStart)
  );

  if (viewport <= 0) {
    return {
      visible: { start: visibleStart, count: 0 },
      render: { start: 0, count: 0 },
      maxScrollPx,
      offsetInsideRowPx: 0,
    };
  }

  const renderStart = Math.max(0, visibleStart - overscan);
  const renderEnd = Math.min(
    totalRows,
    visibleStart + visibleCount + overscan
  );

  return {
    visible: { start: visibleStart, count: visibleCount },
    render: { start: renderStart, count: Math.max(0, renderEnd - renderStart) },
    maxScrollPx,
    offsetInsideRowPx: clampedScrollTop - visibleStart * rowHeightPx,
  };
}

/** Inclusive last row index of a window, or -1 for an empty window. */
export function windowEndRow(window: WindowRange | { start: number; count: number }): number {
  if (window.count <= 0) return -1;
  return window.start + window.count - 1;
}

/** Key used for fetch de-duplication: render window identity. */
export function renderWindowKey(window: { start: number; count: number }): string {
  return `${window.start}:${window.count}`;
}
