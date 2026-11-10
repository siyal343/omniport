/**
 * Virtualized HTML5 Canvas Hex/ASCII Monitor Renderer
 * Optimized for 60 FPS rendering under high packet volume.
 */

import { FrameFlags, PinFlags, extractPinStates, type ViewportRowData } from "../../types/serial.ts";
import { type MonitorTheme, darkTheme } from "../../utils/theme.ts";
import { computeWindows, type WindowRange } from "../../ipc/viewportWindow.ts";

// Fast lookup table for hex byte strings (zero GC overhead)
export const HEX_LUT: string[] = Array.from({ length: 256 }, (_, i) =>
  i.toString(16).padStart(2, "0").toUpperCase()
);

/**
 * Mini pin indicators rendered per row. Full three-letter labels prevent the
 * old ambiguity where CTS and DCD both rendered as "C" and DSR was dropped.
 * Order matches the RS-232 control-pin strip (RTS, CTS, DTR, DSR, DCD, RI).
 */
export interface PinIndicatorDef {
  name: string;
  flag: number;
  color: string;
}

export const PIN_INDICATORS: PinIndicatorDef[] = [
  { name: "RTS", flag: PinFlags.RTS, color: "#58a6ff" },
  { name: "CTS", flag: PinFlags.CTS, color: "#3fb950" },
  { name: "DTR", flag: PinFlags.DTR, color: "#d29922" },
  { name: "DSR", flag: PinFlags.DSR, color: "#bc8cff" },
  { name: "DCD", flag: PinFlags.DCD, color: "#f778ba" },
  { name: "RI", flag: PinFlags.RI, color: "#79c0ff" },
];

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

/**
 * Computes fixed column positions for the monitor. `totalWidth` is the full
 * content width; the canvas is sized to it so that narrow windows get a real
 * horizontal scrollbar instead of silently clipping the HEX/ASCII columns.
 */
export function computeColumnLayout(containerWidth: number): HexColumnLayout {
  const indexWidth = 72;
  const timeWidth = 100;
  const endpointWidth = 96;
  const pinsWidth = 120;
  const hexWidth = 348; // 16 bytes * 20px + 8-byte gap + padding
  const asciiWidth = 152;

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
  const calc = computeWindows(scrollTop, viewportHeight, rowHeight, totalRows, overscan);
  const clampedScrollTop = calc.visible.start * rowHeight + calc.offsetInsideRowPx;
  return {
    startRow: calc.visible.start,
    endRow: calc.visible.start + calc.visible.count,
    renderStartRow: calc.render.start,
    renderEndRow: calc.render.start + calc.render.count,
    offsetY: calc.render.start * rowHeight - clampedScrollTop,
    totalHeight: totalRows > 0 && rowHeight > 0 ? totalRows * rowHeight : 0,
  };
}

/**
 * Formats a packet timestamp as an offset from the capture-session epoch.
 * The caller MUST supply the session base timestamp; defaulting to 0 rendered
 * garbage like `+1700000000.000000s` for real Unix-epoch nanosecond timestamps.
 */
export function formatTimeOffset(timestampNs: bigint, baseTimeNs: bigint): string {
  const deltaNs = timestampNs >= baseTimeNs ? timestampNs - baseTimeNs : 0n;
  const sec = Number(deltaNs / 1_000_000_000n);
  const frac = Number((deltaNs % 1_000_000_000n) / 1_000n); // microseconds
  const secText = sec < 100 ? sec.toString().padStart(2, "0") : sec.toString();
  return `+${secText}.${frac.toString().padStart(6, "0")}s`;
}

export interface RenderHexOptions {
  ctx: CanvasRenderingContext2D;
  /** Logical width of the canvas (must be >= column layout totalWidth). */
  width: number;
  /** Logical width of the visible viewport used to place the fixed columns. */
  viewportWidth?: number;
  height: number;
  rowHeight: number;
  headerHeight: number;
  scrollTop: number;
  totalRows: number;
  rows: ViewportRowData[];
  selectedRowIndex?: number | null;
  hoveredRowIndex?: number | null;
  theme?: MonitorTheme;
  /** Capture-session epoch, i.e. timestamp of frame 0. Required. */
  baseTimeNs: bigint;
  /**
   * Row window supplied by the ViewportSession. When omitted the renderer
   * falls back to its own (legacy) range calculation — used by unit tests and
   * any standalone embedding.
   */
  renderWindow?: WindowRange;
}

const MONO_FONT = '12px "JetBrains Mono", "SF Mono", "Cascadia Code", Consolas, monospace';
const HEADER_FONT = 'bold 11px -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif';
const PIN_FONT = '9px "JetBrains Mono", "SF Mono", "Cascadia Code", Consolas, monospace';

/**
 * Renders the visible slice of the Hex/ASCII monitor onto an HTML5 canvas.
 */
