/**
 * Virtualized HTML5 Canvas Hex/ASCII Monitor Renderer
 * Optimized for 60 FPS rendering under high packet volume.
 */

import { FrameFlags, PinFlags, type ViewportRowData } from "../../types/serial.ts";
import { type MonitorTheme, darkTheme } from "../../utils/theme.ts";

// Fast lookup table for hex byte strings (zero GC overhead)
export const HEX_LUT: string[] = Array.from({ length: 256 }, (_, i) =>
  i.toString(16).padStart(2, "0").toUpperCase()
);

export interface HexColumnLayout {
  indexX: number;
  indexWidth: number;
  timeX: number;
  timeWidth: number;
  endpointX: number;
  endpointWidth: number;
  pinsX: number;
  pinsWidth: number;
  hexX: number;
  hexWidth: number;
  asciiX: number;
  asciiWidth: number;
  totalWidth: number;
}

export function computeColumnLayout(containerWidth: number): HexColumnLayout {
  const indexWidth = 72;
  const timeWidth = 100;
  const endpointWidth = 84;
  const pinsWidth = 72;
  const hexWidth = 360; // 16 bytes * ~22px + separator
  const asciiWidth = 160;

  const indexX = 8;
  const timeX = indexX + indexWidth;
  const endpointX = timeX + timeWidth;
  const pinsX = endpointX + endpointWidth;
  const hexX = pinsX + pinsWidth + 8;
  const asciiX = hexX + hexWidth + 12;
  const totalWidth = Math.max(containerWidth, asciiX + asciiWidth + 16);

  return {
    indexX,
    indexWidth,
    timeX,
    timeWidth,
    endpointX,
    endpointWidth,
    pinsX,
    pinsWidth,
    hexX,
    hexWidth,
    asciiX,
    asciiWidth,
    totalWidth,
  };
}

export interface ViewportCalcResult {
  startRow: number;
  endRow: number;
  renderStartRow: number;
  renderEndRow: number;
  offsetY: number;
  totalHeight: number;
}

export function calculateViewportRange(
  scrollTop: number,
  viewportHeight: number,
  rowHeight: number,
  totalRows: number,
  overscan = 5
): ViewportCalcResult {
  if (totalRows <= 0 || rowHeight <= 0) {
    return {
      startRow: 0,
      endRow: 0,
      renderStartRow: 0,
      renderEndRow: 0,
      offsetY: 0,
      totalHeight: 0,
    };
  }

  const startRow = Math.max(0, Math.floor(scrollTop / rowHeight));
  const visibleCount = Math.ceil(viewportHeight / rowHeight);
  const endRow = Math.min(totalRows, startRow + visibleCount);

  const renderStartRow = Math.max(0, startRow - overscan);
  const renderEndRow = Math.min(totalRows, endRow + overscan);
  const offsetY = renderStartRow * rowHeight - scrollTop;
  const totalHeight = totalRows * rowHeight;

  return {
    startRow,
    endRow,
    renderStartRow,
    renderEndRow,
    offsetY,
    totalHeight,
  };
}

export function formatTimeOffset(timestampNs: bigint, baseTimeNs = 0n): string {
  const deltaNs = timestampNs >= baseTimeNs ? timestampNs - baseTimeNs : 0n;
  const sec = Number(deltaNs / 1_000_000_000n);
  const frac = Number((deltaNs % 1_000_000_000n) / 1_000n); // microseconds
  return `+${sec.toString().padStart(2, "0")}.${frac.toString().padStart(6, "0")}s`;
}

export interface RenderHexOptions {
  ctx: CanvasRenderingContext2D;
  width: number;
  height: number;
  rowHeight: number;
  headerHeight: number;
  scrollTop: number;
  totalRows: number;
  rows: ViewportRowData[];
  selectedRowIndex?: number | null;
  hoveredRowIndex?: number | null;
  theme?: MonitorTheme;
  baseTimeNs?: bigint;
}

/**
 * Renders the visible slice of the Hex/ASCII monitor onto an HTML5 canvas.
 */
