/**
 * Pure row-mapping helpers for the binary viewport wire format.
 *
 * Kept free of React/DOM imports so `node:test` can exercise the real code
 * (the previous topology test never imported a module at all).
 */

import type { ViewportBatch } from "./viewportDecoder.ts";
import type { ViewportRowData } from "../types/serial.ts";

/**
 * Maps a decoded batch onto absolute row indices.
 *
 * The backend returns rows starting at the requested `startRow`; if a request
 * is answered with fewer rows than requested (e.g. clamped at the end of the
 * capture), only the returned prefix is mapped instead of fabricating
 * non-existent frames.
 */
export function mapBatchToRows(batch: ViewportBatch, requestedStartRow: number): ViewportRowData[] {
  return batch.rows.map((row, i) => ({
    rowIndex: requestedStartRow + i,
    timestampNs: row.timestampNs,
    endpointId: row.endpointId,
    flags: row.flags,
    pins: row.pins,
    payload: row.payload,
  }));
}

/**
 * Selects the oldest timestamp in a batch, used to seed the session epoch for
 * the monitor's TIME OFFSET column. Returns null for an empty batch.
 */
export function batchEpochNs(batch: ViewportBatch): bigint | null {
  const first = batch.rows[0];
  return first ? first.timestampNs : null;
}

/**
 * Picks the epoch to display offsets against: the earliest frame observed so
 * far. Scrolling backwards to an older frame moves the epoch back so the
 * offset always starts at +00.000000s for the oldest visible data.
 */
export function nextSessionEpochNs(current: bigint | null, candidate: bigint | null): bigint | null {
  if (candidate === null) return current;
  if (current === null) return candidate;
  return candidate < current ? candidate : current;
}
