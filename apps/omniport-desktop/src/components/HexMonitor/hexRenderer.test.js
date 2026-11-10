import test from "node:test";
import assert from "node:assert/strict";
import {
  HEX_LUT,
  PIN_INDICATORS,
  computeColumnLayout,
  calculateViewportRange,
  formatTimeOffset,
  renderHexMonitor,
} from "./hexRenderer.ts";
import { darkTheme } from "../../utils/theme.ts";
import { FrameFlags, PinFlags } from "../../types/serial.ts";

/**
 * Minimal recording 2D-context double. It records every drawing call together
 * with the fillStyle active at the time so tests can assert on real renderer
 * output (text, colour, position) rather than simply "did not throw".
 */
function createRecordingContext() {
  const calls = [];
  const ctx = {
    fillStyle: "",
    strokeStyle: "",
    font: "",
    textBaseline: "",
    textAlign: "start",
    lineWidth: 1,
    globalAlpha: 1,
    fillRect: (x, y, w, h) => calls.push({ op: "fillRect", x, y, w, h, fillStyle: ctx.fillStyle }),
    fillText: (text, x, y) => calls.push({ op: "fillText", text, x, y, fillStyle: ctx.fillStyle }),
    strokeRect: (x, y, w, h) => calls.push({ op: "strokeRect", x, y, w, h }),
    beginPath: () => {},
    moveTo: () => {},
    lineTo: () => {},
    stroke: () => {},
  };
  return { ctx, calls };
}

function makeRow(overrides = {}) {
  return {
    rowIndex: 0,
    timestampNs: 1_000_000_000n,
    endpointId: 1,
    flags: FrameFlags.DIR_TX,
    pins: PinFlags.RTS,
    payload: new Uint8Array([0x48, 0x45, 0x58]),
    ...overrides,
  };
}

test("HEX_LUT contains correct 2-character uppercase hex strings", () => {
  assert.equal(HEX_LUT.length, 256);
  assert.equal(HEX_LUT[0], "00");
  assert.equal(HEX_LUT[10], "0A");
  assert.equal(HEX_LUT[16], "10");
  assert.equal(HEX_LUT[255], "FF");
});

test("computeColumnLayout keeps columns ordered and exposes the full content width", () => {
  const layout = computeColumnLayout(1000);
  assert.ok(layout.indexX < layout.timeX);
  assert.ok(layout.timeX < layout.endpointX);
  assert.ok(layout.endpointX < layout.pinsX);
  assert.ok(layout.pinsX < layout.hexX);
  assert.ok(layout.hexX < layout.asciiX);
  assert.ok(
    layout.totalWidth >= layout.asciiX + layout.asciiWidth,
    "totalWidth must contain the ASCII column; otherwise it is clipped"
  );
  assert.equal(layout.totalWidth, 1000, "wide windows should not force scrolling");
});

test("computeColumnLayout forces horizontal overflow at narrow window widths (800px)", () => {
  const layout = computeColumnLayout(800);
  assert.ok(layout.totalWidth > 800, "the HEX/ASCII columns need a scrollbar at 800px");
  assert.ok(layout.asciiX + layout.asciiWidth <= layout.totalWidth);
});

test("calculateViewportRange calculates accurate virtual rows with overscan", () => {
  const vp = calculateViewportRange(100, 200, 20, 1000, 5);
  assert.equal(vp.startRow, 5);
  assert.equal(vp.endRow, 15);
  assert.equal(vp.renderStartRow, 0);
  assert.equal(vp.renderEndRow, 20);
  assert.equal(vp.totalHeight, 20000);

  const vp2 = calculateViewportRange(2000, 200, 20, 1000, 5);
  assert.equal(vp2.startRow, 100);
  assert.equal(vp2.renderStartRow, 95);
  assert.equal(vp2.renderEndRow, 115);
});

test("calculateViewportRange is defensive for empty datasets and zero row height", () => {
  assert.deepEqual(calculateViewportRange(0, 400, 22, 0), {
    startRow: 0,
    endRow: 0,
    renderStartRow: 0,
    renderEndRow: 0,
    offsetY: 0,
    totalHeight: 0,
  });
  assert.equal(calculateViewportRange(100, 400, 0, 100).totalHeight, 0);
});

test("formatTimeOffset renders offsets relative to the supplied session epoch", () => {
  const base = 1_000_000_000n; // 1s
  assert.equal(formatTimeOffset(1_500_000_000n, base), "+00.500000s");
  assert.equal(formatTimeOffset(base, base), "+00.000000s");
  assert.equal(formatTimeOffset(base + 5_000_000n, base), "+00.005000s");
  // Large deltas must not be truncated by padStart(2)
  assert.equal(formatTimeOffset(base + 123_456_789_000n, base), "+123.456789s");
  // Clock skew / older frames clamp to zero instead of rendering negatives
  assert.equal(formatTimeOffset(base - 1_000n, base), "+00.000000s");
});

