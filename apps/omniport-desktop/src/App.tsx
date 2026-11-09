import React, { useState, useEffect, useCallback } from "react";
import { SerialWorkbench } from "./components/SerialWorkbench/SerialWorkbench.tsx";
import { fetchViewport } from "./ipc/viewportClient.ts";
import { type ViewportBatch } from "./ipc/viewportDecoder.ts";
import { PinFlags, FrameFlags } from "./types/serial.ts";

export const App: React.FC = () => {
  const [totalPackets, setTotalPackets] = useState<bigint>(0n);
  const isTauri = typeof window !== "undefined" && ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);

  // Fallback synthetic stream generator when running in standard browser dev mode
  const mockFetchViewport = useCallback(
    async (startRow: number, rowCount: number): Promise<ViewportBatch> => {
      const mockTotal = 2500n;
      const rows = [];
      const baseTime = 1_700_000_000_000_000_000n;

      const endRow = Math.min(Number(mockTotal), startRow + rowCount);
      for (let i = startRow; i < endRow; i++) {
        let pins = PinFlags.RTS | PinFlags.DTR;
        if (i % 6 === 0) pins |= PinFlags.CTS;
        if (i % 10 === 0) pins |= PinFlags.DCD;
        if (i % 14 === 0) pins &= ~PinFlags.RTS;

        const isTx = i % 2 === 0;
        const isErr = i % 29 === 0;
        let flags = isTx ? FrameFlags.DIR_TX : 0;
        if (isErr) flags |= FrameFlags.ERR_CRC;

        const msg = isErr
          ? "ERR_CRC_CHECKSUM"
          : isTx
          ? `TX_COMMAND_SERIAL_${i}`
          : `RX_TELEMETRY_RESPONSE_${i}`;
        const payload = new Uint8Array(Array.from(msg, (c) => c.charCodeAt(0)));

        rows.push({
          timestampNs: baseTime + BigInt(i * 5_000_000), // +5ms intervals
          endpointId: isTx ? 1 : 2,
          flags,
          pins,
          payload,
        });
      }

      return { totalPackets: mockTotal, rows };
    },
    []
  );

  const handleFetchViewport = useCallback(
    async (startRow: number, rowCount: number): Promise<ViewportBatch> => {
      if (isTauri) {
        try {
          const batch = await fetchViewport(startRow, rowCount);
          setTotalPackets(batch.totalPackets);
          return batch;
        } catch {
          return mockFetchViewport(startRow, rowCount);
        }
      } else {
        const batch = await mockFetchViewport(startRow, rowCount);
        setTotalPackets(batch.totalPackets);
        return batch;
      }
    },
    [isTauri, mockFetchViewport]
  );

  useEffect(() => {
    // Initial fetch to get packet count
    handleFetchViewport(0, 50).catch(console.error);
  }, [handleFetchViewport]);

  return (
    <div style={{ width: "100vw", height: "100vh", overflow: "hidden" }}>
      <SerialWorkbench
        totalPackets={totalPackets}
        onFetchViewport={handleFetchViewport}
        isDark={true}
      />
    </div>
  );
};
