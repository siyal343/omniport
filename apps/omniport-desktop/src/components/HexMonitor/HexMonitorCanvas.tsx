import React, { useEffect, useRef, useState, useCallback } from "react";
import type { ViewportRowData } from "../../types/serial.ts";
import { renderHexMonitor, calculateViewportRange } from "./hexRenderer.ts";
import { type MonitorTheme, darkTheme } from "../../utils/theme.ts";

export interface HexMonitorCanvasProps {
  totalRows: number;
  rows: ViewportRowData[];
  onViewportChange: (startRow: number, rowCount: number) => void;
  selectedRowIndex?: number | null;
  onSelectRow?: (rowIndex: number) => void;
  theme?: MonitorTheme;
  rowHeight?: number;
  headerHeight?: number;
  autoScroll?: boolean;
}

export const HexMonitorCanvas: React.FC<HexMonitorCanvasProps> = ({
  totalRows,
  rows,
  onViewportChange,
  selectedRowIndex = null,
  onSelectRow,
  theme = darkTheme,
  rowHeight = 22,
  headerHeight = 28,
  autoScroll = false,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [hoveredRow, setHoveredRow] = useState<number | null>(null);
  const [dimensions, setDimensions] = useState({ width: 800, height: 400 });

  // Keep latest props in ref for 60 FPS requestAnimationFrame loop
  const propsRef = useRef({
    totalRows,
    rows,
    scrollTop,
    selectedRowIndex,
    hoveredRow,
    theme,
    rowHeight,
    headerHeight,
    width: dimensions.width,
    height: dimensions.height,
  });

  propsRef.current = {
    totalRows,
    rows,
    scrollTop,
    selectedRowIndex,
    hoveredRow,
    theme,
    rowHeight,
    headerHeight,
    width: dimensions.width,
    height: dimensions.height,
  };

  // Synchronize viewport request when scroll or dimensions change
  const triggerViewportFetch = useCallback(
    (currentScrollTop: number, currentHeight: number, currentTotal: number) => {
      const contentHeight = Math.max(0, currentHeight - headerHeight);
      const vp = calculateViewportRange(
        currentScrollTop,
        contentHeight,
        rowHeight,
        currentTotal,
        10 // 10 rows overscan
      );
      const count = Math.max(1, vp.renderEndRow - vp.renderStartRow);
      onViewportChange(vp.renderStartRow, count);
    },
    [headerHeight, rowHeight, onViewportChange]
  );

  // ResizeObserver for high-DPI canvas
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const { width, height } = entry.contentRect;
        if (width > 0 && height > 0) {
          setDimensions({ width: Math.floor(width), height: Math.floor(height) });
          triggerViewportFetch(scrollTop, height, totalRows);
        }
      }
    });

    observer.observe(container);
    return () => observer.disconnect();
  }, [scrollTop, totalRows, triggerViewportFetch]);

  // Handle auto-scroll to bottom
  useEffect(() => {
    if (autoScroll && totalRows > 0) {
      const contentHeight = Math.max(0, dimensions.height - headerHeight);
      const maxScroll = Math.max(0, totalRows * rowHeight - contentHeight);
      setScrollTop(maxScroll);
      triggerViewportFetch(maxScroll, dimensions.height, totalRows);
    }
  }, [autoScroll, totalRows, dimensions.height, headerHeight, rowHeight, triggerViewportFetch]);

  // Smooth 60 FPS Rendering Loop
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

      renderHexMonitor({
        ctx,
        width: p.width,
        height: p.height,
        rowHeight: p.rowHeight,
        headerHeight: p.headerHeight,
        scrollTop: p.scrollTop,
        totalRows: p.totalRows,
        rows: p.rows,
        selectedRowIndex: p.selectedRowIndex,
        hoveredRowIndex: p.hoveredRow,
        theme: p.theme,
      });

      ctx.restore();
      animFrameId = requestAnimationFrame(render);
    };

    animFrameId = requestAnimationFrame(render);
    return () => cancelAnimationFrame(animFrameId);
  }, []);

  // Mouse wheel virtual scrolling
  const handleWheel = (e: React.WheelEvent) => {
    e.preventDefault();
    const contentHeight = Math.max(0, dimensions.height - headerHeight);
    const maxScroll = Math.max(0, totalRows * rowHeight - contentHeight);
    const nextScroll = Math.min(maxScroll, Math.max(0, scrollTop + e.deltaY));
    if (nextScroll !== scrollTop) {
      setScrollTop(nextScroll);
      triggerViewportFetch(nextScroll, dimensions.height, totalRows);
    }
  };

  // Mouse click selection
  const handleClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const y = e.clientY - rect.top;
    if (y < headerHeight) return;

    const clickedRow = Math.floor((scrollTop + (y - headerHeight)) / rowHeight);
    if (clickedRow >= 0 && clickedRow < totalRows) {
      onSelectRow?.(clickedRow);
    }
  };

  // Mouse move hover highlight
  const handleMouseMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const y = e.clientY - rect.top;
    if (y < headerHeight) {
      if (hoveredRow !== null) setHoveredRow(null);
      return;
    }
    const row = Math.floor((scrollTop + (y - headerHeight)) / rowHeight);
    if (row >= 0 && row < totalRows) {
      if (hoveredRow !== row) setHoveredRow(row);
    } else if (hoveredRow !== null) {
      setHoveredRow(null);
    }
  };

  const handleMouseLeave = () => {
    if (hoveredRow !== null) setHoveredRow(null);
  };

  const contentHeight = Math.max(0, dimensions.height - headerHeight);
  const maxScroll = Math.max(0, totalRows * rowHeight - contentHeight);
  const scrollbarThumbHeight = maxScroll > 0
    ? Math.max(20, (contentHeight / (totalRows * rowHeight)) * contentHeight)
    : 0;
  const scrollbarThumbTop = maxScroll > 0
    ? (scrollTop / maxScroll) * (contentHeight - scrollbarThumbHeight)
    : 0;

  return (
    <div
      ref={containerRef}
      onWheel={handleWheel}
      style={{
        position: "relative",
        width: "100%",
        height: "100%",
        overflow: "hidden",
        backgroundColor: theme.background,
        userSelect: "none",
      }}
    >
      <canvas
        ref={canvasRef}
        onClick={handleClick}
        onMouseMove={handleMouseMove}
        onMouseLeave={handleMouseLeave}
        style={{ display: "block", cursor: "pointer" }}
      />

      {/* Virtual Scrollbar Track */}
      {maxScroll > 0 && (
        <div
          style={{
            position: "absolute",
            right: 0,
            top: headerHeight,
            width: 8,
            height: contentHeight,
            backgroundColor: "rgba(0,0,0,0.2)",
          }}
        >
          <div
            style={{
              position: "absolute",
              top: scrollbarThumbTop,
              right: 1,
              width: 6,
              height: scrollbarThumbHeight,
              borderRadius: 3,
              backgroundColor: "rgba(255,255,255,0.25)",
            }}
          />
        </div>
      )}
    </div>
  );
};
