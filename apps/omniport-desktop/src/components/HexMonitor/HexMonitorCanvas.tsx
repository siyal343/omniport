import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { ViewportRowData } from "../../types/serial.ts";
import { renderHexMonitor, computeColumnLayout } from "./hexRenderer.ts";
import { computeScrollMax, type WindowRange } from "../../ipc/viewportWindow.ts";
import { type MonitorTheme, darkTheme } from "../../utils/theme.ts";

export type CanvasScrollDeltaMode = "pixel" | "line" | "page";

/**
 * Renderer-only Hex/ASCII monitor canvas.
 *
 * It no longer owns the viewport window: `scrollTopPx`, `visibleWindow`,
 * `renderWindow` and `rows` arrive from the ViewportSession. The canvas draws
 * them, keeps local hover/canvas-size state, and emits intents (scroll, select,
 * resize) upward. Native `{passive:false}` wheel handling, mount-once
 * ResizeObserver, rAF dirty-paint and horizontal scrolling are preserved.
 */
export interface HexMonitorCanvasProps {
  totalRows: number;
  rows: ViewportRowData[];
  scrollTopPx: number;
  visibleWindow: WindowRange;
  renderWindow: WindowRange;
  selectedRowIndex?: number | null;
  onSelectRow?: (rowIndex: number) => void;
  onScrollByPixels?: (deltaPx: number, deltaMode: CanvasScrollDeltaMode) => void;
  onViewportResize?: (viewportHeightPx: number) => void;
  theme?: MonitorTheme;
  rowHeight?: number;
  headerHeight?: number;
  /** Capture-session epoch (timestamp of frame 0) for the TIME OFFSET column. */
  baseTimeNs: bigint;
}

interface LatestState {
  scrollTopPx: number;
  totalRows: number;
  rowHeight: number;
  headerHeight: number;
  dimensions: { width: number; height: number };
  hBar: number;
  selectedRowIndex: number | null;
  hoveredRow: number | null;
  theme: MonitorTheme;
  rows: ViewportRowData[];
  renderWindow: WindowRange;
  baseTimeNs: bigint;
}