test("renderHexMonitor uses the real session epoch (no absurd +1700000000s offsets)", () => {
  const { ctx, calls } = createRecordingContext();
  const sessionEpoch = 1_700_000_000_000_000_000n;
  renderHexMonitor({
    ctx,
    width: 1200,
    height: 300,
    rowHeight: 22,
    headerHeight: 28,
    scrollTop: 0,
    totalRows: 1,
    rows: [makeRow({ timestampNs: sessionEpoch + 5_000_000n })],
    theme: darkTheme,
    baseTimeNs: sessionEpoch,
  });

  const timeTexts = calls.filter((c) => c.op === "fillText" && c.text.startsWith("+")).map((c) => c.text);
  assert.deepEqual(timeTexts, ["+00.005000s"]);
  assert.ok(
    calls.every((c) => c.text === undefined || !c.text.includes("1700000000")),
    "renderer must never paint raw Unix-epoch seconds as an offset"
  );
});

test("renderHexMonitor renders all six modem pin channels with unambiguous labels", () => {
  const allPins =
    PinFlags.RTS | PinFlags.CTS | PinFlags.DTR | PinFlags.DSR | PinFlags.DCD | PinFlags.RI;
  const { ctx, calls } = createRecordingContext();
  renderHexMonitor({
    ctx,
    width: 1200,
    height: 300,
    rowHeight: 22,
    headerHeight: 28,
    scrollTop: 0,
    totalRows: 1,
    rows: [makeRow({ pins: allPins })],
    theme: darkTheme,
    baseTimeNs: 1_000_000_000n,
  });

  assert.deepEqual(
    PIN_INDICATORS.map((p) => p.name),
    ["RTS", "CTS", "DTR", "DSR", "DCD", "RI"]
  );
  const pinTexts = calls.filter((c) => c.op === "fillText" && PIN_INDICATORS.some((p) => p.name === c.text));
  assert.equal(pinTexts.length, 6, "every pin channel must be drawn, including DSR");
  // CTS and DCD must not collapse to the same glyph, and each high pin uses its channel colour.
  const drawnNames = pinTexts.map((c) => c.text);
  assert.equal(new Set(drawnNames).size, 6);
  for (const indicator of PIN_INDICATORS) {
    const rendered = pinTexts.find((c) => c.text === indicator.name);
    assert.equal(rendered.fillStyle, indicator.color, `${indicator.name} HIGH must use its channel colour`);
  }
});

test("renderHexMonitor dims inactive pins but still reserves all six indicators", () => {
  const { ctx, calls } = createRecordingContext();
  renderHexMonitor({
    ctx,
    width: 1200,
    height: 300,
    rowHeight: 22,
    headerHeight: 28,
    scrollTop: 0,
    totalRows: 1,
    rows: [makeRow({ pins: 0 })],
    theme: darkTheme,
    baseTimeNs: 1_000_000_000n,
  });
  const pinTexts = calls.filter((c) => c.op === "fillText" && PIN_INDICATORS.some((p) => p.name === c.text));
  assert.equal(pinTexts.length, 6);
  for (const rendered of pinTexts) {
    assert.equal(rendered.fillStyle, darkTheme.asciiUnprintable);
  }
});

test("renderHexMonitor paints the empty state when the capture session has no frames", () => {
  const { ctx, calls } = createRecordingContext();
  renderHexMonitor({
    ctx,
    width: 1200,
    height: 300,
    rowHeight: 22,
    headerHeight: 28,
    scrollTop: 0,
    totalRows: 0,
    rows: [],
    theme: darkTheme,
    baseTimeNs: 0n,
  });
  assert.ok(
    calls.some((c) => c.op === "fillText" && c.text === "WAITING FOR CAPTURE DATA"),
    "empty state must tell the operator the monitor is waiting for data"
  );
});

test("renderHexMonitor draws loading placeholders for not-yet-fetched rows", () => {
  const { ctx, calls } = createRecordingContext();
  renderHexMonitor({
    ctx,
    width: 1200,
    height: 300,
    rowHeight: 22,
    headerHeight: 28,
    scrollTop: 0,
    totalRows: 30,
    rows: [],
    theme: darkTheme,
    baseTimeNs: 1_000_000_000n,
  });
  const loading = calls.filter((c) => c.op === "fillText" && c.text.includes("[loading...]"));
  assert.ok(loading.length > 0);
});

