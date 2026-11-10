import test from "node:test";
import assert from "node:assert/strict";
import {
  clampScrollTop,
  computeContentHeight,
  computeScrollMax,
  computeWindows,
  renderWindowKey,
  windowEndRow,
} from "./viewportWindow.ts";

test("computeContentHeight and computeScrollMax handle normal and degenerate inputs", () => {
  assert.equal(computeContentHeight(1000, 20), 20000);
  assert.equal(computeContentHeight(0, 20), 0);
  assert.equal(computeContentHeight(1000, 0), 0);

  assert.equal(computeScrollMax(1000, 20, 200), 19800);
  assert.equal(computeScrollMax(5, 20, 200), 0, "content shorter than viewport cannot scroll");
  assert.equal(computeScrollMax(0, 20, 200), 0);
  assert.equal(computeScrollMax(1000, 0, 200), 0);
  assert.equal(computeScrollMax(1000, 20, 0), 20000, "zero viewport height scrolls the whole list");
});

test("clampScrollTop clamps into [0, maxScroll] and rejects non-finite input", () => {
  assert.equal(clampScrollTop(-50, 200, 20, 1000), 0);
  assert.equal(clampScrollTop(0, 200, 20, 1000), 0);
  assert.equal(clampScrollTop(999999, 200, 20, 1000), 19800);
  assert.equal(clampScrollTop(1234, 200, 20, 1000), 1234);
  assert.equal(clampScrollTop(Number.NaN, 200, 20, 1000), 0);
  assert.equal(clampScrollTop(50, 200, 20, 0), 0);
});

test("computeWindows returns visible + overscanned render windows", () => {
  const calc = computeWindows(100, 200, 20, 1000, 5);
  assert.deepEqual(calc.visible, { start: 5, count: 10 });
  assert.deepEqual(calc.render, { start: 0, count: 20 });
  assert.equal(calc.maxScrollPx, 19800);
  assert.equal(calc.offsetInsideRowPx, 0);
});

test("computeWindows keeps the pixel offset inside a partially scrolled row", () => {
  const calc = computeWindows(110, 200, 20, 1000, 5);
  assert.equal(calc.visible.start, 5);
  assert.equal(calc.offsetInsideRowPx, 10);
  assert.deepEqual(calc.render, { start: 0, count: 20 });
});

test("computeWindows clamps a scroll offset past the end to the last page", () => {
  const calc = computeWindows(999999, 200, 20, 1000, 5);
  assert.equal(calc.maxScrollPx, 19800);
  assert.equal(calc.visible.start, 990);
  assert.equal(calc.visible.count, 10);
  assert.deepEqual(calc.render, { start: 985, count: 15 }, "render window clamps at totalRows");
});

test("computeWindows clamps visible/render counts at the end of the list", () => {
  const calc = computeWindows(0, 200, 20, 7, 5);
  assert.deepEqual(calc.visible, { start: 0, count: 7 });
  assert.deepEqual(calc.render, { start: 0, count: 7 });
  assert.equal(calc.maxScrollPx, 0, "a short list cannot scroll");

  // 7 rows in a 5-row viewport: scroll is clamped to 40px, row 2 is first.
  const lastPage = computeWindows(999, 100, 20, 7, 5);
  assert.equal(lastPage.maxScrollPx, 40);
  assert.deepEqual(lastPage.visible, { start: 2, count: 5 });
  assert.deepEqual(lastPage.render, { start: 0, count: 7 });
});

test("computeWindows yields zeros for empty stores or invalid row height", () => {
  const zeroRows = computeWindows(50, 200, 20, 0, 5);
  assert.deepEqual(zeroRows.visible, { start: 0, count: 0 });
  assert.deepEqual(zeroRows.render, { start: 0, count: 0 });
  assert.equal(zeroRows.maxScrollPx, 0);
  assert.equal(zeroRows.offsetInsideRowPx, 0);

  const zeroRowHeight = computeWindows(50, 200, 0, 100, 5);
  assert.deepEqual(zeroRowHeight.visible, { start: 0, count: 0 });
  assert.deepEqual(zeroRowHeight.render, { start: 0, count: 0 });
});

test("computeWindows handles a zero-height viewport (hidden/unmounted canvas)", () => {
  const calc = computeWindows(0, 0, 20, 100, 5);
  assert.deepEqual(calc.visible, { start: 0, count: 0 });
  assert.deepEqual(calc.render, { start: 0, count: 0 });
  assert.equal(calc.maxScrollPx, 2000);
  assert.equal(calc.offsetInsideRowPx, 0);
});

test("computeWindows without overscan renders exactly the visible rows", () => {
  const calc = computeWindows(100, 200, 20, 1000, 0);
  assert.deepEqual(calc.visible, { start: 5, count: 10 });
  assert.deepEqual(calc.render, { start: 5, count: 10 });
});

test("windowEndRow / renderWindowKey describe a window", () => {
  assert.equal(windowEndRow({ start: 0, count: 10 }), 9);
  assert.equal(windowEndRow({ start: 5, count: 0 }), -1);
  assert.equal(renderWindowKey({ start: 990, count: 10 }), "990:10");
});

test("computeWindows is compatible with the legacy calculateViewportRange contract", () => {
  // These are the exact expectations the renderer's legacy helper asserted.
  const calc = computeWindows(100, 200, 20, 1000, 5);
  assert.equal(calc.visible.start, 5);
  assert.equal(calc.visible.start + calc.visible.count, 15);
  assert.equal(calc.render.start, 0);
  assert.equal(calc.render.start + calc.render.count, 20);

  const large = computeWindows(2000, 200, 20, 1000, 5);
  assert.equal(large.visible.start, 100);
  assert.equal(large.render.start, 95);
  assert.equal(large.render.start + large.render.count, 115);
});
