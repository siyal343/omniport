import test from "node:test";
import assert from "node:assert/strict";
import {
  HEX_LUT,
  computeColumnLayout,
  calculateViewportRange,
  formatTimeOffset,
  renderHexMonitor,
} from "./hexRenderer.ts";
import { darkTheme } from "../../utils/theme.ts";
import { FrameFlags, PinFlags } from "../../types/serial.ts";

test("HEX_LUT contains correct 2-character uppercase hex strings", () => {
  assert.equal(HEX_LUT.length, 256);
  assert.equal(HEX_LUT[0], "00");
  assert.equal(HEX_LUT[10], "0A");
  assert.equal(HEX_LUT[16], "10");
  assert.equal(HEX_LUT[255], "FF");
});

test("computeColumnLayout calculates correct column offsets", () => {
  const layout = computeColumnLayout(1000);
  assert.ok(layout.indexX < layout.timeX);
  assert.ok(layout.timeX < layout.endpointX);
  assert.ok(layout.endpointX < layout.pinsX);
  assert.ok(layout.pinsX < layout.hexX);
  assert.ok(layout.hexX < layout.asciiX);
  assert.ok(layout.totalWidth >= 1000);
});

test("calculateViewportRange calculates accurate virtual rows with overscan", () => {
  const rowHeight = 20;
  const viewportHeight = 200; // 10 rows visible
  const totalRows = 1000;
  const scrollTop = 100; // startRow = 5

  const vp = calculateViewportRange(scrollTop, viewportHeight, rowHeight, totalRows, 5);

  assert.equal(vp.startRow, 5);
  assert.equal(vp.endRow, 15);
  assert.equal(vp.renderStartRow, 0); // clamped at 0
  assert.equal(vp.renderEndRow, 20); // 15 + 5
  assert.equal(vp.totalHeight, 20000);

  // Large scroll
  const vp2 = calculateViewportRange(2000, viewportHeight, rowHeight, totalRows, 5);
  assert.equal(vp2.startRow, 100);
  assert.equal(vp2.renderStartRow, 95);
  assert.equal(vp2.renderEndRow, 115);
});

test("formatTimeOffset formats delta nanoseconds properly", () => {
  const base = 1_000_000_000n; // 1s
  const t1 = 1_500_000_000n;  // 1.5s (+0.5s)
  const formatted = formatTimeOffset(t1, base);
  assert.equal(formatted, "+00.500000s");
});

test("renderHexMonitor executes canvas drawing without throwing", () => {
  const drawCalls = [];
  const mockCtx = {
    fillRect: (x, y, w, h) => drawCalls.push(["fillRect", x, y, w, h]),
    fillText: (text, x, y) => drawCalls.push(["fillText", text, x, y]),
    strokeRect: (x, y, w, h) => drawCalls.push(["strokeRect", x, y, w, h]),
    beginPath: () => {},
    moveTo: () => {},
    lineTo: () => {},
    stroke: () => {},
    font: "",
    textBaseline: "",
    fillStyle: "",
    strokeStyle: "",
    lineWidth: 1,
  };

  const rows = [
    {
      rowIndex: 0,
      timestampNs: 1_000_000_000n,
      endpointId: 1,
      flags: FrameFlags.DIR_TX,
      pins: PinFlags.RTS | PinFlags.DTR,
      payload: new Uint8Array([0x48, 0x45, 0x58, 0x20, 0x54, 0x45, 0x53, 0x54]),
    },
    {
      rowIndex: 1,
      timestampNs: 1_050_000_000n,
      endpointId: 2,
      flags: FrameFlags.ERR_CRC,
      pins: PinFlags.CTS | PinFlags.DCD,
      payload: new Uint8Array([0xDE, 0xAD, 0xBE, 0xEF]),
    },
  ];

  renderHexMonitor({
    ctx: mockCtx,
    width: 800,
    height: 300,
    rowHeight: 22,
    headerHeight: 28,
    scrollTop: 0,
    totalRows: 2,
    rows,
    selectedRowIndex: 0,
    hoveredRowIndex: 1,
    theme: darkTheme,
  });

  assert.ok(drawCalls.length > 10, "renderHexMonitor should perform text and rect draws");
});
