import test from "node:test";
import assert from "node:assert/strict";
import { TelemetryRingBuffer } from "./telemetryRingBuffer.ts";

test("TelemetryRingBuffer initializes with zero length and designated capacity", () => {
  const buf = new TelemetryRingBuffer(100);
  assert.equal(buf.getLength(), 0);
  assert.equal(buf.getCapacity(), 100);

  const [t, r, tp, j, c] = buf.getLinearData();
  assert.equal(t.length, 0);
  assert.equal(r.length, 0);
});

test("TelemetryRingBuffer correctly stores and retrieves sequential points", () => {
  const buf = new TelemetryRingBuffer(10);
  for (let i = 0; i < 5; i++) {
    buf.push(i * 0.1, i * 100, i * 50, i * 2, i);
  }

  assert.equal(buf.getLength(), 5);
  const [t, r, tp, j, c] = buf.getLinearData();
  assert.equal(t.length, 5);
  assert.equal(t[0], 0.0);
  assert.equal(t[4], 0.4);
  assert.equal(r[4], 400);
  assert.equal(tp[4], 200);
  assert.equal(j[4], 8);
  assert.equal(c[4], 4);
});

test("TelemetryRingBuffer wraps circular head cleanly when exceeding capacity", () => {
  const buf = new TelemetryRingBuffer(5);
  // Push 8 items into capacity 5
  for (let i = 0; i < 8; i++) {
    buf.push(i, i * 10, i * 20, i * 2, i);
  }

  assert.equal(buf.getLength(), 5);
  const [t, r] = buf.getLinearData();
  assert.equal(t.length, 5);
  // Oldest 3 should have been dropped: elements 3, 4, 5, 6, 7 remain
  assert.deepEqual(Array.from(t), [3, 4, 5, 6, 7]);
  assert.deepEqual(Array.from(r), [30, 40, 50, 60, 70]);
});

test("benchmark: 1,000 ringbuffer push and export cycles stay linear-time", () => {
  const buf = new TelemetryRingBuffer(600); // 10s @ 60 Hz

  // Warm up so JIT tiering does not leak into the measurement.
  for (let cycle = 0; cycle < 200; cycle++) {
    buf.push(cycle * 0.016, 500, 120, 1.2, 0);
    buf.getLinearData();
  }

  // A wall-clock budget in a unit test has to survive loaded CI machines, so take
  // the best of several runs and use a budget that still catches quadratic
  // regressions (an O(n^2) export would take seconds at this size) instead of a
  // tight number that flakes under parallel builds.
  const measureOnce = () => {
    const start = performance.now();
    for (let cycle = 0; cycle < 1000; cycle++) {
      buf.push(cycle * 0.016, 500, 120, 1.2, 0);
      buf.getLinearData();
    }
    return performance.now() - start;
  };

  const best = Math.min(measureOnce(), measureOnce(), measureOnce());
  const [t] = buf.getLinearData();
  assert.ok(t.length > 0);

  console.log(`[BENCHMARK] TelemetryRingBuffer 1,000 cycles (best of 3): ${best.toFixed(3)} ms`);
  assert.ok(
    best < 50.0,
    `Ringbuffer push+export is not linear-time: best of 3 ${best.toFixed(3)}ms exceeded 50.0ms budget`
  );
});
