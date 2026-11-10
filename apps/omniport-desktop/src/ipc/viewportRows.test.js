import test from "node:test";
import assert from "node:assert/strict";
import { mapBatchToRows, batchEpochNs, nextSessionEpochNs } from "./viewportRows.ts";
import { ViewportRequestGate } from "./viewportRequestGate.ts";
import { FrameFlags, PinFlags } from "../types/serial.ts";

function makeBatch(rows, totalPackets = BigInt(rows.length)) {
  return { totalPackets, rows };
}

function makeWireRow(overrides = {}) {
  return {
    timestampNs: 1_000n,
    endpointId: 1,
    flags: FrameFlags.DIR_TX,
    pins: PinFlags.RTS,
    payload: new Uint8Array([1, 2, 3]),
    ...overrides,
  };
}

test("mapBatchToRows maps decoded rows onto absolute row indices", () => {
  const batch = makeBatch([
    makeWireRow({ timestampNs: 10n }),
    makeWireRow({ timestampNs: 20n }),
    makeWireRow({ timestampNs: 30n }),
  ]);

  const rows = mapBatchToRows(batch, 2400);
  assert.deepEqual(rows.map((r) => r.rowIndex), [2400, 2401, 2402]);
  assert.deepEqual(rows.map((r) => r.timestampNs), [10n, 20n, 30n]);
  assert.equal(rows[0].endpointId, 1);
  assert.equal(rows[0].flags, FrameFlags.DIR_TX);
  assert.equal(rows[0].pins, PinFlags.RTS);
  assert.deepEqual(Array.from(rows[0].payload), [1, 2, 3]);
});

test("mapBatchToRows never fabricates rows for a short (clamped) response", () => {
  const batch = makeBatch([makeWireRow()], 2n);
  const rows = mapBatchToRows(batch, 1);
  assert.equal(rows.length, 1, "clamped batches must not invent placeholder frames");
  assert.equal(rows[0].rowIndex, 1);
});

test("mapBatchToRows handles an empty batch", () => {
  assert.deepEqual(mapBatchToRows(makeBatch([]), 42), []);
});

test("batchEpochNs returns the timestamp of the first returned row", () => {
  assert.equal(batchEpochNs(makeBatch([makeWireRow({ timestampNs: 123n })])), 123n);
  assert.equal(batchEpochNs(makeBatch([])), null);
});

test("nextSessionEpochNs keeps the earliest timestamp observed", () => {
  assert.equal(nextSessionEpochNs(null, 500n), 500n);
  assert.equal(nextSessionEpochNs(500n, null), 500n);
  assert.equal(nextSessionEpochNs(500n, 400n), 400n, "scrolling back moves the epoch back");
  assert.equal(nextSessionEpochNs(500n, 900n), 500n, "scrolling forward keeps the session start");
});

test("ViewportRequestGate lets only the newest request win", () => {
  const gate = new ViewportRequestGate();
  const first = gate.begin();
  const second = gate.begin();

  assert.equal(gate.isCurrent(first), false, "the superseded request must be dropped");
  assert.equal(gate.isCurrent(second), true);

  gate.invalidate();
  assert.equal(gate.isCurrent(second), false, "invalidate drops every in-flight request");
});

test("race simulation: a stale slow fetch cannot overwrite newer rows", async () => {
  const gate = new ViewportRequestGate();
  let painted = null;

  const responseFor = (startRow, delayMs) =>
    new Promise((resolve) =>
      setTimeout(() => resolve({ startRow, delayMs }), delayMs)
    );

  async function fetchAndPaint(startRow, delayMs) {
    const ticket = gate.begin();
    const response = await responseFor(startRow, delayMs);
    if (!gate.isCurrent(ticket)) return;
    painted = response.startRow;
  }

  // Wheel from row 0 to row 2400: the newer request resolves first, the older
  // request trickles in afterwards and must not clobber the monitor.
  const slowOld = fetchAndPaint(0, 40);
  const fastNew = fetchAndPaint(2400, 5);
  await Promise.all([slowOld, fastNew]);

  assert.equal(painted, 2400);
});

test("race simulation: superseded failures do not clear the newest batch", async () => {
  const gate = new ViewportRequestGate();
  let lastError = null;
  let painted = null;

  async function fetchAndPaint(startRow, fail, delayMs) {
    const ticket = gate.begin();
    await new Promise((resolve) => setTimeout(resolve, delayMs));
    if (!gate.isCurrent(ticket)) return;
    if (fail) {
      lastError = startRow;
      return;
    }
    painted = startRow;
  }

  const slowFail = fetchAndPaint(0, true, 30);
  const fastOk = fetchAndPaint(2400, false, 5);
  await Promise.all([slowFail, fastOk]);

  assert.equal(painted, 2400);
  assert.equal(lastError, null, "an outdated failed request must not surface an error state");
});
