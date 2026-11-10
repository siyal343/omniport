/**
 * Golden contract test for the ICD §2 visible-viewport wire format.
 *
 * The oracle is `fixtures/viewport_v1.json` (frozen), generated independently
 * from the ICD text — never hand-edit the fixture or relax it here. If a case
 * fails, the decoder nibble offsets or the fixture must be reported, not
 * silently adjusted.
 */

import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { decodeViewportBuffer } from "./viewportDecoder.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
const FIXTURE_PATH = resolve(HERE, "../../../../fixtures/viewport_v1.json");
const fixture = JSON.parse(readFileSync(FIXTURE_PATH, "utf8"));

function wireBytes(wireHex) {
  return new Uint8Array(Buffer.from(wireHex, "hex"));
}

test("fixture declares the ICD §2 batch and row header sizes", () => {
  assert.equal(fixture.schema, "omniport.viewport.v1");
  assert.equal(fixture.frozen, true);
  assert.equal(fixture.batch_header_bytes, 16);
  assert.equal(fixture.row_header_bytes, 24);
  assert.equal(fixture.endianness, "little");
  assert.ok(Array.isArray(fixture.cases));
  assert.equal(fixture.cases.length, 6);
  assert.equal(new Set(fixture.cases.map((c) => c.id)).size, fixture.cases.length);
});

for (const testCase of fixture.cases) {
  test(`viewport contract: ${testCase.id} — ${testCase.comment}`, () => {
    const wire = wireBytes(testCase.wire_hex);

    // The declared header sizes must exactly account for the wire payload.
    const expectedLen =
      fixture.batch_header_bytes +
      testCase.rows.reduce(
        (sum, row) => sum + fixture.row_header_bytes + row.payload_hex.length / 2,
        0
      );
    assert.equal(wire.byteLength, testCase.wire_len, "wire_hex length must match wire_len");
    assert.equal(expectedLen, testCase.wire_len, "16B batch header + 24B/row + payload must match");

    // Decode both from a standalone ArrayBuffer and from a Uint8Array view.
    const fromArrayBuffer = decodeViewportBuffer(
      wire.buffer.slice(wire.byteOffset, wire.byteOffset + wire.byteLength)
    );
    const fromView = decodeViewportBuffer(wire);

    assert.equal(fromArrayBuffer.totalPackets, BigInt(testCase.total_packets));
    assert.equal(fromView.totalPackets, BigInt(testCase.total_packets));
    assert.equal(fromArrayBuffer.rows.length, testCase.rows.length);
    assert.equal(fromView.rows.length, testCase.rows.length);

    for (let i = 0; i < testCase.rows.length; i++) {
      const expected = testCase.rows[i];
      const actual = fromArrayBuffer.rows[i];
      const label = `${testCase.id} row ${i}`;

      assert.equal(actual.timestampNs, BigInt(expected.timestamp_ns), `${label} timestamp`);
      assert.equal(actual.endpointId, expected.endpoint_id, `${label} endpoint_id`);
      assert.equal(actual.flags, expected.flags, `${label} flags`);
      assert.equal(actual.pins, expected.pins, `${label} pins`);

      const expectedPayload = Buffer.from(expected.payload_hex, "hex");
      assert.ok(
        Buffer.from(actual.payload).equals(expectedPayload),
        `${label} payload bytes: expected ${expected.payload_hex}`
      );
      assert.equal(
        actual.payload.length,
        expected.payload_hex.length / 2,
        `${label} payload length`
      );
    }
  });
}

test("absurd start_row (u64::MAX) case returns a header-only batch without throwing", () => {
  const absurd = fixture.cases.find((c) => c.id === "absurd_start_row");
  assert.ok(absurd, "fixture must contain the absurd_start_row case");
  assert.equal(absurd.start_row, "18446744073709551615");

  const batch = decodeViewportBuffer(wireBytes(absurd.wire_hex));
  assert.equal(batch.totalPackets, 1n);
  assert.deepEqual(batch.rows, []);
});
