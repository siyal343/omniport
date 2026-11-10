import React, { useEffect, useMemo, useRef, useState } from "react";
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import { TelemetryRingBuffer } from "./telemetryRingBuffer.ts";
import { summarizeViewportRows } from "./telemetryFromRows.ts";
import { type MonitorTheme, darkTheme } from "../../utils/theme.ts";
import type { ViewportRowData } from "../../types/serial.ts";

export interface TelemetryStripChartProps {
  height?: number;
  capacity?: number;
  /** Theme tokens from the workbench so the chart follows dark/light mode. */
  theme?: MonitorTheme;
  /** Frames fetched for the current viewport — the live telemetry source. */
  rows?: ViewportRowData[];
  /**
   * True only for the browser demo stream (ipc/demoStream.ts). In demo mode the
   * strip synthesizes a waveform but is clearly labelled as demo data.
   */
  isDemo?: boolean;
  /**
   * True when the Tauri host is serving its seeded demo session. The strip
   * still derives metrics from fetched viewport rows, but labels the source as
   * demo so it never claims live traffic while the toolbar shows DEMO STREAM.
   */
  hostDemoMode?: boolean;
}

const WINDOW_SECONDS = 10;
const LIVE_SAMPLE_INTERVAL_MS = 250;

/**
 * Renders the 60 Hz telemetry strip. The ring buffer is recreated only when the
 * capacity changes; the uPlot instance only when the chart is (re)mounted.
 */
