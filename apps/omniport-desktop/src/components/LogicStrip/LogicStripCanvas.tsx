import React, { useEffect, useRef, useState } from "react";
import type { ViewportRowData } from "../../types/serial.ts";
import { renderLogicStrip } from "./logicStripRenderer.ts";
import { type MonitorTheme, darkTheme } from "../../utils/theme.ts";

export interface LogicStripCanvasProps {
  rows: ViewportRowData[];
  startRowIndex: number;
  visibleRowCount: number;
  selectedRowIndex?: number | null;
  onSelectRow?: (rowIndex: number) => void;
  theme?: MonitorTheme;
  height?: number;
}

export const LogicStripCanvas: React.FC<LogicStripCanvasProps> = ({
  rows,
  startRowIndex,
  visibleRowCount,
  selectedRowIndex = null,
  onSelectRow,
  theme = darkTheme,
  height = 140,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [width, setWidth] = useState(800);

  const propsRef = useRef({
    rows,
    startRowIndex,
    visibleRowCount,
    selectedRowIndex,
    theme,
    width,
    height,
  });

  propsRef.current = {
    rows,
    startRowIndex,
    visibleRowCount,
    selectedRowIndex,
    theme,
    width,
    height,
  };

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    let raf = 0;
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (!entry) return;
      const measured = Math.floor(entry.contentRect.width);
      if (measured <= 0) return;
      // Defer the state update out of the observer delivery to avoid layout
      // feedback loops ("ResizeObserver loop completed...").
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        setWidth((prev) => (prev === measured ? prev : measured));
      });
    });

    observer.observe(container);
    return () => {
      observer.disconnect();
      cancelAnimationFrame(raf);
    };
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d", { alpha: false });
    if (!ctx) return;

    let animFrameId = 0;
    const dpr = window.devicePixelRatio || 1;
    let lastRows: ViewportRowData[] | null = null;
    let lastKey = "";

    const paint = () => {
      const p = propsRef.current;
      const targetW = Math.round(p.width * dpr);
      const targetH = Math.round(p.height * dpr);

      if (canvas.width !== targetW || canvas.height !== targetH) {
        canvas.width = targetW;
        canvas.height = targetH;
        canvas.style.width = `${p.width}px`;
        canvas.style.height = `${p.height}px`;
      }

      const key = [p.width, p.height, p.startRowIndex, p.visibleRowCount, p.selectedRowIndex, p.theme.background].join("|");
      if (p.rows === lastRows && key === lastKey) return;
      lastRows = p.rows;
      lastKey = key;

      ctx.save();
      ctx.scale(dpr, dpr);

      renderLogicStrip({
        ctx,
        width: p.width,
        height: p.height,
        rows: p.rows,
        startRowIndex: p.startRowIndex,
        visibleRowCount: p.visibleRowCount,
        selectedRowIndex: p.selectedRowIndex,
        theme: p.theme,
      });

      ctx.restore();
    };

    const render = () => {
      paint();
      animFrameId = requestAnimationFrame(render);
    };
    animFrameId = requestAnimationFrame(render);
    return () => cancelAnimationFrame(animFrameId);
  }, []);

  const handleClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const labelWidth = 64;
    const rect = e.currentTarget.getBoundingClientRect();
    const x = e.clientX - rect.left;
    if (x < labelWidth) return;

    const waveformWidth = Math.max(10, width - labelWidth - 16);
    const effectiveCount = Math.max(1, visibleRowCount);
    const stepX = waveformWidth / effectiveCount;
    const colIdx = Math.floor((x - labelWidth) / stepX);

    if (colIdx >= 0 && colIdx < effectiveCount) {
      const clickedRow = startRowIndex + colIdx;
      onSelectRow?.(clickedRow);
    }
  };

  return (
    <div
      ref={containerRef}
      data-testid="logic-strip"
      style={{
        width: "100%",
        height: `${height}px`,
        overflow: "hidden",
        backgroundColor: theme.background,
        borderTop: `1px solid ${theme.borderColor}`,
      }}
    >
      <canvas
        ref={canvasRef}
        onClick={handleClick}
        aria-label="Synchronized modem control pin logic waveforms"
        style={{ display: "block", cursor: "crosshair" }}
      />
    </div>
  );
};
