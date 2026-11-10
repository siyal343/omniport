import test from "node:test";
import assert from "node:assert/strict";
import { decodeViewportBuffer } from "../ipc/viewportDecoder.ts";
import { renderHexMonitor } from "../components/HexMonitor/hexRenderer.ts";
import { TelemetryRingBuffer } from "../components/Telemetry/telemetryRingBuffer.ts";
import { darkTheme } from "../utils/theme.ts";

const ROWS_PER_FRAME = 200;
const ROW_HEIGHT = 22;
const HEADER_HEIGHT = 28;
const CANVAS_WIDTH = 1280;
const CANVAS_HEIGHT = 720;
const FRAME_BUDGET_MS = 16.6; // NFR-104: 60 FPS
const MEASURED_FRAMES = 240;

/**
 * Builds one ICD §2 viewport batch with `rows` rows (24-byte row header plus a
 * 16-byte payload each), i.e. the exact ArrayBuffer the Tauri command returns.
 */
function viewportWireBuffer(rows) {
  const rowSize = 24 + 16;
  const buffer = new ArrayBuffer(16 + rows * rowSize);
  const view = new DataView(buffer);
  const bytes = new Uint8Array(buffer);
  view.setBigUint64(0, BigInt(rows), true);
  view.setUint32(8, rows, true);
  view.setUint32(12, 0, true);
  let offset = 16;
  for (let i = 0; i < rows; i++) {
    view.setBigUint64(offset, BigInt(1_500_000_000 + i * 1_000_000), true);
    view.setBigUint64(offset + 8, BigInt(i * 16), true);
    view.setUint32(offset + 16, 16, true);
    view.setUint16(offset + 20, (i % 4) + 1, true);
    view.setUint8(offset + 22, 0b0000_0001);
    view.setUint8(offset + 23, 0b0000_0101);
    for (let b = 0; b < 16; b++) bytes[offset + 24 + b] = (i + b) & 0xff;
    offset += rowSize;
  }
  return buffer;
}

/** No-op 2D context mirroring the calls HexMonitor makes (canvas rasterization excluded). */
function recordingContext() {
  const noop = () => {};
  return {
    fillStyle: "",
    strokeStyle: "",
    font: "",
    textBaseline: "",
    textAlign: "start",
    lineWidth: 1,
    globalAlpha: 1,
    beginPath: noop,
    fillRect: noop,
    fillText: noop,
    lineTo: noop,
    moveTo: noop,
    stroke: noop,
  };
}

function toViewportRows(batch) {
  return batch.rows.map((row, index) => ({
    rowIndex: index,
    timestampNs: row.timestampNs,
    endpointId: row.endpointId,
    flags: row.flags,
    pins: row.pins,
    payload: row.payload,
  }));
}

/**
 * NFR-104 frame budget.
 *
 * Measures the per-frame CPU work the desktop performs at 60 FPS: ICD §2 wire
 * decode, Hex Canvas draw-call emission for a 200-row viewport, and telemetry
 * ingest into the ring buffer. Actual GPU/compositor rasterization is hardware
 * dependent and is covered by the manual WebView2 pass, not by headless CI.
 */
test("NFR-104: desktop 60 FPS frame budget for viewport decode + hex render + telemetry ingest", () => {
  const wire = viewportWireBuffer(ROWS_PER_FRAME);
  const ctx = recordingContext();
  const telemetry = new TelemetryRingBuffer(4096);

  const oneFrame = () => {
    const started = performance.now();

    const batch = decodeViewportBuffer(wire);
    assert.equal(batch.rows.length, ROWS_PER_FRAME);
    const rows = toViewportRows(batch);

    renderHexMonitor({
      ctx,
      width: CANVAS_WIDTH,
      height: CANVAS_HEIGHT,
      rowHeight: ROW_HEIGHT,
      headerHeight: HEADER_HEIGHT,
      scrollTop: 0,
      totalRows: ROWS_PER_FRAME,
      rows,
      baseTimeNs: rows[0].timestampNs,
      theme: darkTheme,
    });

    for (const row of rows) {
      telemetry.push(
        Number(row.timestampNs) / 1_000_000_000,
        row.payload.length,
        12.5,
        0.2,
        row.flags
      );
    }
    const [time, rate] = telemetry.getLinearData();
    assert.equal(time.length, rate.length);

    return performance.now() - started;
  };

  // Warm-up: JIT, BigInt paths, ring-buffer growth.
  for (let i = 0; i < 20; i++) oneFrame();

  const samples = [];
  for (let i = 0; i < MEASURED_FRAMES; i++) samples.push(oneFrame());
  samples.sort((a, b) => a - b);
  const average = samples.reduce((total, value) => total + value, 0) / samples.length;
  const p95 = samples[Math.floor(0.95 * (samples.length - 1))];
  const worst = samples[samples.length - 1];

  console.log(
    `NFR-104: frame budget over ${MEASURED_FRAMES} frames — avg=${average.toFixed(3)} ms, ` +
      `p95=${p95.toFixed(3)} ms, max=${worst.toFixed(3)} ms (target <= ${FRAME_BUDGET_MS} ms)`
  );

  assert.ok(
    p95 <= FRAME_BUDGET_MS,
    `NFR-104 violated: p95 frame budget ${p95.toFixed(3)} ms exceeds ${FRAME_BUDGET_MS} ms`
  );
  assert.ok(
    average <= FRAME_BUDGET_MS,
    `NFR-104 violated: average frame budget ${average.toFixed(3)} ms exceeds ${FRAME_BUDGET_MS} ms`
  );
});
