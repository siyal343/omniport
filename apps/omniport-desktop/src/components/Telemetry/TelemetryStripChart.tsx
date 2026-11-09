import React, { useEffect, useRef, useState } from "react";
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import { TelemetryRingBuffer } from "./telemetryRingBuffer.ts";

export interface TelemetryStripChartProps {
  height?: number;
  capacity?: number;
  isDark?: boolean;
}

export const TelemetryStripChart: React.FC<TelemetryStripChartProps> = ({
  height = 240,
  capacity = 600, // 10 seconds of 60 Hz telemetry
  isDark = true,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const uplotRef = useRef<uPlot | null>(null);
  const ringBufferRef = useRef<TelemetryRingBuffer>(new TelemetryRingBuffer(capacity));
  const [activeRate, setActiveRate] = useState(0);
  const [activeThroughput, setActiveThroughput] = useState(0);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const ring = ringBufferRef.current;
    const initialData = ring.getLinearData();

    const opts: uPlot.Options = {
      width: container.clientWidth || 800,
      height: height - 36, // Subtract top stat header
      cursor: {
        sync: { key: "omniport" },
        drag: { setScale: false },
      },
      legend: {
        show: false, // We render custom high-frequency stat readout above
      },
      scales: {
        x: { time: false },
        y: { auto: true },
        ms: { auto: true },
        err: { auto: true },
      },
      axes: [
        {
          stroke: isDark ? "#8b949e" : "#57606a",
          grid: { stroke: isDark ? "#21262d" : "#e1e4e8", width: 1 },
          ticks: { stroke: isDark ? "#30363d" : "#d0d7de", width: 1 },
          values: (_u, vals) => vals.map((v) => `+${v.toFixed(1)}s`),
        },
        {
          scale: "y",
          stroke: isDark ? "#8b949e" : "#57606a",
          grid: { stroke: isDark ? "#21262d" : "#e1e4e8", width: 1 },
          ticks: { stroke: isDark ? "#30363d" : "#d0d7de", width: 1 },
        },
        {
          side: 1,
          scale: "ms",
          stroke: "#d29922",
          grid: { show: false },
          values: (_u, vals) => vals.map((v) => `${v.toFixed(0)}ms`),
        },
      ],
      series: [
        {
          label: "Time",
        },
        {
          label: "Packet Rate (pkt/s)",
          scale: "y",
          stroke: "#58a6ff",
          width: 2,
          points: { show: false },
        },
        {
          label: "Throughput (KB/s)",
          scale: "y",
          stroke: "#3fb950",
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
          stroke: "#f85149",
          width: 2,
          points: { show: true, size: 4 },
        },
      ],
    };

    const plot = new uPlot(opts, initialData, container);
    uplotRef.current = plot;

    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const w = entry.contentRect.width;
        if (w > 0) {
          plot.setSize({ width: Math.floor(w), height: height - 36 });
        }
      }
    });
    ro.observe(container);

    // 60 Hz Update Loop
    let animId: number;
    let baseTime = performance.now();
    let simSec = 0;
    let lastUiUpdate = 0;

    const tick = (now: number) => {
      const deltaSec = (now - baseTime) / 1000;
      simSec += 0.016;

      // Realistic simulated telemetry waveform: periodic traffic bursts + Box-Muller jitter
      const burst = Math.sin(simSec * 1.5) > 0.3 ? 1.0 : 0.2;
      const rate = 120 * burst + (Math.random() * 20 - 10);
      const throughput = (rate * 96) / 1024; // KB/s
      const jitter = Math.max(0, 2.5 + (Math.random() - 0.5) * 3);
      const crc = Math.random() < 0.02 ? 1 : 0;

      ring.push(simSec, rate, throughput, jitter, crc);

      if (plot) {
        plot.setData(ring.getLinearData(), false);
      }

      if (now - lastUiUpdate > 250) {
        setActiveRate(Math.round(rate));
        setActiveThroughput(Number(throughput.toFixed(1)));
        lastUiUpdate = now;
      }

      animId = requestAnimationFrame(tick);
    };

    animId = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(animId);
      ro.disconnect();
      plot.destroy();
      uplotRef.current = null;
    };
  }, [height, capacity, isDark]);

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        width: "100%",
        height: `${height}px`,
        backgroundColor: "#0d1117",
        borderTop: "1px solid #30363d",
        overflow: "hidden",
        fontFamily: 'SFMono-Regular, Consolas, "Liberation Mono", Menlo, monospace',
      }}
    >
      {/* Telemetry Stat Header */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          padding: "4px 12px",
          backgroundColor: "#161b22",
          borderBottom: "1px solid #30363d",
          fontSize: 11,
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
          <span style={{ fontWeight: 600, color: "#e6edf3" }}>
            REAL-TIME TELEMETRY STRIP (60 Hz)
          </span>
          <span style={{ color: "#58a6ff" }}>● Rate: {activeRate} pkt/s</span>
          <span style={{ color: "#3fb950" }}>● Throughput: {activeThroughput} KB/s</span>
          <span style={{ color: "#d29922" }}>● Jitter: 2.5ms</span>
        </div>
        <div style={{ color: "#8b949e", fontSize: 10 }}>
          uPlot TypedArray RingBuffer ({capacity} samples)
        </div>
      </div>

      {/* Chart Canvas mount point */}
      <div ref={containerRef} style={{ flex: 1, minHeight: 0, position: "relative" }} />
    </div>
  );
};
