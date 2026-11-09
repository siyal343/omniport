import test from "node:test";
import assert from "node:assert/strict";
import {
  LOGIC_CHANNELS,
  renderLogicStrip,
} from "./logicStripRenderer.ts";
import { PinFlags, extractPinStates } from "../../types/serial.ts";
import { darkTheme } from "../../utils/theme.ts";

test("extractPinStates correctly decomposes bitmask into booleans", () => {
  const pins = PinFlags.RTS | PinFlags.DTR | PinFlags.BREAK;
  const snapshot = extractPinStates(pins);

  assert.equal(snapshot.rts, true);
  assert.equal(snapshot.cts, false);
  assert.equal(snapshot.dtr, true);
  assert.equal(snapshot.dsr, false);
  assert.equal(snapshot.dcd, false);
  assert.equal(snapshot.ri, false);
  assert.equal(snapshot.break, true);
});

test("LOGIC_CHANNELS contains standard RS-232 modem signals", () => {
  const names = LOGIC_CHANNELS.map((c) => c.name);
  assert.deepEqual(names, ["RTS", "CTS", "DTR", "DSR", "DCD", "RI"]);
});

test("renderLogicStrip executes waveform generation and cursor line without errors", () => {
  const drawOps = [];
  const mockCtx = {
    fillRect: (x, y, w, h) => drawOps.push(["fillRect", x, y, w, h]),
    strokeRect: (x, y, w, h) => drawOps.push(["strokeRect", x, y, w, h]),
    fillText: (text, x, y) => drawOps.push(["fillText", text, x, y]),
    beginPath: () => drawOps.push(["beginPath"]),
    moveTo: (x, y) => drawOps.push(["moveTo", x, y]),
    lineTo: (x, y) => drawOps.push(["lineTo", x, y]),
    stroke: () => drawOps.push(["stroke"]),
    setLineDash: () => {},
    font: "",
    textBaseline: "",
    fillStyle: "",
    strokeStyle: "",
    lineWidth: 1,
  };

  const rows = [
    {
      rowIndex: 10,
      timestampNs: 1_000_000_000n,
      endpointId: 1,
      flags: 0,
      pins: PinFlags.RTS,
      payload: new Uint8Array([]),
    },
    {
      rowIndex: 11,
      timestampNs: 1_010_000_000n,
      endpointId: 1,
      flags: 0,
      pins: PinFlags.RTS | PinFlags.CTS,
      payload: new Uint8Array([]),
    },
    {
      rowIndex: 12,
      timestampNs: 1_020_000_000n,
      endpointId: 1,
      flags: 0,
      pins: PinFlags.DTR,
      payload: new Uint8Array([]),
    },
  ];

  renderLogicStrip({
    ctx: mockCtx,
    width: 600,
    height: 120,
    rows,
    startRowIndex: 10,
    visibleRowCount: 3,
    selectedRowIndex: 11,
    theme: darkTheme,
  });

  assert.ok(drawOps.length > 20, "Should generate lines, rects, and text for channels");
});

test("benchmark: 100 rows rendered across 6 channels completes in < 2ms (NFR-104)", () => {
  const rows = [];
  for (let i = 0; i < 100; i++) {
    rows.push({
      rowIndex: i,
      timestampNs: BigInt(i * 10_000),
      endpointId: 1,
      flags: 0,
      pins: (i % 2 === 0 ? PinFlags.RTS : 0) | (i % 3 === 0 ? PinFlags.CTS : 0) | (i % 5 === 0 ? PinFlags.DTR : 0),
      payload: new Uint8Array(16),
    });
  }

  const mockCtx = {
    fillRect: () => {},
    strokeRect: () => {},
    fillText: () => {},
    beginPath: () => {},
    moveTo: () => {},
    lineTo: () => {},
    stroke: () => {},
    setLineDash: () => {},
    font: "",
    textBaseline: "",
    fillStyle: "",
    strokeStyle: "",
    lineWidth: 1,
  };

  const start = performance.now();
  for (let iter = 0; iter < 10; iter++) {
    renderLogicStrip({
      ctx: mockCtx,
      width: 1200,
      height: 180,
      rows,
      startRowIndex: 0,
      visibleRowCount: 100,
      selectedRowIndex: 42,
      theme: darkTheme,
    });
  }
  const elapsed = performance.now() - start;
  const avgMs = elapsed / 10;

  console.log(`[BENCHMARK] LogicStrip 100 rows x 6 channels average: ${avgMs.toFixed(3)} ms`);
  assert.ok(avgMs < 2.0, `Logic strip render ${avgMs.toFixed(3)} ms exceeds 2.0 ms target`);
});
