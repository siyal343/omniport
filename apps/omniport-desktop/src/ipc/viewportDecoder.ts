/**
 * Decodes the raw binary ArrayBuffer returned by Tauri's `fetch_viewport_rows`
 * according to Section 2 of docs/ICD.md. Zero JSON parsing overhead.
 */
export interface ViewportPacketRow {
  timestampNs: bigint;
  endpointId: number;
  flags: number;
  pins: number;
  payload: Uint8Array;
}

export interface ViewportBatch {
  totalPackets: bigint;
  rows: ViewportPacketRow[];
}

export function decodeViewportBuffer(buffer: ArrayBuffer): ViewportBatch {
  const view = new DataView(buffer);
  const totalPackets = view.getBigUint64(0, true);
  const returnedRows = view.getUint32(8, true);
  const rows: ViewportPacketRow[] = [];

  let offset = 16; // Skip 16-byte batch header
  for (let i = 0; i < returnedRows; i++) {
    const timestampNs = view.getBigUint64(offset, true);
    const payloadLen = view.getUint32(offset + 16, true);
    const endpointId = view.getUint16(offset + 20, true);
    const flags = view.getUint8(offset + 22);
    const pins = view.getUint8(offset + 23);

    const payloadStart = offset + 24;
    const payload = new Uint8Array(buffer, payloadStart, payloadLen);

    rows.push({ timestampNs, endpointId, flags, pins, payload });
    offset = payloadStart + payloadLen;
  }

  return { totalPackets, rows };
}
