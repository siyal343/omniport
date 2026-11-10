import React, { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { SerialWorkbench } from "./components/SerialWorkbench/SerialWorkbench.tsx";
import { fetchViewport } from "./ipc/viewportClient.ts";
import { createDemoViewport } from "./ipc/demoStream.ts";
import { createViewportSession } from "./ipc/useViewportSession.ts";
import type { ViewportSource, ViewportSourceKind } from "./ipc/viewportSession.ts";
import { parseCaptureStatus, type HostCaptureStatus } from "./ipc/captureStatus.ts";

export const App: React.FC = () => {
  const isTauri =
    typeof window !== "undefined" &&
    ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);
  const sourceKind: ViewportSourceKind = isTauri ? "tauri" : "demo";

  // Tauri adapter: NO demo fallback on IPC failure — the session surfaces
  // status "error" and the workbench renders CAPTURE DISCONNECTED. Browser dev
  // uses the single demo adapter and stays labelled DEMO STREAM.
  const source = useMemo<ViewportSource>(() => {
    if (isTauri) {
      return { read: (startRow: number, rowCount: number) => fetchViewport(startRow, rowCount) };
    }
    return {
      read: async (startRow: number, rowCount: number) =>
        createDemoViewport({ startRow, rowCount }),
    };
  }, [isTauri]);

  // One window owner for the whole app lifetime. Not disposed on unmount:
  // under React StrictMode the effect cleanup would kill the session between
  // the double-mount, and the SPA has a single page-long lifetime anyway.
  const session = useMemo(
    () =>
      createViewportSession(source, {
        rowHeightPx: 22,
        overscanRows: 10,
        sourceKind,
        initialTotalRows: 0,
      }),
    [source, sourceKind]
  );

  // Host truth about demo seeding (Tauri only). Missing command / older host is
  // "unknown", never an error.
  const [hostStatus, setHostStatus] = useState<HostCaptureStatus | null>(null);
  const readCaptureStatus = useCallback(async () => {
    try {
      const raw = await invoke<unknown>("capture_status");
      setHostStatus(parseCaptureStatus(raw));
    } catch (err) {
      setHostStatus(null);
      console.warn("capture_status unavailable (older host?):", err);
    }
  }, []);

  useEffect(() => {
    if (!isTauri) return;
    void readCaptureStatus();
  }, [isTauri, readCaptureStatus]);

  return (
    <div style={{ width: "100vw", height: "100vh", overflow: "hidden" }}>
      <SerialWorkbench
        session={session}
        isDark
        hostDemoMode={hostStatus?.demoMode ?? null}
      />
    </div>
  );
};
