import { invoke } from '@tauri-apps/api/core';
import { decodeViewportBuffer, ViewportBatch } from './viewportDecoder';

/**
 * Fetches visible rows from Tauri backend via raw binary IPC and decodes them with zero JSON overhead.
 */
export async function fetchViewport(
  startRow: number | bigint,
  rowCount: number
): Promise<ViewportBatch> {
  // In a test/mock environment where Tauri isn't running, return an empty batch
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) {
    return { totalPackets: 0n, rows: [] };
  }

  const rawBytes = await invoke<ArrayBuffer | Uint8Array>('fetch_viewport_rows', {
    startRow: Number(startRow),
    rowCount,
  });

  return decodeViewportBuffer(rawBytes);
}
