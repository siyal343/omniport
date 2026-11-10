/**
 * Derives telemetry sample values from the frames actually fetched for the
 * current viewport, instead of synthesizing an unrelated waveform.
 *
 * The Tauri telemetry command does not exist yet, so in Tauri mode the strip
 * chart samples these viewport-derived values. Kept dependency-free (no React
 * or DOM imports) so `node:test` can exercise the real math.
 */

import { FrameFlags, type ViewportRowData } from "../../types/serial.ts";

export interface TelemetrySample {
  /** Frames per second across the fetched window. */
  packetRate: number;
  /** Payload bytes per second, in KiB/s. */
  throughputKbps: number;
  /** Standard deviation of inter-frame gaps, in milliseconds. */
  jitterMs: number;
  /** Number of frames carrying the CRC error flag. */
  crcErrors: number;
}

export const EMPTY_TELEMETRY_SAMPLE: TelemetrySample = {
  packetRate: 0,
  throughputKbps: 0,
  jitterMs: 0,
  crcErrors: 0,
};

export function summarizeViewportRows(rows: readonly ViewportRowData[]): TelemetrySample {
  if (rows.length === 0) return { ...EMPTY_TELEMETRY_SAMPLE };

  let bytes = 0;
  let crcErrors = 0;
  let minNs = rows[0].timestampNs;
  let maxNs = rows[0].timestampNs;

  for (const row of rows) {
    bytes += row.payload.length;
    if ((row.flags & FrameFlags.ERR_CRC) !== 0) crcErrors += 1;
    if (row.timestampNs < minNs) minNs = row.timestampNs;
    if (row.timestampNs > maxNs) maxNs = row.timestampNs;
  }

  const spanNs = maxNs - minNs;
  const spanSec = Number(spanNs) / 1_000_000_000;

  // n frames span n-1 inter-frame intervals.
  const packetRate = spanSec > 0 ? (rows.length - 1) / spanSec : 0;
  const throughputKbps = spanSec > 0 ? bytes / 1024 / spanSec : 0;

  let jitterMs = 0;
  if (rows.length > 1) {
    const deltas: number[] = [];
    for (let i = 1; i < rows.length; i++) {
      const deltaNs = rows[i].timestampNs - rows[i - 1].timestampNs;
      const deltaMs = Number(deltaNs) / 1_000_000;
      if (deltaMs >= 0 && Number.isFinite(deltaMs)) deltas.push(deltaMs);
    }
    if (deltas.length > 0) {
      const mean = deltas.reduce((sum, value) => sum + value, 0) / deltas.length;
      const variance =
        deltas.reduce((sum, value) => sum + (value - mean) * (value - mean), 0) / deltas.length;
      jitterMs = Math.sqrt(variance);
    }
  }

  return { packetRate, throughputKbps, jitterMs, crcErrors };
}
