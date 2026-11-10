import React, { useState } from "react";
import { HexMonitorCanvas } from "../HexMonitor/HexMonitorCanvas.tsx";
import { LogicStripCanvas } from "../LogicStrip/LogicStripCanvas.tsx";
import { TopologyCanvas } from "../Topology/TopologyCanvas.tsx";
import { TelemetryStripChart } from "../Telemetry/TelemetryStripChart.tsx";
import { extractPinStates } from "../../types/serial.ts";
import { getTheme, type MonitorTheme } from "../../utils/theme.ts";
import type { ViewportSession } from "../../ipc/viewportSession.ts";
import { useViewportSession } from "../../ipc/useViewportSession.ts";
import type { WindowRange } from "../../ipc/viewportWindow.ts";
import type { ViewportRowData } from "../../types/serial.ts";

export type WorkbenchViewTab = "workbench" | "monitor" | "topology" | "telemetry";

const VIEW_TABS: ReadonlyArray<readonly [WorkbenchViewTab, string]> = [
  ["workbench", "Split Workbench"],
  ["monitor", "Hex Monitor"],
  ["topology", "Visual Topology"],
  ["telemetry", "Telemetry 60Hz"],
];

export interface SerialWorkbenchProps {
  /** Single owner of the viewport window (rows, scroll, epoch, selection). */
  session: ViewportSession;
  isDark?: boolean;
  /** True when the Tauri host itself is serving seeded demo frames. */
  hostDemoMode?: boolean | null;
}

interface HexViewProps {
  session: ViewportSession;
  totalRows: number;
  rows: ViewportRowData[];
  scrollTopPx: number;
  visibleWindow: WindowRange;
  renderWindow: WindowRange;
  selectedRowIndex: number | null;
  theme: MonitorTheme;
  baseTimeNs: bigint;
  logicHeight: number;
}

/**
 * Shared Hex Monitor + synchronized logic strip view. The monitor is a pure
 * renderer over the ViewportSession snapshot; scroll/select/resize are emitted
 * back to the session as intents (single owner of the window).
 */
const HexView: React.FC<HexViewProps> = ({
  session,
  totalRows,
  rows,
  scrollTopPx,
  visibleWindow,
  renderWindow,
  selectedRowIndex,
  theme,
  baseTimeNs,
  logicHeight,
}) => (
  <div style={{ display: "flex", flexDirection: "column", width: "100%", height: "100%" }}>
    <div style={{ flex: 1, minHeight: 0, position: "relative" }}>
      <HexMonitorCanvas
        totalRows={totalRows}
        rows={rows}
        scrollTopPx={scrollTopPx}
        visibleWindow={visibleWindow}
        renderWindow={renderWindow}
        selectedRowIndex={selectedRowIndex}
        onSelectRow={(rowIndex) => session.selectRow(rowIndex)}
        onScrollByPixels={(deltaPx, deltaMode) => session.scrollByPixels(deltaPx, deltaMode)}
        onViewportResize={(viewportHeightPx) => session.setViewportHeight(viewportHeightPx)}
        theme={theme}
        baseTimeNs={baseTimeNs}
      />
    </div>
    <div className="op-logic-panel" style={{ height: `${logicHeight}px`, flexShrink: 0 }}>
      <LogicStripCanvas
        rows={rows}
        startRowIndex={visibleWindow.start}
        visibleRowCount={visibleWindow.count}
        selectedRowIndex={selectedRowIndex}
        onSelectRow={(rowIndex) => session.selectRow(rowIndex)}
        theme={theme}
        height={logicHeight}
      />
    </div>
  </div>
);