export function renderHexMonitor(options: RenderHexOptions): void {
  const {
    ctx,
    width,
    height,
    rowHeight,
    headerHeight,
    scrollTop,
    totalRows,
    rows,
    selectedRowIndex = null,
    hoveredRowIndex = null,
    theme = darkTheme,
    baseTimeNs = 0n,
  } = options;

  const cols = computeColumnLayout(width);

  // 1. Clear background
  ctx.fillStyle = theme.background;
  ctx.fillRect(0, 0, width, height);

  // 2. Compute visible rows
  const contentHeight = Math.max(0, height - headerHeight);
  const vp = calculateViewportRange(
    scrollTop,
    contentHeight,
    rowHeight,
    totalRows
  );

  // Map input rows by rowIndex for fast random access
  const rowMap = new Map<number, ViewportRowData>();
  for (const r of rows) {
    rowMap.set(r.rowIndex, r);
  }

  // 3. Render Data Rows
  ctx.font = '12px "JetBrains Mono", "SF Mono", "Cascadia Code", Consolas, monospace';
  ctx.textBaseline = "middle";

  for (let idx = vp.renderStartRow; idx < vp.renderEndRow; idx++) {
    const y = headerHeight + (idx * rowHeight - scrollTop);
    if (y + rowHeight < headerHeight || y > height) continue;

    // Row highlights (hover & selection)
    if (selectedRowIndex === idx) {
      ctx.fillStyle = theme.selectionFill;
      ctx.fillRect(0, y, width, rowHeight);
    } else if (hoveredRowIndex === idx) {
      ctx.fillStyle = theme.hoverFill;
      ctx.fillRect(0, y, width, rowHeight);
    }

    const rowData = rowMap.get(idx);
    const textY = y + rowHeight / 2;

    if (!rowData) {
      // Row placeholder when scrolling faster than IPC fetch
      ctx.fillStyle = theme.asciiUnprintable;
      ctx.fillText(`#${idx.toString().padStart(6, "0")}  [loading...]`, cols.indexX, textY);
      continue;
    }

    // A. Row Index
    ctx.fillStyle = theme.indexText;
    ctx.fillText(`#${idx.toString().padStart(6, "0")}`, cols.indexX, textY);

    // B. Timestamp
    ctx.fillStyle = theme.timestampText;
    ctx.fillText(formatTimeOffset(rowData.timestampNs, baseTimeNs), cols.timeX, textY);

    // C. Direction & Endpoint Badge
    const isTx = (rowData.flags & FrameFlags.DIR_TX) !== 0;
    const hasCrcErr = (rowData.flags & FrameFlags.ERR_CRC) !== 0;
    const dirLabel = isTx ? "TX" : "RX";
    const epLabel = `${dirLabel}:P${rowData.endpointId}`;

    if (hasCrcErr) {
      ctx.fillStyle = theme.errorText;
      ctx.fillText(`!${epLabel} ERR`, cols.endpointX, textY);
    } else {
      ctx.fillStyle = isTx ? theme.txText : theme.rxText;
      ctx.fillText(epLabel, cols.endpointX, textY);
    }

    // D. Mini Pin Indicators (R, C, D, S, C, R)
    const pins = rowData.pins;
    const rts = (pins & PinFlags.RTS) !== 0 ? "R" : "·";
    const cts = (pins & PinFlags.CTS) !== 0 ? "C" : "·";
    const dtr = (pins & PinFlags.DTR) !== 0 ? "D" : "·";
    const dcd = (pins & PinFlags.DCD) !== 0 ? "C" : "·";
    ctx.fillStyle = theme.headerText;
    ctx.fillText(`${rts}${cts}${dtr}${dcd}`, cols.pinsX, textY);

    // E. Hex Bytes (16 bytes per row)
    const payload = rowData.payload;
    const byteCount = Math.min(16, payload.length);
    let hexCursorX = cols.hexX;

    for (let b = 0; b < byteCount; b++) {
      if (b === 8) {
        // Gap after 8 bytes
        hexCursorX += 10;
      }
      const byteVal = payload[b];
      ctx.fillStyle = hasCrcErr ? theme.errorText : theme.hexText;
      ctx.fillText(HEX_LUT[byteVal], hexCursorX, textY);
      hexCursorX += 20;
    }

    // F. ASCII Characters
    let asciiCursorX = cols.asciiX;
    for (let b = 0; b < byteCount; b++) {
      const byteVal = payload[b];
      const isPrintable = byteVal >= 32 && byteVal <= 126;
      ctx.fillStyle = isPrintable ? theme.asciiText : theme.asciiUnprintable;
      ctx.fillText(isPrintable ? String.fromCharCode(byteVal) : ".", asciiCursorX, textY);
      asciiCursorX += 8.5;
    }
  }

  // 4. Render Sticky Header
  ctx.fillStyle = theme.headerBackground;
  ctx.fillRect(0, 0, width, headerHeight);

  ctx.strokeStyle = theme.borderColor;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, headerHeight - 0.5);
  ctx.lineTo(width, headerHeight - 0.5);
  ctx.stroke();

  ctx.fillStyle = theme.headerText;
  ctx.font = 'bold 11px -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif';
  const headerTextY = headerHeight / 2;

  ctx.fillText("FRAME #", cols.indexX, headerTextY);
  ctx.fillText("TIME OFFSET", cols.timeX, headerTextY);
  ctx.fillText("ENDPOINT", cols.endpointX, headerTextY);
  ctx.fillText("PINS", cols.pinsX, headerTextY);
  ctx.fillText("HEX DATA (16 BYTES)", cols.hexX, headerTextY);
  ctx.fillText("ASCII DUMP", cols.asciiX, headerTextY);
}