test("renderHexMonitor with a wide content canvas keeps every column inside the painted width", () => {
  const { ctx, calls } = createRecordingContext();
  const viewportWidth = 800;
  const layout = computeColumnLayout(viewportWidth);
  renderHexMonitor({
    ctx,
    width: layout.totalWidth,
    viewportWidth,
    height: 300,
    rowHeight: 22,
    headerHeight: 28,
    scrollTop: 0,
    totalRows: 1,
    rows: [makeRow({ payload: new Uint8Array([0x41, 0x42, 0x43]) })],
    theme: darkTheme,
    baseTimeNs: 1_000_000_000n,
  });

  const asciiCall = calls.find((c) => c.op === "fillText" && c.text === "A");
  assert.ok(asciiCall, "ASCII column must be drawn");
  assert.equal(asciiCall.x, layout.asciiX);
  assert.ok(asciiCall.x + 40 <= layout.totalWidth, "ASCII column must fit inside the scrollable canvas");
  // Header cells are painted over the full content width, not the viewport.
  const headerFill = calls.find((c) => c.op === "fillRect" && c.y === 0 && c.h === 28);
  assert.ok(headerFill);
  assert.ok(headerFill.w >= layout.asciiX + layout.asciiWidth);
});

test("benchmark: 100 rows render within the 16.6ms frame budget", () => {
  const { ctx } = createRecordingContext();
  const rows = Array.from({ length: 100 }, (_, i) =>
    makeRow({
      rowIndex: i,
      timestampNs: 1_000_000_000n + BigInt(i) * 1_000_000n,
      pins: (i % 2 === 0 ? PinFlags.RTS : 0) | PinFlags.CTS,
      payload: new Uint8Array(16).fill(i),
    })
  );
  const start = performance.now();
  const iterations = 20;
  for (let i = 0; i < iterations; i++) {
    renderHexMonitor({
      ctx,
      width: 1440,
      height: 900,
      rowHeight: 22,
      headerHeight: 28,
      scrollTop: 0,
      totalRows: 2000,
      rows,
      selectedRowIndex: 3,
      theme: darkTheme,
      baseTimeNs: 1_000_000_000n,
    });
  }
  const avgMs = (performance.now() - start) / iterations;
  console.log(`[BENCHMARK] HexMonitor 100 rows visible average: ${avgMs.toFixed(3)} ms`);
  assert.ok(avgMs < 16.6, `render averaged ${avgMs.toFixed(3)} ms, exceeding the 16.6 ms frame budget`);
});

test("renderHexMonitor paints exactly the session-supplied render window", () => {
  const { ctx, calls } = createRecordingContext();
  renderHexMonitor({
    ctx,
    width: 1200,
    height: 300,
    rowHeight: 22,
    headerHeight: 28,
    scrollTop: 0,
    totalRows: 100,
    rows: [
      makeRow({ rowIndex: 3, timestampNs: 1_000_000_000n }),
      makeRow({ rowIndex: 4, timestampNs: 1_022_000_000n }),
    ],
    theme: darkTheme,
    baseTimeNs: 1_000_000_000n,
    renderWindow: { start: 3, count: 2 },
  });
  const frameTexts = calls
    .filter((c) => c.op === "fillText" && /^#\d{6}$/.test(c.text))
    .map((c) => c.text);
  assert.deepEqual(frameTexts, ["#000003", "#000004"]);
});
test("renderHexMonitor marks payloads longer than 16 bytes as truncated", () => {
  const { ctx, calls } = createRecordingContext();
  renderHexMonitor({
    ctx,
    width: 1200,
    height: 300,
    rowHeight: 22,
    headerHeight: 28,
    scrollTop: 0,
    totalRows: 1,
    rows: [makeRow({ payload: new Uint8Array(24).fill(0x41) })],
    theme: darkTheme,
    baseTimeNs: 1_000_000_000n,
  });
  const texts = calls.filter((c) => c.op === "fillText").map((c) => c.text);
  assert.ok(texts.includes("…+8"), "hex gutter must show …+N for hidden bytes");
  assert.ok(texts.includes("…"), "ascii gutter must show a truncation ellipsis");
});

test("renderHexMonitor does not mark payloads that fit the 16-byte column", () => {
  const { ctx, calls } = createRecordingContext();
  renderHexMonitor({
    ctx,
    width: 1200,
    height: 300,
    rowHeight: 22,
    headerHeight: 28,
    scrollTop: 0,
    totalRows: 1,
    rows: [makeRow({ payload: new Uint8Array(16).fill(0x41) })],
    theme: darkTheme,
    baseTimeNs: 1_000_000_000n,
  });
  const texts = calls.filter((c) => c.op === "fillText").map((c) => c.text);
  assert.ok(!texts.some((text) => typeof text === "string" && text.startsWith("…+")));
});