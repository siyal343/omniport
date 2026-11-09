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

    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const w = entry.contentRect.width;
        if (w > 0) {
          setWidth(Math.floor(w));
        }
      }
    });

    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d", { alpha: false });
    if (!ctx) return;

    let animFrameId: number;
    const dpr = window.devicePixelRatio || 1;

    const render = () => {
      const p = propsRef.current;
      const targetW = Math.round(p.width * dpr);
      const targetH = Math.round(p.height * dpr);

      if (canvas.width !== targetW || canvas.height !== targetH) {
        canvas.width = targetW;
        canvas.height = targetH;
        canvas.style.width = `${p.width}px`;
        canvas.style.height = `${p.height}px`;
      }

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
        style={{ display: "block", cursor: "crosshair" }}
      />
    </div>
  );
};