export const SerialWorkbench: React.FC<SerialWorkbenchProps> = ({
  session,
  isDark = true,
  hostDemoMode = null,
}) => {
  const state = useViewportSession(session);
  const [themeMode, setThemeMode] = useState(isDark);
  const [activeTab, setActiveTab] = useState<WorkbenchViewTab>("workbench");

  const theme = getTheme(themeMode);
  const rows = state.rows;
  const totalRows = state.totalRows;
  const selectedRowIndex = state.selectedRowIndex;
  const selectedFrame = state.selectedFrame;
  const visibleWindow = state.visibleWindow;
  const selectedPins = selectedFrame ? extractPinStates(selectedFrame.pins) : null;
  const baseTimeNs =
    state.sessionEpochNs !== 0n ? state.sessionEpochNs : (rows[0]?.timestampNs ?? 0n);
  const showsDemoData = state.sourceKind === "demo" || hostDemoMode === true;

  return (
    <div
      className="op-workbench"
      style={{
        display: "flex",
        flexDirection: "column",
        width: "100%",
        height: "100%",
        backgroundColor: theme.background,
        color: theme.asciiText,
        "--op-border": theme.borderColor,
      } as React.CSSProperties}
    >
      {/* Top Toolbar & View Tabs */}
      <div
        className="op-toolbar"
        style={{
          backgroundColor: theme.cardBackground,
          borderBottom: `1px solid ${theme.borderColor}`,
        }}
      >
        <div className="op-toolbar-left">
          <span className="op-brand" style={{ color: theme.txText }}>
            OMNIPORT
          </span>
          {showsDemoData && (
            <span
              className="op-demo-badge"
              data-testid="demo-badge"
              title={
                state.sourceKind === "demo"
                  ? "Browser fallback stream — not live hardware traffic"
                  : "Tauri host is serving its seeded demo session — not live hardware traffic"
              }
            >
              DEMO STREAM
            </span>
          )}

          {/* Navigation View Tabs */}
          <div className="op-tabs" data-testid="view-tabs">
            {VIEW_TABS.map(([tabKey, tabLabel]) => {
              const active = activeTab === tabKey;
              return (
                <button
                  key={tabKey}
                  type="button"
                  className="op-tab"
                  data-testid={`tab-${tabKey}`}
                  data-active={active}
                  aria-pressed={active}
                  onClick={() => setActiveTab(tabKey)}
                  style={{
                    backgroundColor: active ? theme.cardBackground : "transparent",
                    color: active ? theme.asciiText : theme.headerText,
                  }}
                >
                  {tabLabel}
                </button>
              );
            })}
          </div>

          <span className="op-metric" style={{ color: theme.timestampText }}>
            Total: <strong>{totalRows.toLocaleString()}</strong> frames
          </span>
          <span className="op-metric op-viewport-metric" style={{ color: theme.headerText }}>
            Viewport: [{visibleWindow.start} ..{" "}
            {visibleWindow.start + Math.max(0, visibleWindow.count - 1)}]
          </span>
        </div>

        <div className="op-actions">
          {selectedFrame && (
            <div className="op-selection" style={{ color: theme.headerText }} data-testid="frame-detail">
              <span>Frame #{selectedFrame.rowIndex}</span>
              <span>EP:{selectedFrame.endpointId}</span>
              <span>Len:{selectedFrame.payload.length}B</span>
              {selectedPins && (
                <span style={{ color: theme.txText }}>
                  PINS:{" "}
                  {[
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
            type="button"
            className="op-btn"
            data-testid="autoscroll-toggle"
            aria-pressed={state.autoScroll}
            onClick={() => session.setAutoScroll(!state.autoScroll)}
            style={{
              border: `1px solid ${theme.borderColor}`,
              backgroundColor: state.autoScroll ? theme.selectionFill : "transparent",
              color: state.autoScroll ? theme.txText : theme.headerText,
            }}
          >
            {state.autoScroll ? "● Auto-Scroll ON" : "○ Auto-Scroll OFF"}
          </button>

          <button
            type="button"
            className="op-btn"
            data-testid="theme-toggle"
            aria-pressed={!themeMode}
            title={`Switch to ${themeMode ? "light" : "dark"} theme`}
            onClick={() => setThemeMode((prev) => !prev)}
            style={{
              border: `1px solid ${theme.borderColor}`,
              backgroundColor: "transparent",
              color: theme.headerText,
            }}
          >
            {themeMode ? "☾ Dark" : "☼ Light"}
          </button>
        </div>
      </div>

      {/* Capture transport failure: never invent frames, tell the operator. */}
      {state.status === "error" && (
        <div
          className="op-banner op-banner-error"
          data-testid="capture-disconnected-banner"
          role="alert"
        >
          <span className="op-banner-title">CAPTURE DISCONNECTED</span>
          <span className="op-banner-message">
            {state.error ?? "The capture viewport source is unavailable."}
          </span>
          <button type="button" className="op-btn" onClick={() => session.refresh()}>
            RETRY
          </button>
        </div>
      )}

      {/* Main Content Area */}
      <div className="op-content">
        {activeTab === "workbench" && (
          <div style={{ display: "flex", flexDirection: "column", width: "100%", height: "100%" }}>
            {/* Top Half: Hex Monitor & Logic Strip */}
            <div style={{ flex: 1, minHeight: 0 }}>
              <HexView
                session={session}
                totalRows={totalRows}
                rows={rows}
                scrollTopPx={state.scrollTopPx}
                visibleWindow={visibleWindow}
                renderWindow={state.renderWindow}
                selectedRowIndex={selectedRowIndex}
                theme={theme}
                baseTimeNs={baseTimeNs}
                logicHeight={120}
              />
            </div>

            {/* Bottom Half: Split view between Topology and Telemetry */}
            <div
              className="op-bottom-split"
              style={{ borderTop: `1px solid ${theme.borderColor}` }}
            >
              <div className="op-topology-panel">
                <TopologyCanvas theme={theme} />
              </div>
              <div className="op-telemetry-panel">
                <TelemetryStripChart
                  height={280}
                  theme={theme}
                  rows={rows}
                  isDemo={state.sourceKind === "demo"}
                  hostDemoMode={hostDemoMode === true}
                />
              </div>
            </div>
          </div>
        )}

        {activeTab === "monitor" && (
          <HexView
            session={session}
            totalRows={totalRows}
            rows={rows}
            scrollTopPx={state.scrollTopPx}
            visibleWindow={visibleWindow}
            renderWindow={state.renderWindow}
            selectedRowIndex={selectedRowIndex}
            theme={theme}
            baseTimeNs={baseTimeNs}
            logicHeight={140}
          />
        )}

        {activeTab === "topology" && (
          <div style={{ width: "100%", height: "100%" }}>
            <TopologyCanvas theme={theme} />
          </div>
        )}

        {activeTab === "telemetry" && (
          <div style={{ width: "100%", height: "100%" }}>
            <TelemetryStripChart
              height={600}
              theme={theme}
              rows={rows}
              isDemo={state.sourceKind === "demo"}
              hostDemoMode={hostDemoMode === true}
            />
          </div>
        )}
      </div>
    </div>
  );
};