export const HexMonitorCanvas: React.FC<HexMonitorCanvasProps> = ({
  totalRows,
  rows,
  scrollTopPx,
  visibleWindow,
  renderWindow,
  selectedRowIndex = null,
  onSelectRow,
  onScrollByPixels,
  onViewportResize,
  theme = darkTheme,
  rowHeight = 22,
  headerHeight = 28,
  baseTimeNs,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const scrollerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [hoveredRow, setHoveredRow] = useState<number | null>(null);
  const [dimensions, setDimensions] = useState({ width: 0, height: 0 });
  const [hBar, setHBar] = useState(0);

  // Latest values for native event handlers / rAF loops that must not be
  // torn down and re-created on every scroll tick.
  const latestRef = useRef<LatestState>({
    scrollTopPx,
    totalRows,
    rowHeight,
    headerHeight,
    dimensions,
    hBar,
    selectedRowIndex,
    hoveredRow,
    theme,
    rows,
    renderWindow,
    baseTimeNs,
  });
  latestRef.current = {
    scrollTopPx,
    totalRows,
    rowHeight,
    headerHeight,
    dimensions,
    hBar,
    selectedRowIndex,
    hoveredRow,
    theme,
    rows,
    renderWindow,
    baseTimeNs,
  };

  const onScrollByPixelsRef = useRef(onScrollByPixels);
  onScrollByPixelsRef.current = onScrollByPixels;
  const onViewportResizeRef = useRef(onViewportResize);
  onViewportResizeRef.current = onViewportResize;

  const visibleHeight = Math.max(0, dimensions.height - hBar);
  const contentHeight = Math.max(0, visibleHeight - headerHeight);
  const maxScroll = computeScrollMax(totalRows, rowHeight, contentHeight);

  // Report the scrollable row-area height to the window owner (session).
  useEffect(() => {
    onViewportResizeRef.current?.(contentHeight);
  }, [contentHeight]);

  // ResizeObserver: mounted once. Deferring the state update to rAF avoids
  // "ResizeObserver loop completed with undelivered notifications".
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    let raf = 0;
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (!entry) return;
      const width = Math.floor(entry.contentRect.width);
      const height = Math.floor(entry.contentRect.height);
      if (width <= 0 || height <= 0) return;

      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        setDimensions((prev) =>
          prev.width === width && prev.height === height ? prev : { width, height }
        );
      });
    });

    observer.observe(container);
    return () => {
      observer.disconnect();
      cancelAnimationFrame(raf);
    };
  }, []);

  // Measure the horizontal scrollbar so the canvas can reserve its height.
  useLayoutEffect(() => {
    const el = scrollerRef.current;
    if (!el) return;
    const measured = Math.max(0, el.offsetHeight - el.clientHeight);
    setHBar((prev) => (prev === measured ? prev : measured));
  });

  // Native non-passive wheel listener: React's onWheel is registered as a
  // passive root listener, so calling preventDefault() there is ignored and
  // logs a browser console error. Horizontal intent is left to the native
  // scroller so the HEX/ASCII columns can be reached at narrow widths.
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const onWheel = (event: WheelEvent) => {
      if (Math.abs(event.deltaX) > Math.abs(event.deltaY)) return;
      event.preventDefault();

      const latest = latestRef.current;
      const availableHeight = Math.max(
        0,
        latest.dimensions.height - latest.hBar - latest.headerHeight
      );
      if (availableHeight <= 0) return;

      const deltaMode: CanvasScrollDeltaMode =
        event.deltaMode === 1 ? "line" : event.deltaMode === 2 ? "page" : "pixel";
      onScrollByPixelsRef.current?.(event.deltaY, deltaMode);
    };

    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  // 60 FPS paint loop with a dirty check: skip the whole render pass when
  // nothing changed so an idle monitor does not burn CPU at 60 Hz.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d", { alpha: false });
    if (!ctx) return;

    let animFrameId = 0;
    const dpr = window.devicePixelRatio || 1;
    let lastPaintKey = "";
    let lastRows: ViewportRowData[] | null = null;

    const paint = () => {
      const latest = latestRef.current;
      const layout = computeColumnLayout(Math.max(latest.dimensions.width, 1));
      const canvasHeight = Math.max(0, latest.dimensions.height - latest.hBar);
      if (latest.dimensions.width <= 0 || canvasHeight <= 0) return;

      const targetW = Math.round(layout.totalWidth * dpr);
      const targetH = Math.round(canvasHeight * dpr);
      if (canvas.width !== targetW || canvas.height !== targetH) {
        canvas.width = targetW;
        canvas.height = targetH;
        canvas.style.width = `${layout.totalWidth}px`;
        canvas.style.height = `${canvasHeight}px`;
      }

      const paintKey = [
        latest.dimensions.width,
        canvasHeight,
        latest.scrollTopPx,
        latest.renderWindow.start,
        latest.renderWindow.count,
        latest.totalRows,
        latest.selectedRowIndex,
        latest.hoveredRow,
        latest.theme.background,
        latest.baseTimeNs.toString(),
      ].join("|");

      if (latest.rows === lastRows && paintKey === lastPaintKey) return;
      lastRows = latest.rows;
      lastPaintKey = paintKey;

      ctx.save();
      ctx.scale(dpr, dpr);
      renderHexMonitor({
        ctx,
        width: layout.totalWidth,
        viewportWidth: latest.dimensions.width,
        height: canvasHeight,
        rowHeight: latest.rowHeight,
        headerHeight: latest.headerHeight,
        scrollTop: latest.scrollTopPx,
        totalRows: latest.totalRows,
        rows: latest.rows,
        selectedRowIndex: latest.selectedRowIndex,
        hoveredRowIndex: latest.hoveredRow,
        theme: latest.theme,
        baseTimeNs: latest.baseTimeNs,
        renderWindow: latest.renderWindow,
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

  // Mouse click selection
  const handleClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const y = e.clientY - rect.top;
    if (y < headerHeight) return;

    const clickedRow = Math.floor((scrollTopPx + (y - headerHeight)) / rowHeight);
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
    const row = Math.floor((scrollTopPx + (y - headerHeight)) / rowHeight);
    if (row >= 0 && row < totalRows) {
      if (hoveredRow !== row) setHoveredRow(row);
    } else if (hoveredRow !== null) {
      setHoveredRow(null);
    }
  };

  const handleMouseLeave = () => {
    if (hoveredRow !== null) setHoveredRow(null);
  };

  const scrollbarThumbHeight =
    maxScroll > 0 ? Math.max(20, (contentHeight / (totalRows * rowHeight)) * contentHeight) : 0;
  const scrollbarThumbTop =
    maxScroll > 0
      ? (scrollTopPx / maxScroll) * Math.max(0, contentHeight - scrollbarThumbHeight)
      : 0;

  // The visible window is exposed read-only for tests/tools; painting uses the
  // (wider) render window from the session.
  return (
    <div
      ref={containerRef}
      data-testid="hex-monitor"
      data-visible-start={visibleWindow.start}
      data-visible-count={visibleWindow.count}
      style={{
        position: "relative",
        width: "100%",
        height: "100%",
        overflow: "hidden",
        backgroundColor: theme.background,
        userSelect: "none",
      }}
    >
      <div
        ref={scrollerRef}
        data-testid="hex-hscroll"
        style={{
          width: "100%",
          height: "100%",
          overflowX: "auto",
          overflowY: "hidden",
          scrollbarWidth: "thin",
        }}
      >
        <canvas
          ref={canvasRef}
          onClick={handleClick}
          onMouseMove={handleMouseMove}
          onMouseLeave={handleMouseLeave}
          aria-label="Serial hex and ASCII monitor"
          style={{ display: "block", cursor: "pointer" }}
        />
      </div>

      {/* Virtual Scrollbar Track */}
      {maxScroll > 0 && (
        <div
          style={{
            position: "absolute",
            right: 0,
            top: headerHeight,
            width: 8,
            height: Math.max(0, visibleHeight - headerHeight),
            backgroundColor: "rgba(0,0,0,0.2)",
            pointerEvents: "none",
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
