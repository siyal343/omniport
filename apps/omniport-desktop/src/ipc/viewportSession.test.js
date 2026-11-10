import test from "node:test";
import assert from "node:assert/strict";
import { ViewportSession } from "./viewportSession.ts";

const ROW = 20;
const OVERSCAN = 5;
const VIEWPORT = 200; // 10 visible rows
const EPOCH = 1_700_000_000_000_000_000n;

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function makeBatch(startRow, count, totalPackets) {
  const rows = Array.from({ length: count }, (_, i) => {
    const index = startRow + i;
    return {
      timestampNs: EPOCH + BigInt(index) * 1_000_000n,
      endpointId: (index % 2) + 1,
      flags: 0,
      pins: 0,
      payload: new Uint8Array([index % 256]),
    };
  });
  return { totalPackets: BigInt(totalPackets), rows };
}

function createSource() {
  const calls = [];
  const source = {
    calls,
    read(startRow, rowCount) {
      const call = deferred();
      calls.push({ startRow, rowCount, resolve: call.resolve, reject: call.reject });
      return call.promise;
    },
  };
  return source;
}

function createSession(source, overrides = {}) {
  return new ViewportSession(source, {
    rowHeightPx: ROW,
    overscanRows: OVERSCAN,
    sourceKind: "demo",
    ...overrides,
  });
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

async function settle() {
  await flush();
  await flush();
}

/** Bootstraps a session with a resolved first batch. */
async function boot(overrides = {}) {
  const source = createSource();
  const session = createSession(source, overrides);
  session.setViewportHeight(VIEWPORT);
  source.calls[0].resolve(makeBatch(0, 15, 1000));
  await settle();
  return { source, session };
}

test("first fetch bootstraps on setViewportHeight and adopts the reported total", async () => {
  const source = createSource();
  const session = createSession(source);
  assert.equal(session.getState().status, "idle");

  session.setViewportHeight(VIEWPORT);
  assert.equal(source.calls.length, 1);
  assert.deepEqual(
    { start: source.calls[0].startRow, count: source.calls[0].rowCount },
    { start: 0, count: 15 },
    "bootstrap reads one visible page + one overscan margin"
  );
  assert.equal(session.getState().status, "loading");

  source.calls[0].resolve(makeBatch(0, 15, 1000));
  await settle();

  const state = session.getState();
  assert.equal(state.status, "ready");
  assert.equal(state.totalRows, 1000);
  assert.deepEqual(state.visibleWindow, { start: 0, count: 10 });
  assert.deepEqual(state.renderWindow, { start: 0, count: 15 });
  assert.equal(state.rows.length, 15);
  assert.equal(state.rows[0].rowIndex, 0);
  assert.equal(state.sessionEpochNs, EPOCH);
  assert.equal(source.calls.length, 1, "bootstrap window already equals the render window");
});

test("sourceKind is surfaced in the state for demo/live labelling", () => {
  const demo = createSession(createSource(), { sourceKind: "demo" });
  const tauri = createSession(createSource(), { sourceKind: "tauri" });
  assert.equal(demo.getState().sourceKind, "demo");
  assert.equal(tauri.getState().sourceKind, "tauri");
});

test("dedupe: scrolling inside the same render window does not refetch", async () => {
  const { source, session } = await boot();
  session.scrollToPixels(10);
  await settle();
  assert.equal(source.calls.length, 1, "sub-row scroll stays inside the cached render window");
  assert.equal(session.getState().scrollTopPx, 10);
  assert.equal(session.getState().status, "ready");
});

test("dedupe: cache serves a render window that was fetched before", async () => {
  const { source, session } = await boot();
  // Move forward one page, resolve it.
  session.scrollToPixels(2000);
  assert.equal(source.calls.length, 2);
  source.calls[1].resolve(makeBatch(95, 20, 1000));
  await settle();
  const forwardRows = session.getState().rows;
  assert.equal(forwardRows[0].rowIndex, 95);

  // Scroll back into the original cached window: no third read.
  session.scrollToPixels(0);
  await settle();
  assert.equal(source.calls.length, 2, "cached rows must be served without an IPC read");
  assert.equal(session.getState().rows[0].rowIndex, 0);
});

test("loading keeps the previous rows visible (no flicker)", async () => {
  const { source, session } = await boot();
  const previousRows = session.getState().rows;
  session.scrollToPixels(2000);
  assert.equal(session.getState().status, "loading");
  assert.equal(session.getState().rows, previousRows, "rows array is not cleared while fetching");
});

test("race: a slow old fetch resolving last cannot overwrite the newer window", async () => {
  const { source, session } = await boot();

  session.scrollToPixels(2000);
  session.scrollToPixels(4000);
  assert.equal(source.calls.length, 3);
  assert.deepEqual(
    { start: source.calls[1].startRow, count: source.calls[1].rowCount },
    { start: 95, count: 20 }
  );
  assert.deepEqual(
    { start: source.calls[2].startRow, count: source.calls[2].rowCount },
    { start: 195, count: 20 }
  );

  // Newer request resolves first...
  source.calls[2].resolve(makeBatch(195, 20, 1000));
  await settle();
  assert.equal(session.getState().rows[0].rowIndex, 195);
  assert.equal(session.getState().status, "ready");

  // ...then the stale one trickles in and must be discarded.
  source.calls[1].resolve(makeBatch(95, 20, 1000));
  await settle();
  assert.equal(session.getState().rows[0].rowIndex, 195, "stale batch was applied");
  assert.equal(session.getState().status, "ready");
});

test("auto-scroll follows a growing total and clamps to the bottom", async () => {
  // Boot with 100 rows (boot() defaults to 1000, which would put the bottom far
  // below the 200px viewport).
  const source = createSource();
  const session = createSession(source);
  session.setViewportHeight(VIEWPORT);
  source.calls[0].resolve(makeBatch(0, 15, 100));
  await settle();

  session.setAutoScroll(true);
  assert.equal(session.getState().autoScroll, true);
  assert.equal(session.getState().scrollTopPx, 1800, "100 rows * 20px - 200px viewport");
  assert.equal(session.getState().visibleWindow.start, 90);
  assert.deepEqual(
    { start: source.calls[1].startRow, count: source.calls[1].rowCount },
    { start: 85, count: 15 }
  );
  source.calls[1].resolve(makeBatch(85, 15, 100));
  await settle();

  session.setTotalRows(200);
  assert.equal(session.getState().scrollTopPx, 3800);
  assert.equal(session.getState().visibleWindow.start, 190);
  assert.deepEqual(
    { start: source.calls[2].startRow, count: source.calls[2].rowCount },
    { start: 185, count: 15 }
  );
  source.calls[2].resolve(makeBatch(185, 15, 200));
  await settle();
  assert.equal(session.getState().scrollTopPx, 3800);

  // Shrinking the store re-clamps instead of leaving a blank window.
  session.setTotalRows(50);
  assert.equal(session.getState().scrollTopPx, 800);
  assert.equal(session.getState().visibleWindow.start, 40);
});

test("scrollByPixels translates line and page delta modes", async () => {
  const { session } = await boot();
  session.scrollByPixels(3, "line");
  assert.equal(session.getState().scrollTopPx, 60);
  session.scrollByPixels(1, "page");
  assert.equal(session.getState().scrollTopPx, 260);
  session.scrollByPixels(10, "pixel");
  assert.equal(session.getState().scrollTopPx, 270);
});

test("selected-frame snapshot persists after the row leaves the fetched slice", async () => {
  const { source, session } = await boot();
  session.selectRow(3);
  assert.equal(session.getState().selectedRowIndex, 3);
  assert.equal(session.getState().selectedFrame?.rowIndex, 3);

  session.scrollToPixels(4000);
  source.calls[1].resolve(makeBatch(195, 20, 1000));
  await settle();

  assert.equal(session.getState().rows[0].rowIndex, 195);
  assert.equal(session.getState().selectedFrame?.rowIndex, 3, "detail readout must survive scrolling");
  assert.equal(session.getState().selectedRowIndex, 3);
});

test("selecting a row outside the fetched slice keeps the index without inventing a frame", async () => {
  const { session } = await boot();
  session.selectRow(9999);
  assert.equal(session.getState().selectedRowIndex, 9999);
  assert.equal(session.getState().selectedFrame, null);

  session.selectRow(-5);
  assert.equal(session.getState().selectedRowIndex, 0);
  assert.equal(session.getState().selectedFrame?.rowIndex, 0, "row 0 is cached");
});

test("source rejection sets error status with the message; refresh recovers", async () => {
  const source = createSource();
  const session = createSession(source);
  session.setViewportHeight(VIEWPORT);
  source.calls[0].reject(new Error("IPC viewport unavailable"));
  await settle();

  assert.equal(session.getState().status, "error");
  assert.equal(session.getState().error, "IPC viewport unavailable");
  assert.equal(session.getState().rows.length, 0);

  session.refresh();
  assert.equal(source.calls.length, 2, "refresh forces a new read");
  assert.equal(session.getState().status, "loading");
  source.calls[1].resolve(makeBatch(0, 15, 1000));
  await settle();

  assert.equal(session.getState().status, "ready");
  assert.equal(session.getState().error, null);
  assert.equal(session.getState().rows.length, 15);
});

test("an error while scrolled keeps the previously loaded rows on screen", async () => {
  const { source, session } = await boot();
  session.scrollToPixels(2000);
  source.calls[1].reject(new Error("capture transport closed"));
  await settle();

  assert.equal(session.getState().status, "error");
  assert.equal(session.getState().error, "capture transport closed");
  assert.equal(session.getState().rows.length, 15, "previous rows remain for the visible window");
});

test("refresh force-reads even when the render window is fully cached", async () => {
  const { source, session } = await boot();
  session.refresh();
  assert.equal(source.calls.length, 2);
  assert.equal(session.getState().status, "loading");
  source.calls[1].resolve(makeBatch(0, 15, 1000));
  await settle();
  assert.equal(session.getState().status, "ready");
});

test("dispose ignores in-flight responses and stops notifying listeners", async () => {
  const source = createSource();
  const session = createSession(source);
  session.setViewportHeight(VIEWPORT);
  let notifications = 0;
  session.subscribe(() => {
    notifications += 1;
  });
  const beforeDispose = notifications;

  session.dispose();
  source.calls[0].resolve(makeBatch(0, 15, 1000));
  await settle();

  assert.equal(session.getState().rows.length, 0);
  assert.equal(session.getState().status, "loading", "late data must not flip status to ready");
  assert.equal(notifications, beforeDispose, "no listener callbacks after dispose");
});

test("setViewportHeight(0) collapses the windows without throwing or fetching", async () => {
  const { source, session } = await boot();
  session.setViewportHeight(0);
  assert.deepEqual(session.getState().visibleWindow, { start: 0, count: 0 });
  assert.deepEqual(session.getState().renderWindow, { start: 0, count: 0 });
  assert.equal(source.calls.length, 1);
});

test("subscribe/unsubscribe works and the snapshot object is stable when nothing changes", async () => {
  const { session } = await boot();
  let notifications = 0;
  const unsubscribe = session.subscribe(() => {
    notifications += 1;
  });

  session.setTotalRows(2000);
  assert.ok(notifications >= 1);
  const afterUpdate = notifications;
  unsubscribe();
  session.setTotalRows(2001);
  assert.equal(notifications, afterUpdate, "unsubscribed listener is not called");

  const snapshot = session.getState();
  session.scrollToPixels(session.getState().scrollTopPx);
  assert.equal(session.getState(), snapshot, "no-op intents must not allocate a new snapshot");
});

test("an empty store is ready, not an error (0 total_packets is legitimate)", async () => {
  const source = createSource();
  const session = createSession(source);
  session.setViewportHeight(VIEWPORT);
  source.calls[0].resolve({ totalPackets: 0n, rows: [] });
  await settle();

  const state = session.getState();
  assert.equal(state.status, "ready", "an empty capture session must not render CAPTURE DISCONNECTED");
  assert.equal(state.error, null);
  assert.equal(state.totalRows, 0);
  assert.equal(state.rows.length, 0);
  assert.deepEqual(state.visibleWindow, { start: 0, count: 0 });
});