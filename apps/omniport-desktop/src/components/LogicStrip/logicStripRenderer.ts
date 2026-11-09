/**
 * Synchronized DTR/RTS/CTS/DCD Digital Logic Timing Strip Renderer
 * Renders digital square waveforms aligned to serial packet stream.
 */

import { PinFlags, type ViewportRowData } from "../../types/serial.ts";
import { type MonitorTheme, darkTheme } from "../../utils/theme.ts";

export interface PinChannelDef {
  name: string;
  flag: number;
  color: string;
}

export const LOGIC_CHANNELS: PinChannelDef[] = [
  { name: "RTS", flag: PinFlags.RTS, color: "#58a6ff" }, // Cyan/Blue
  { name: "CTS", flag: PinFlags.CTS, color: "#3fb950" }, // Green
  { name: "DTR", flag: PinFlags.DTR, color: "#d29922" }, // Amber/Yellow
  { name: "DSR", flag: PinFlags.DSR, color: "#bc8cff" }, // Purple
  { name: "DCD", flag: PinFlags.DCD, color: "#f778ba" }, // Pink
  { name: "RI",  flag: PinFlags.RI,  color: "#79c0ff" }, // Sky Blue
];

export interface RenderLogicStripOptions {
  ctx: CanvasRenderingContext2D;
  width: number;
  height: number;
  rows: ViewportRowData[];
  startRowIndex: number;
  visibleRowCount: number;
  selectedRowIndex?: number | null;
  theme?: MonitorTheme;
}

/**
 * Renders the multi-channel digital logic waveforms onto the canvas
 */
export function renderLogicStrip(options: RenderLogicStripOptions): void {
  const {
    ctx,
    width,
    height,
    rows,
    startRowIndex,
    visibleRowCount,
    selectedRowIndex = null,
    theme = darkTheme,
  } = options;

  // Clear canvas
  ctx.fillStyle = theme.background;
  ctx.fillRect(0, 0, width, height);

  const channelCount = LOGIC_CHANNELS.length;
  if (channelCount === 0 || height <= 0 || width <= 0) return;

  const labelWidth = 64;
  const waveformWidth = Math.max(10, width - labelWidth - 16);
  const channelHeight = height / channelCount;

  // Map rows by index
  const rowMap = new Map<number, ViewportRowData>();
  for (const r of rows) {
    rowMap.set(r.rowIndex, r);
  }

  const effectiveCount = Math.max(1, visibleRowCount);
  const stepX = waveformWidth / effectiveCount;

  // Render each pin channel
  for (let chIdx = 0; chIdx < channelCount; chIdx++) {
    const channel = LOGIC_CHANNELS[chIdx];
    const topY = chIdx * channelHeight;
    const bottomY = topY + channelHeight;
    const highY = topY + channelHeight * 0.22;
    const lowY = topY + channelHeight * 0.78;

    // Background track line
    ctx.strokeStyle = theme.borderColor;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(0, bottomY - 0.5);
    ctx.lineTo(width, bottomY - 0.5);
    ctx.stroke();

    // Channel Label Box
    ctx.fillStyle = theme.headerBackground;
    ctx.fillRect(0, topY, labelWidth, channelHeight - 1);

    ctx.strokeStyle = theme.borderColor;
    ctx.strokeRect(0, topY, labelWidth, channelHeight - 1);

    ctx.fillStyle = channel.color;
    ctx.font = 'bold 11px "JetBrains Mono", Consolas, monospace';
    ctx.textBaseline = "middle";
    ctx.fillText(channel.name, 12, topY + channelHeight / 2);

    // Baseline low reference wire
    ctx.strokeStyle = theme.pinLow;
    ctx.setLineDash([2, 4]);
    ctx.beginPath();
    ctx.moveTo(labelWidth, lowY);
    ctx.lineTo(width, lowY);
    ctx.stroke();
    ctx.setLineDash([]); // Reset dash

    // Draw square digital waveform
    let lastLevel: boolean | null = null;
    ctx.strokeStyle = channel.color;
    ctx.lineWidth = 2;
    ctx.beginPath();

    for (let i = 0; i < effectiveCount; i++) {
      const rowIndex = startRowIndex + i;
      const row = rowMap.get(rowIndex);
      const isHigh = row ? (row.pins & channel.flag) !== 0 : false;

      const xStart = labelWidth + i * stepX;
      const xEnd = labelWidth + (i + 1) * stepX;
      const currentY = isHigh ? highY : lowY;

      if (lastLevel === null) {
        ctx.moveTo(xStart, currentY);
      } else if (lastLevel !== isHigh) {
        // Vertical step edge transition
        ctx.lineTo(xStart, currentY);
      }

      // Horizontal active signal line
      ctx.lineTo(xEnd, currentY);
      lastLevel = isHigh;
    }
    ctx.stroke();

    // Subtle gradient fill under active HIGH pulse
    ctx.fillStyle = `${channel.color}20`; // ~12% opacity hex
    ctx.beginPath();
    let inHigh = false;
    let highStartX = 0;

    for (let i = 0; i < effectiveCount; i++) {
      const rowIndex = startRowIndex + i;
      const row = rowMap.get(rowIndex);
      const isHigh = row ? (row.pins & channel.flag) !== 0 : false;
      const xStart = labelWidth + i * stepX;
      const xEnd = labelWidth + (i + 1) * stepX;

      if (isHigh) {
        if (!inHigh) {
          inHigh = true;
          highStartX = xStart;
        }
        if (i === effectiveCount - 1) {
          ctx.fillRect(highStartX, highY, xEnd - highStartX, lowY - highY);
        }
      } else {
        if (inHigh) {
          inHigh = false;
          ctx.fillRect(highStartX, highY, xStart - highStartX, lowY - highY);
        }
      }
    }
  }

  // Draw synchronized cursor line for selected row
  if (selectedRowIndex !== null && selectedRowIndex >= startRowIndex && selectedRowIndex < startRowIndex + effectiveCount) {
    const cursorX = labelWidth + (selectedRowIndex - startRowIndex + 0.5) * stepX;
    ctx.strokeStyle = theme.pinEdge;
    ctx.lineWidth = 1.5;
    ctx.setLineDash([4, 2]);
    ctx.beginPath();
    ctx.moveTo(cursorX, 0);
    ctx.lineTo(cursorX, height);
    ctx.stroke();
    ctx.setLineDash([]);
  }
}