export const TelemetryStripChart: React.FC<TelemetryStripChartProps> = ({
  height = 240,
  capacity = 600, // 10 seconds of 60 Hz telemetry
  theme = darkTheme,
  rows = [],
  isDemo = false,
  hostDemoMode = false,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const uplotRef = useRef<uPlot | null>(null);
  const rowsRef = useRef<ViewportRowData[]>(rows);
  rowsRef.current = rows;
  const isDemoRef = useRef(isDemo);
  isDemoRef.current = isDemo;
  const ringBuffer = useMemo(() => {
    const rb = new TelemetryRingBuffer(capacity);
    // Seed two finite points so uPlot has a valid x domain before the first tick.
    rb.push(0, 0, 0, 0, 0);
    rb.push(0.016, 0, 0, 0, 0);
    return rb;
  }, [capacity]);
  const [activeRate, setActiveRate] = useState(0);
  const [activeThroughput, setActiveThroughput] = useState(0);
  const [activeJitter, setActiveJitter] = useState(0);
  const [initFailed, setInitFailed] = useState(false);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const initialData = ringBuffer.getLinearData();
    const resolveHeight = () => Math.max(container.clientHeight || height - 34, 100);

    const opts: uPlot.Options = {
      width: Math.max(container.clientWidth || 480, 200),
      height: resolveHeight(),
      cursor: {
        sync: { key: "omniport" },
        drag: { setScale: false },
      },
      legend: {
        show: false, // Custom high-frequency stat readout is rendered above
      },
      scales: {
        x: {
          time: false,
          // Scrolling 10-second strip window anchored to the newest sample.
          range: (_u, _min, _max) => {
            const xs = _u.data[0];
            const last = xs.length > 0 ? xs[xs.length - 1] : 0;
            const end = Math.max(last, WINDOW_SECONDS);
            return [end - WINDOW_SECONDS, end];
          },
        },
        y: { auto: true },
        ms: { auto: true },
        err: { auto: true },
      },
      axes: [
        {
          stroke: theme.headerText,
          grid: { stroke: theme.borderColor, width: 1 },
          ticks: { stroke: theme.borderColor, width: 1 },
          values: (_u, vals) => vals.map((v) => `+${v.toFixed(1)}s`),
        },
        {
          scale: "y",
          stroke: theme.headerText,
          grid: { stroke: theme.borderColor, width: 1 },
          ticks: { stroke: theme.borderColor, width: 1 },
        },
        {
          side: 1,
          scale: "ms",
          stroke: "#d29922",
          grid: { show: false },
          values: (_u, vals) => vals.map((v) => `${v.toFixed(1)}ms`),
        },
      ],
      series: [
        {
          label: "Time",
        },
        {
          label: "Packet Rate (pkt/s)",
          scale: "y",
          stroke: theme.txText,
          width: 2,
          points: { show: false },
        },
        {
          label: "Throughput (KB/s)",
          scale: "y",
          stroke: theme.rxText,
          width: 1.5,
          points: { show: false },
        },
        {
          label: "Jitter (ms)",
          scale: "ms",
          stroke: "#d29922",
          width: 1.5,
          points: { show: false },
        },
        {
          label: "CRC Errors",
          scale: "err",
          stroke: theme.errorText,
          width: 2,
          points: { show: true, size: 4 },
        },
      ],
    };

    let plot: uPlot | null = null;
    try {
      plot = new uPlot(opts, initialData, container);
      uplotRef.current = plot;
      setInitFailed(false);
    } catch (err) {
      console.warn("uPlot initialization deferred:", err);
      setInitFailed(true);
    }

    let roRaf = 0;
    const ro = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (!entry || !plot) return;
      cancelAnimationFrame(roRaf);
      roRaf = requestAnimationFrame(() => {
        const width = Math.floor(entry.contentRect.width);
        const chartHeight = Math.max(100, Math.floor(entry.contentRect.height));
        if (width > 0) {
          plot!.setSize({ width, height: chartHeight });
        }
      });
    });
    ro.observe(container);

    // 60 Hz update loop. Paused while the tab is hidden so a backgrounded
    // window does not keep synthesizing frames and repainting.
    let animId = 0;
    let lastNow = performance.now();
    let simSec = 0.016;
    let lastUiUpdate = 0;
    let lastLiveSample = Number.NEGATIVE_INFINITY;

    const tick = (now: number) => {
      const deltaSec = Math.max(0, (now - lastNow) / 1000);
      lastNow = now;

      if (!document.hidden) {
        simSec += Math.min(deltaSec, 0.25);

        if (isDemoRef.current) {
          // Demo mode only (browser fallback): periodic traffic bursts + jitter.
          const burst = Math.sin(simSec * 1.5) > 0.3 ? 1.0 : 0.2;
          const rate = 120 * burst + (Math.random() * 20 - 10);
          const throughput = (rate * 96) / 1024; // KB/s
          const jitter = Math.max(0, 2.5 + (Math.random() - 0.5) * 3);
          const crc = Math.random() < 0.02 ? 1 : 0;

          ringBuffer.push(simSec, rate, throughput, jitter, crc);

          if (plot) {
            // resetScales=true is required, otherwise the x domain freezes at
            // the two seed samples and every later point is off-screen.
            plot.setData(ringBuffer.getLinearData(), true);
          }

          if (now - lastUiUpdate > 250) {
            setActiveRate(Math.round(rate));
            setActiveThroughput(Number(throughput.toFixed(1)));
            setActiveJitter(Number(jitter.toFixed(1)));
            lastUiUpdate = now;
          }
        } else if (now - lastLiveSample >= LIVE_SAMPLE_INTERVAL_MS) {
          // Live mode: sample the telemetry derived from real fetched frames.
          const sample = summarizeViewportRows(rowsRef.current);
          ringBuffer.push(
            simSec,
            sample.packetRate,
            sample.throughputKbps,
            sample.jitterMs,
            sample.crcErrors
          );
          if (plot) {
            plot.setData(ringBuffer.getLinearData(), true);
          }
          setActiveRate(Math.round(sample.packetRate));
          setActiveThroughput(Number(sample.throughputKbps.toFixed(1)));
          setActiveJitter(Number(sample.jitterMs.toFixed(1)));
          lastLiveSample = now;
        }
      }

      animId = requestAnimationFrame(tick);
    };

    animId = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(animId);
      cancelAnimationFrame(roRaf);
      ro.disconnect();
      plot?.destroy();
      uplotRef.current = null;
    };
  }, [height, capacity, theme, ringBuffer]);

  return (
    <div
      data-testid="telemetry-chart"
      aria-label="Real-time telemetry strip chart"
      style={{
        display: "flex",
        flexDirection: "column",
        width: "100%",
        height: `${height}px`,
        backgroundColor: theme.background,
        borderTop: `1px solid ${theme.borderColor}`,
        overflow: "hidden",
      }}
    >
      {/* Telemetry Stat Header */}
      <div
        className="op-telemetry-header"
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          flexWrap: "wrap",
          gap: "2px 12px",
          padding: "4px 12px",
          backgroundColor: theme.cardBackground,
          borderBottom: `1px solid ${theme.borderColor}`,
          fontSize: 11,
          fontFamily: 'SFMono-Regular, Consolas, "Liberation Mono", Menlo, monospace',
          fontVariantNumeric: "tabular-nums",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", flexWrap: "wrap", gap: "2px 12px" }}>
          <span style={{ fontWeight: 600, color: theme.asciiText }}>
            REAL-TIME TELEMETRY STRIP
          </span>
          <span
            data-testid="telemetry-source"
            style={{
              color: isDemo || hostDemoMode ? "#d29922" : theme.rxText,
              border: `1px solid ${isDemo || hostDemoMode ? "#d29922" : theme.rxText}`,
              borderRadius: 3,
              padding: "0 4px",
              fontSize: 9,
              letterSpacing: "0.4px",
            }}
          >
            {isDemo
              ? "DEMO STREAM"
              : hostDemoMode
                ? "DEMO · VIEWPORT-DERIVED"
                : "LIVE · VIEWPORT-DERIVED"}
          </span>
          <span style={{ color: theme.txText }}>● Rate: {activeRate} pkt/s</span>
          <span style={{ color: theme.rxText }}>● Throughput: {activeThroughput} KB/s</span>
          <span style={{ color: "#d29922" }}>● Jitter: {activeJitter.toFixed(1)}ms</span>
        </div>
        <div className="op-telemetry-meta" style={{ color: theme.headerText, fontSize: 10 }}>
          uPlot TypedArray RingBuffer ({capacity} samples)
        </div>
      </div>

      {/* Chart Canvas mount point */}
      <div ref={containerRef} style={{ flex: 1, minHeight: 0, position: "relative" }}>
        {initFailed && (
          <div
            style={{
              position: "absolute",
              inset: 0,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              color: theme.headerText,
              fontSize: 12,
            }}
          >
            Telemetry chart unavailable — uPlot failed to initialize.
          </div>
        )}
      </div>
    </div>
  );
};
