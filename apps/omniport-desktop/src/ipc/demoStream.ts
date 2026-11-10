/**
 * Browser demo stream adapter.
 *
 * This is the single frontend source of synthetic frames. It is used ONLY when
 * the app is running outside Tauri in browser dev mode (`vite dev`). The
 * first 25 frames mirror the host-side `FixtureCaptureStore` naming so both
 * demo layers stay recognisable, while the remaining frames give the virtual
 * scrollbar enough data to exercise real scrolling.
 *
 * Anything that renders this data must label it as DEMO — operators must never
 * mistake it for live traffic.
 */

import { FrameFlags, PinFlags } from "../types/serial.ts";
import type { ViewportBatch } from "./viewportDecoder.ts";

export const DEMO_TOTAL_PACKETS = 2500n;
export const DEMO_EPOCH_NS = 1_700_000_000_000_000_000n;
export const DEMO_FRAME_INTERVAL_NS = 5_000_000n; // 5 ms between frames

/** Number of leading demo frames that mirror the host seed session. */
export const DEMO_SEED_FRAME_COUNT = 25;
export const DEMO_SEED_FRAME_PREFIX = "OMNIPORT_INITIAL_FRAME_";

export interface DemoViewportOptions {
  startRow: number;
  rowCount: number;
}

export function createDemoViewport({ startRow, rowCount }: DemoViewportOptions): ViewportBatch {
  const from = Math.max(0, Math.floor(startRow));
  const to = Math.min(Number(DEMO_TOTAL_PACKETS), from + Math.max(0, rowCount));
  const rows = [];

  for (let i = from; i < to; i++) {
    let pins = PinFlags.RTS | PinFlags.DTR;
    if (i % 6 === 0) pins |= PinFlags.CTS;
    if (i % 10 === 0) pins |= PinFlags.DCD;
    if (i % 14 === 0) pins &= ~PinFlags.RTS;
    if (i % 17 === 0) pins |= PinFlags.DSR;
    if (i % 23 === 0) pins |= PinFlags.RI;

    const isTx = i % 2 === 0;
    const isErr = i % 29 === 0;
    let flags: number = isTx ? FrameFlags.DIR_TX : 0;
    if (isErr) flags |= FrameFlags.ERR_CRC;

    const message =
      i < DEMO_SEED_FRAME_COUNT
        ? `${DEMO_SEED_FRAME_PREFIX}${i.toString().padStart(3, "0")}`
        : isErr
          ? "ERR_CRC_CHECKSUM"
          : isTx
            ? `TX_COMMAND_SERIAL_${i}`
            : `RX_TELEMETRY_RESPONSE_${i}`;
    const payload = new Uint8Array(Array.from(message, (char) => char.charCodeAt(0)));

    rows.push({
      timestampNs: DEMO_EPOCH_NS + BigInt(i) * DEMO_FRAME_INTERVAL_NS,
      endpointId: isTx ? 1 : 2,
      flags,
      pins,
      payload,
    });
  }

  return { totalPackets: DEMO_TOTAL_PACKETS, rows };
}
