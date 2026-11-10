#!/usr/bin/env node
// Independent oracle for the OmniPort viewport wire format (ICD Section 2).
//
// This generator is intentionally written from the ICD text alone — it does not
// import, call, or mirror any Rust or TypeScript implementation. Its output
// (`viewport_v1.json`) is the contract both language implementations must
// reproduce:
//
//   Rust  : crates/omniport-capture/tests/viewport_contract.rs  (encode)
//   TS    : apps/omniport-desktop/src/ipc/viewportDecodeFixture.test.js (decode)
//
// If either implementation drifts from the ICD, one of those tests goes red.
//
// Regenerate with: node fixtures/generate_viewport_fixture.mjs

import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const HERE = dirname(fileURLToPath(import.meta.url));

// ---------------------------------------------------------------------------
// ICD Section 1: dual-mmap index entry (24 bytes, little endian)
// ---------------------------------------------------------------------------
const ROW_HEADER_BYTES = 24;
const BATCH_HEADER_BYTES = 16;

/** @typedef {{timestamp_ns: bigint, data_offset: bigint, endpoint_id: number, flags: number, pins: number, payload: Buffer}} Row */

function u64le(v) {
  const b = Buffer.alloc(8);
  b.writeBigUInt64LE(BigInt(v));
  return b;
}

function encodeBatch(totalPackets, rows) {
  const parts = [u64le(totalPackets)];
  const returned = Buffer.alloc(4);
  returned.writeUInt32LE(rows.length);
  parts.push(returned);
  parts.push(Buffer.alloc(4)); // reserved / zero

  for (const row of rows) {
    const payloadLen = Buffer.alloc(4);
    payloadLen.writeUInt32LE(row.payload.length);
    const endpointId = Buffer.alloc(2);
    endpointId.writeUInt16LE(row.endpoint_id);
    parts.push(u64le(row.timestamp_ns));
    parts.push(u64le(row.data_offset));
    parts.push(payloadLen);
    parts.push(endpointId);
    parts.push(Buffer.from([row.flags, row.pins]));
    parts.push(row.payload);
  }
  return Buffer.concat(parts);
}

/** Builds the full store from the case's event list, then applies start_row/row_count. */
function wireFor(caseDef) {
  const events = caseDef.events.map((e) => ({
    timestamp_ns: BigInt(e.timestamp_ns),
    data_offset: 0n, // filled below
    endpoint_id: e.endpoint_id,
    flags: e.flags,
    pins: e.pins,
    payload: Buffer.from(e.payload_hex, "hex"),
  }));

  // session.dat grows by each payload; data_offset is the cumulative byte offset.
  let cursor = 0n;
  for (const ev of events) {
    ev.data_offset = cursor;
    cursor += BigInt(ev.payload.length);
  }

  const total = BigInt(events.length);
  const start = BigInt(caseDef.start_row);
  let returned = [];
  if (start < total) {
    const end = (start + BigInt(caseDef.row_count)) < total ? (start + BigInt(caseDef.row_count)) : total;
    for (let i = start; i < end; i++) returned.push(events[Number(i)]);
  }
  return encodeBatch(total, returned);
}

function toCase(id, comment, def) {
  const wire = wireFor(def);
  // The fixture's `rows` are the rows the decoder must produce for this wire.
  const events = def.events.map((e) => ({ ...e }));
  const total = BigInt(events.length);
  const start = BigInt(def.start_row);
  const expected = [];
  if (start < total) {
    const end = start + BigInt(def.row_count) < total ? start + BigInt(def.row_count) : total;
    let dataOffset = 0n;
    const offsets = events.map((e) => {
      const at = dataOffset;
      dataOffset += BigInt((e.payload_hex.length / 2) | 0);
      return at;
    });
    for (let i = start; i < end; i++) {
      expected.push({
        timestamp_ns: events[Number(i)].timestamp_ns.toString(),
        endpoint_id: events[Number(i)].endpoint_id,
        flags: events[Number(i)].flags,
        pins: events[Number(i)].pins,
        payload_hex: events[Number(i)].payload_hex,
        data_offset: offsets[Number(i)].toString(),
      });
    }
  }

  return {
    id,
    comment,
    start_row: def.start_row.toString(),
    row_count: def.row_count,
    total_packets: events.length.toString(),
    events: events.map((e, i) => ({
      index: i,
      timestamp_ns: e.timestamp_ns.toString(),
      endpoint_id: e.endpoint_id,
      flags: e.flags,
      pins: e.pins,
      payload_hex: e.payload_hex,
    })),
    wire_hex: wire.toString("hex"),
    wire_len: wire.length,
    rows: expected,
  };
}