export function renderHexMonitor(options: RenderHexOptions): void {
  const {
    ctx,
    width,
    viewportWidth,
    height,
    rowHeight,
    headerHeight,
    scrollTop,
    totalRows,
    rows,
    selectedRowIndex = null,
    hoveredRowIndex = null,
    theme = darkTheme,
    baseTimeNs,
    renderWindow,
  } = options;

  const cols = computeColumnLayout(viewportWidth ?? width);
  const contentWidth = Math.max(width, cols.totalWidth);

  // 1. Clear background
  ctx.fillStyle = theme.background;
  ctx.fillRect(0, 0, contentWidth, height);

  // 2. Compute rows to paint. The session is the window owner; the legacy
  // in-renderer calculation remains for standalone renderer use/tests.
  const contentHeight = Math.max(0, height - headerHeight);
  const range = renderWindow
    ? {
        renderStartRow: Math.max(0, renderWindow.start),
        renderEndRow: Math.max(0, renderWindow.start) + Math.max(0, renderWindow.count),
      }
    : (() => {
        const vp = calculateViewportRange(scrollTop, contentHeight, rowHeight, totalRows);
        return { renderStartRow: vp.renderStartRow, renderEndRow: vp.renderEndRow };
      })();

  // Map input rows by rowIndex for fast random access
  const rowMap = new Map<number, ViewportRowData>();
  for (const r of rows) {
    rowMap.set(r.rowIndex, r);
  }

  // 3. Render Data Rows
  ctx.font = MONO_FONT;
  ctx.textBaseline = "middle";

  for (let idx = range.renderStartRow; idx < range.renderEndRow; idx++) {
    const y = headerHeight + (idx * rowHeight - scrollTop);
    if (y + rowHeight < headerHeight || y > height) continue;

    // Row highlights (hover & selection)
    if (selectedRowIndex === idx) {
      ctx.fillStyle = theme.selectionFill;
      ctx.fillRect(0, y, contentWidth, rowHeight);
    } else if (hoveredRowIndex === idx) {
      ctx.fillStyle = theme.hoverFill;
      ctx.fillRect(0, y, contentWidth, rowHeight);
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

    // B. Timestamp offset from session epoch
    ctx.fillStyle = theme.timestampText;
    ctx.fillText(formatTimeOffset(rowData.timestampNs, baseTimeNs), cols.timeX, textY);

    // C. Direction & Endpoint Badge
    const isTx = (rowData.flags & FrameFlags.DIR_TX) !== 0;
    const hasCrcErr = (rowData.flags & FrameFlags.ERR_CRC) !== 0;
    const dirLabel = isTx ? "TX" : "RX";
    const epLabel = `${dirLabel}:P${rowData.endpointId}${hasCrcErr ? " ERR" : ""}`;

    if (hasCrcErr) {
      ctx.fillStyle = theme.errorText;
      ctx.fillText(`!${epLabel}`, cols.endpointX, textY);
    } else {
      ctx.fillStyle = isTx ? theme.txText : theme.rxText;
      ctx.fillText(epLabel, cols.endpointX, textY);
    }

    // D. Mini Pin Indicators: full RTS/CTS/DTR/DSR/DCD/RI labels, dimmed when low.
    const pinStates = extractPinStates(rowData.pins);
    const pinHigh = [
      pinStates.rts,
      pinStates.cts,
      pinStates.dtr,
      pinStates.dsr,
      pinStates.dcd,
      pinStates.ri,
    ];
    ctx.font = PIN_FONT;
    for (let p = 0; p < PIN_INDICATORS.length; p++) {
      const indicator = PIN_INDICATORS[p];
      const high = pinHigh[p];
      ctx.fillStyle = high ? indicator.color : theme.asciiUnprintable;
      ctx.globalAlpha = high ? 1 : 0.55;
      ctx.fillText(indicator.name, cols.pinsX + p * 19, textY);
    }
    ctx.globalAlpha = 1;
    ctx.font = MONO_FONT;

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

    // E2. Truncation marker: the monitor never silently hides payload bytes.
    const truncatedBytes = payload.length - byteCount;
    if (truncatedBytes > 0) {
      const compactCount =
        truncatedBytes < 1000 ? `${truncatedBytes}` : `${Math.round(truncatedBytes / 1024)}k`;
      ctx.font = PIN_FONT;
      ctx.fillStyle = theme.headerText;
      ctx.fillText(`…+${compactCount}`, hexCursorX + 2, textY);
      ctx.font = MONO_FONT;
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
    if (truncatedBytes > 0) {
      ctx.fillStyle = theme.headerText;
      ctx.fillText("…", asciiCursorX, textY);
    }
  }

  // 3b. Empty state
  if (totalRows <= 0) {
    const centerY = headerHeight + Math.max(0, contentHeight) / 2;
    ctx.textAlign = "center";
    ctx.fillStyle = theme.headerText;
    ctx.font = 'bold 13px -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif';
    ctx.fillText("WAITING FOR CAPTURE DATA", contentWidth / 2, centerY - 8);
    ctx.fillStyle = theme.asciiUnprintable;
    ctx.font = '11px -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif';
    ctx.fillText(
      "No frames in the active session — start a route to populate the monitor.",
      contentWidth / 2,
      centerY + 12
    );
    ctx.textAlign = "start";
  }

  // 4. Render Sticky Header
  ctx.fillStyle = theme.headerBackground;
  ctx.fillRect(0, 0, contentWidth, headerHeight);

  ctx.strokeStyle = theme.borderColor;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, headerHeight - 0.5);
  ctx.lineTo(contentWidth, headerHeight - 0.5);
  ctx.stroke();

  ctx.fillStyle = theme.headerText;
  ctx.font = HEADER_FONT;
  const headerTextY = headerHeight / 2;

  ctx.fillText("FRAME #", cols.indexX, headerTextY);
  ctx.fillText("TIME OFFSET", cols.timeX, headerTextY);
  ctx.fillText("ENDPOINT", cols.endpointX, headerTextY);
  ctx.fillText("PINS", cols.pinsX, headerTextY);
  ctx.fillText("HEX DATA (16 BYTES)", cols.hexX, headerTextY);
  ctx.fillText("ASCII DUMP", cols.asciiX, headerTextY);
}
