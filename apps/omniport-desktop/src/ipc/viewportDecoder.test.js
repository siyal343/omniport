import test from 'node:test';
import assert from 'node:assert';
import { decodeViewportBuffer } from './viewportDecoder.ts';

test('decodeViewportBuffer parses valid ICD Section 2 binary stream', () => {
  // Construct 16B header + 2 packet rows
  // Packet 0: 4B payload ("TEST")
  // Packet 1: 5B payload ("HELLO")
  const p0Payload = Buffer.from('TEST');
  const p1Payload = Buffer.from('HELLO');
  const row0Size = 24 + p0Payload.length;
  const row1Size = 24 + p1Payload.length;
  const totalSize = 16 + row0Size + row1Size;

  const buffer = new ArrayBuffer(totalSize);
  const view = new DataView(buffer);

  // Header: total=100n, returned=2, reserved=0
  view.setBigUint64(0, 100n, true);
  view.setUint32(8, 2, true);
  view.setUint32(12, 0, true);

  // Row 0
  let offset = 16;
  view.setBigUint64(offset, 1_500_000_000n, true); // timestamp
  view.setBigUint64(offset + 8, 0n, true); // data_offset
  view.setUint32(offset + 16, p0Payload.length, true);
  view.setUint16(offset + 20, 42, true); // endpoint_id
  view.setUint8(offset + 22, 0b0000_0001); // flags
  view.setUint8(offset + 23, 0b0000_0101); // pins: RTS | DTR
  new Uint8Array(buffer, offset + 24).set(p0Payload);

  // Row 1
  offset += row0Size;
  view.setBigUint64(offset, 1_600_000_000n, true); // timestamp
  view.setBigUint64(offset + 8, 100n, true);
  view.setUint32(offset + 16, p1Payload.length, true);
  view.setUint16(offset + 20, 84, true); // endpoint_id
  view.setUint8(offset + 22, 0);
  view.setUint8(offset + 23, 0b0000_0010); // pins: CTS
  new Uint8Array(buffer, offset + 24).set(p1Payload);

  // Decode from ArrayBuffer
  const batch = decodeViewportBuffer(buffer);
  assert.strictEqual(batch.totalPackets, 100n);
  assert.strictEqual(batch.rows.length, 2);

  // Verify Row 0
  assert.strictEqual(batch.rows[0].timestampNs, 1_500_000_000n);
  assert.strictEqual(batch.rows[0].endpointId, 42);
  assert.strictEqual(batch.rows[0].flags, 1);
  assert.strictEqual(batch.rows[0].pins, 5);
  assert.deepStrictEqual(Array.from(batch.rows[0].payload), Array.from(p0Payload));

  // Verify Row 1
  assert.strictEqual(batch.rows[1].timestampNs, 1_600_000_000n);
  assert.strictEqual(batch.rows[1].endpointId, 84);
  assert.strictEqual(batch.rows[1].flags, 0);
  assert.strictEqual(batch.rows[1].pins, 2);
  assert.deepStrictEqual(Array.from(batch.rows[1].payload), Array.from(p1Payload));

  // Decode from Uint8Array view
  const uint8View = new Uint8Array(buffer);
  const batchFromView = decodeViewportBuffer(uint8View);
  assert.strictEqual(batchFromView.totalPackets, 100n);
  assert.strictEqual(batchFromView.rows.length, 2);
});

test('decodeViewportBuffer handles edge cases gracefully', () => {
  // Empty buffer
  const empty = decodeViewportBuffer(new ArrayBuffer(0));
  assert.strictEqual(empty.totalPackets, 0n);
  assert.strictEqual(empty.rows.length, 0);

  // Header truncated (< 16 bytes)
  const truncatedHeader = decodeViewportBuffer(new ArrayBuffer(12));
  assert.strictEqual(truncatedHeader.totalPackets, 0n);
  assert.strictEqual(truncatedHeader.rows.length, 0);

  // Valid header indicating 10 rows, but payload is truncated
  const partial = new ArrayBuffer(16 + 10);
  const view = new DataView(partial);
  view.setBigUint64(0, 50n, true);
  view.setUint32(8, 10, true);

  const res = decodeViewportBuffer(partial);
  assert.strictEqual(res.totalPackets, 50n);
  assert.strictEqual(res.rows.length, 0); // Guarded against out-of-bounds
});

test('decodeViewportBuffer benchmark (NFR-104 60 FPS compliance)', () => {
  // Construct 100 rows
  const rowCount = 100;
  const payload = Buffer.from('REALTIME_TELEMETRY_SAMPLE_DATA_FRAME_BYTE_PAYLOAD');
  const rowSize = 24 + payload.length;
  const totalSize = 16 + rowCount * rowSize;

  const buffer = new ArrayBuffer(totalSize);
  const view = new DataView(buffer);
  view.setBigUint64(0, 50000n, true);
  view.setUint32(8, rowCount, true);

  let offset = 16;
  for (let i = 0; i < rowCount; i++) {
    view.setBigUint64(offset, BigInt(1000 + i), true);
    view.setUint32(offset + 16, payload.length, true);
    view.setUint16(offset + 20, i, true);
    new Uint8Array(buffer, offset + 24).set(payload);
    offset += rowSize;
  }

  // Measure decode latency
  const start = performance.now();
  const iterations = 500;
  for (let i = 0; i < iterations; i++) {
    const batch = decodeViewportBuffer(buffer);
    assert.strictEqual(batch.rows.length, rowCount);
  }
  const totalMs = performance.now() - start;
  const avgUsPerRowBatch = (totalMs / iterations) * 1000;

  // 100 rows should decode in < 500 microseconds (< 0.5 ms), well within 16.6 ms frame budget
  assert.ok(
    avgUsPerRowBatch < 500,
    `Decoding 100 rows took ${avgUsPerRowBatch.toFixed(2)} us, exceeding 500 us target`
  );
});