const CASES = [];

CASES.push(
  toCase(
    "empty_store",
    "No frames captured: header only, all zero.",
    { start_row: 0, row_count: 10, events: [] }
  )
);

CASES.push(
  toCase(
    "out_of_range",
    "Store has one frame, viewport starts past the end: total is reported, no rows returned.",
    {
      start_row: 2,
      row_count: 5,
      events: [
        { timestamp_ns: "1000", endpoint_id: 1, flags: 1, pins: 5, payload_hex: "aa55" },
      ],
    }
  )
);

CASES.push(
  toCase(
    "slice_middle",
    "Slice of a three-frame store: only the requested rows appear, data_offset still points into session.dat.",
    {
      start_row: 1,
      row_count: 2,
      events: [
        { timestamp_ns: "1700000000000000000", endpoint_id: 1, flags: 1, pins: 1, payload_hex: "414243" },
        { timestamp_ns: "1700000000005000000", endpoint_id: 2, flags: 0, pins: 20, payload_hex: "4445" },
        { timestamp_ns: "1700000000010000000", endpoint_id: 1, flags: 2, pins: 8, payload_hex: "46474849" },
      ],
    }
  )
);

CASES.push(
  toCase(
    "full_page_bitcode",
    "Every pin/flag bit crossing the seam: empty payload, zero bytes, 0xFF bytes, u64-max timestamp.",
    {
      start_row: 0,
      row_count: 10,
      events: [
        {
          timestamp_ns: "1700000000000000000",
          endpoint_id: 1,
          flags: 0x01, // DIR_TX
          pins: 0x05, // RTS | DTR
          payload_hex: "4f4d4e49504f52545f4652414d455f303030300d0a", // 20 bytes
        },
        {
          timestamp_ns: "1700000000000000001",
          endpoint_id: 65535,
          flags: 0x10, // ERR_CRC
          pins: 0x10, // DCD
          payload_hex: "",
        },
        {
          timestamp_ns: "1700000000000000002",
          endpoint_id: 2,
          flags: 0x30, // ERR_CRC | FAULT_INJECT
          pins: 0x38, // DSR | DCD | RI
          payload_hex: "00000000",
        },
        {
          timestamp_ns: "18446744073709551615",
          endpoint_id: 7,
          flags: 0x40, // PIN_CHANGE
          pins: 0x7f, // every defined pin bit (RTS|CTS|DTR|DSR|DCD|RI|BREAK); bit 7 is reserved-zero per ICD 1.1
          payload_hex: "ffffffffffffffff",
        },
      ],
    }
  )
);

CASES.push(
  toCase(
    "clamped_count",
    "row_count larger than the store: returned_rows is clamped to total_packets.",
    {
      start_row: 0,
      row_count: 4096,
      events: [
        { timestamp_ns: "5", endpoint_id: 3, flags: 0, pins: 0, payload_hex: "cc" },
        { timestamp_ns: "6", endpoint_id: 4, flags: 0x04, pins: 0x20, payload_hex: "dddddddd" },
      ],
    }
  )
);

CASES.push(
  toCase(
    "absurd_start_row",
    "start_row = u64::MAX must not overflow or panic; no rows are returned.",
    {
      start_row: "18446744073709551615",
      row_count: 1,
      events: [
        { timestamp_ns: "9", endpoint_id: 1, flags: 0, pins: 0, payload_hex: "01" },
      ],
    }
  )
);

const fixture = {
  schema: "omniport.viewport.v1",
  frozen: true,
  generated_by: "fixtures/generate_viewport_fixture.mjs (independent ICD Section 2 oracle)",
  batch_header_bytes: BATCH_HEADER_BYTES,
  row_header_bytes: ROW_HEADER_BYTES,
  endianness: "little",
  description:
    "Wire format from internal_docs/ICD.md Section 2: [total_packets: u64][returned_rows: u32][reserved: u32], then returned_rows * ([FrameIndexEntry: 24B][payload: payload_len bytes]). u64 values are decimal strings so JavaScript cannot lose precision.",
  cases: CASES,
};

const outPath = join(HERE, "viewport_v1.json");
writeFileSync(outPath, JSON.stringify(fixture, null, 2) + "\n");
console.log(`wrote ${outPath} (${CASES.length} cases)`);
for (const c of CASES) console.log(`  ${c.id}: wire_len=${c.wire_len} rows=${c.rows.length}`);
