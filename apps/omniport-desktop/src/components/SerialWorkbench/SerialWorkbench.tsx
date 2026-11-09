import React, { useState, useCallback, useEffect } from "react";
import { HexMonitorCanvas } from "../HexMonitor/HexMonitorCanvas.tsx";
import { LogicStripCanvas } from "../LogicStrip/LogicStripCanvas.tsx";
import { type ViewportRowData, extractPinStates } from "../../types/serial.ts";
import { getTheme, type MonitorTheme } from "../../utils/theme.ts";
import type { ViewportBatch } from "../../ipc/viewportDecoder.ts";

export interface SerialWorkbenchProps {
  totalPackets: bigint;
  onFetchViewport: (startRow: number, rowCount: number) => Promise<ViewportBatch>;
  isDark?: boolean;
}

export const SerialWorkbench: React.FC<SerialWorkbenchProps> = ({
  totalPackets,
  onFetchViewport,
  isDark = true,
}) => {
  const [rows, setRows] = useState<ViewportRowData[]>([]);
  const [visibleRange, setVisibleRange] = useState({ start: 0, count: 50 });
  const [selectedRowIndex, setSelectedRowIndex] = useState<number | null>(null);
  const [autoScroll, setAutoScroll] = useState(false);
  const [themeMode, setThemeMode] = useState(isDark);

  const theme = getTheme(themeMode);

  // Viewport fetch callback triggered by virtual scrollbar or resize
  const handleViewportChange = useCallback(
    async (startRow: number, rowCount: number) => {
      setVisibleRange({ start: startRow, count: rowCount });
      try {
        const batch = await onFetchViewport(startRow, rowCount);
        const mappedRows: ViewportRowData[] = batch.rows.map((r, i) => ({
          rowIndex: startRow + i,
          timestampNs: r.timestampNs,
          endpointId: r.endpointId,
          flags: r.flags,
          pins: r.pins,
          payload: r.payload,
        }));
        setRows(mappedRows);
      } catch (err) {
        console.error("Failed to fetch viewport batch:", err);
      }
    },
    [onFetchViewport]
  );

  // Selected packet information
  const selectedRow = rows.find((r) => r.rowIndex === selectedRowIndex);
  const selectedPins = selectedRow ? extractPinStates(selectedRow.pins) : null;

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        width: "100%",
        height: "100%",
        backgroundColor: theme.background,
        color: theme.asciiText,
        fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
      }}
    >
      {/* Top Toolbar */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          padding: "6px 12px",
          backgroundColor: theme.cardBackground,
          borderBottom: `1px solid ${theme.borderColor}`,
          fontSize: "12px",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: "16px" }}>
          <span style={{ fontWeight: 600, letterSpacing: "0.5px" }}>OMNIPORT MONITOR</span>
          <span style={{ color: theme.timestampText }}>
            Total: <strong>{totalPackets.toLocaleString()}</strong> frames
          </span>
          <span style={{ color: theme.headerText }}>
            Viewport: [{visibleRange.start} .. {visibleRange.start + visibleRange.count}]
          </span>
        </div>

        <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
          {selectedRow && (
            <div style={{ display: "flex", alignItems: "center", gap: "8px", color: theme.headerText }}>
              <span>Frame #{selectedRow.rowIndex}</span>
              <span>EP:{selectedRow.endpointId}</span>
              <span>Len:{selectedRow.payload.length}B</span>
              {selectedPins && (
                <span style={{ color: theme.txText }}>
                  PINS: {[
                    selectedPins.rts && "RTS",
                    selectedPins.cts && "CTS",
                    selectedPins.dtr && "DTR",
                    selectedPins.dsr && "DSR",
                    selectedPins.dcd && "DCD",
                  ]
                    .filter(Boolean)
                    .join("|") || "NONE"}
                </span>
              )}
            </div>
          )}

          <button
            onClick={() => setAutoScroll(!autoScroll)}
            style={{
              padding: "4px 8px",
              fontSize: "11px",
              borderRadius: "4px",
              border: `1px solid ${theme.borderColor}`,
              backgroundColor: autoScroll ? theme.selectionFill : "transparent",
              color: autoScroll ? theme.txText : theme.headerText,
              cursor: "pointer",
            }}
          >
            {autoScroll ? "● Auto-Scroll ON" : "○ Auto-Scroll OFF"}
          </button>

          <button
            onClick={() => setThemeMode(!themeMode)}
            style={{
              padding: "4px 8px",
              fontSize: "11px",
              borderRadius: "4px",
              border: `1px solid ${theme.borderColor}`,
              backgroundColor: "transparent",
              color: theme.headerText,
              cursor: "pointer",
            }}
          >
            {themeMode ? "☼ Light" : "☾ Dark"}
          </button>
        </div>
      </div>

      {/* Main Hex/ASCII Monitor Canvas */}
      <div style={{ flex: 1, minHeight: 0, position: "relative" }}>
        <HexMonitorCanvas
          totalRows={Number(totalPackets)}
          rows={rows}
          onViewportChange={handleViewportChange}
          selectedRowIndex={selectedRowIndex}
          onSelectRow={setSelectedRowIndex}
          theme={theme}
          autoScroll={autoScroll}
        />
      </div>

      {/* Synchronized Control-Pin Logic Strip */}
      <div style={{ height: "140px", flexShrink: 0 }}>
        <LogicStripCanvas
          rows={rows}
          startRowIndex={visibleRange.start}
          visibleRowCount={visibleRange.count}
          selectedRowIndex={selectedRowIndex}
          onSelectRow={setSelectedRowIndex}
          theme={theme}
          height={140}
        />
      </div>
    </div>
  );
};
