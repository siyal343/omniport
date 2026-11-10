import test from "node:test";
import assert from "node:assert/strict";
import { summarizeViewportRows, EMPTY_TELEMETRY_SAMPLE } from "./telemetryFromRows.ts";
import { FrameFlags } from "../../types/serial.ts";

function makeRow(overrides = {}) {
  return {
    rowIndex: 0,
    timestampNs: 0n,
    endpointId: 1,
    flags: 0,
    pins: 0,
    payload: new Uint8Array(2),
    ...overrides,
  };
}

test("summarizeViewportRows returns all-zero metrics for an empty viewport", () => {
  assert.deepEqual(summarizeViewportRows([]), EMPTY_TELEMETRY_SAMPLE);
});

test("summarizeViewportRows derives rate, throughput and CRC errors from real frames", () => {
  // 3 frames spanning 3ms: 2 inter-frame intervals => 2/0.003 = 666.67 pkt/s
  const rows = [
    makeRow({ timestampNs: 0n, payload: new Uint8Array(2) }),
    makeRow({ timestampNs: 1_000_000n, payload: new Uint8Array(2), flags: FrameFlags.ERR_CRC }),
    makeRow({ timestampNs: 3_000_000n, payload: new Uint8Array(2) }),
  ];

  const sample = summarizeViewportRows(rows);
  const expectedRate = 2 / 0.003;
  const expectedThroughput = 6 / 1024 / 0.003;

  assert.ok(Math.abs(sample.packetRate - expectedRate) < 1e-6, `rate ${sample.packetRate}`);
  assert.ok(
    Math.abs(sample.throughputKbps - expectedThroughput) < 1e-6,
    `throughput ${sample.throughputKbps}`
  );
  assert.equal(sample.crcErrors, 1);
  // Inter-frame gaps are 1ms and 2ms => population std dev 0.5ms.
  assert.ok(Math.abs(sample.jitterMs - 0.5) < 1e-9, `jitter ${sample.jitterMs}`);
});

test("summarizeViewportRows is safe for a single frame or zero time span", () => {
  const single = summarizeViewportRows([makeRow({ timestampNs: 42n })]);
  assert.equal(single.packetRate, 0);
  assert.equal(single.throughputKbps, 0);
  assert.equal(single.jitterMs, 0);
  assert.equal(single.crcErrors, 0);

  const sameTime = summarizeViewportRows([
    makeRow({ timestampNs: 7n }),
    makeRow({ timestampNs: 7n }),
  ]);
  assert.equal(sameTime.packetRate, 0, "zero span must not divide by zero");
  assert.equal(sameTime.throughputKbps, 0);
  assert.equal(sameTime.jitterMs, 0);
});

test("summarizeViewportRows counts CRC flags only, not other frame flags", () => {
  const rows = [
    makeRow({ flags: FrameFlags.DIR_TX | FrameFlags.FAULT_INJECT }),
    makeRow({ flags: FrameFlags.ERR_PARITY }),
    makeRow({ flags: FrameFlags.ERR_CRC }),
  ];
  assert.equal(summarizeViewportRows(rows).crcErrors, 1);
});
