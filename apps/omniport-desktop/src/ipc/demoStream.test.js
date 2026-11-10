import test from "node:test";
import assert from "node:assert/strict";
import {
  createDemoViewport,
  DEMO_TOTAL_PACKETS,
  DEMO_EPOCH_NS,
  DEMO_FRAME_INTERVAL_NS,
  DEMO_SEED_FRAME_COUNT,
  DEMO_SEED_FRAME_PREFIX,
} from "./demoStream.ts";
import { FrameFlags } from "../types/serial.ts";

test("demo stream mirrors the host seed frame naming for its first frames", () => {
  const batch = createDemoViewport({ startRow: 0, rowCount: 30 });
  assert.equal(batch.totalPackets, DEMO_TOTAL_PACKETS);
  assert.equal(batch.rows.length, 30);

  for (let i = 0; i < DEMO_SEED_FRAME_COUNT; i++) {
    const text = Buffer.from(batch.rows[i].payload).toString("ascii");
    assert.equal(text, `${DEMO_SEED_FRAME_PREFIX}${i.toString().padStart(3, "0")}`);
  }
  assert.ok(
    Buffer.from(batch.rows[DEMO_SEED_FRAME_COUNT].payload).toString("ascii").length > 0
  );
});

test("demo stream timestamps are monotonic and 5ms apart from one epoch", () => {
  const batch = createDemoViewport({ startRow: 10, rowCount: 5 });
  for (let i = 0; i < batch.rows.length; i++) {
    const expected = DEMO_EPOCH_NS + BigInt(10 + i) * DEMO_FRAME_INTERVAL_NS;
    assert.equal(batch.rows[i].timestampNs, expected);
  }
});

test("demo stream clamps requests at the end of the synthetic store", () => {
  const batch = createDemoViewport({ startRow: Number(DEMO_TOTAL_PACKETS) - 2, rowCount: 50 });
  assert.equal(batch.rows.length, 2);
  assert.equal(batch.totalPackets, DEMO_TOTAL_PACKETS);
});

test("demo stream is labelled data: every 29th frame carries the CRC error flag", () => {
  const batch = createDemoViewport({ startRow: 0, rowCount: 60 });
  const withCrc = batch.rows.filter((row) => (row.flags & FrameFlags.ERR_CRC) !== 0);
  assert.ok(withCrc.length >= 2);
  assert.ok(withCrc.every((row) => row.payload.length > 0));
});
