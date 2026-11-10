/**
 * ViewportSession — the single owner of the desktop capture viewport window.
 *
 * Deep module: callers can only read a stable snapshot and send intents
 * (resize, scroll, select, auto-scroll, refresh). Everything hard about the
 * window lives behind this seam:
 *   - pixel <-> row-range math (viewportWindow.ts)
 *   - request tickets so a slow old fetch can never overwrite a newer one
 *   - overscan fetch de-duplication + absolute-row cache
 *   - loading state that keeps the previous rows (no flicker)
 *   - auto-scroll follow-bottom when totalRows grows
 *   - session epoch derivation for the TIME OFFSET column
 *   - selected-frame snapshot persistence while scrolling away
 *   - error capture for the CAPTURE DISCONNECTED banner, dispose cancellation
 *
 * No React or DOM imports, so node:test drives the real state machine with a
 * fake async ViewportSource.
 */

import type { ViewportRowData } from "../types/serial.ts";
import type { ViewportBatch } from "./viewportDecoder.ts";
import { ViewportRequestGate } from "./viewportRequestGate.ts";
import { batchEpochNs, nextSessionEpochNs } from "./viewportRows.ts";
import {
  clampScrollTop,
  computeScrollMax,
  computeWindows,
  renderWindowKey,
  type WindowRange,
} from "./viewportWindow.ts";

export type ViewportSourceKind = "tauri" | "demo";
export type ViewportStatus = "idle" | "loading" | "ready" | "error";
export type ScrollDeltaMode = "pixel" | "line" | "page";

/** The only thing a viewport needs from the outside world. */
export interface ViewportSource {
  read(startRow: number, rowCount: number): Promise<ViewportBatch>;
}

export interface ViewportSessionOptions {
  rowHeightPx: number;
  overscanRows: number;
  sourceKind: ViewportSourceKind;
  initialTotalRows?: number;
}

export interface ViewportSessionState {
  totalRows: number;
  scrollTopPx: number;
  viewportHeightPx: number;
  visibleWindow: WindowRange;
  renderWindow: WindowRange;
  /** Fetched rows for the current render window, in ascending row order. */
  rows: ViewportRowData[];
  /** Cached rows keyed by absolute row index (read-only diagnostics). */
  rowCacheSize: number;
  selectedRowIndex: number | null;
  selectedFrame: ViewportRowData | null;
  autoScroll: boolean;
  sessionEpochNs: bigint;
  status: ViewportStatus;
  error: string | null;
  sourceKind: ViewportSourceKind;
}

const ROW_CACHE_CAPACITY = 4096;
const EMPTY_WINDOW_RANGE: WindowRange = { start: 0, count: 0 };

export class ViewportSession {
  private readonly source: ViewportSource;
  private readonly rowHeightPx: number;
  private readonly overscanRows: number;
  private readonly requestGate = new ViewportRequestGate();
  private readonly rowCache = new Map<number, ViewportRowData>();
  private readonly listeners = new Set<() => void>();

  private state: ViewportSessionState;
  private epochNs: bigint | null = null;
  private lastFetchKey: string | null = null;
  private disposed = false;

  constructor(source: ViewportSource, options: ViewportSessionOptions) {
    this.source = source;
    this.rowHeightPx = Math.max(1, options.rowHeightPx);
    this.overscanRows = Math.max(0, options.overscanRows);
    const totalRows = Math.max(0, options.initialTotalRows ?? 0);
    const calc = computeWindows(0, 0, this.rowHeightPx, totalRows, this.overscanRows);
    this.state = {
      totalRows,
      scrollTopPx: 0,
      viewportHeightPx: 0,
      visibleWindow: calc.visible,
      renderWindow: calc.render,
      rows: [],
      rowCacheSize: 0,
      selectedRowIndex: null,
      selectedFrame: null,
      autoScroll: false,
      sessionEpochNs: 0n,
      status: "idle",
      error: null,
      sourceKind: options.sourceKind,
    };
  }

  /* ------------------------------------------------------------------ */
  /* Read side (stable snapshot for useSyncExternalStore)                */
  /* ------------------------------------------------------------------ */

  public getState = (): ViewportSessionState => this.state;

  public subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  /* ------------------------------------------------------------------ */
  /* Intents                                                             */
  /* ------------------------------------------------------------------ */

  /** Height of the scrollable row area (canvas height minus sticky header). */
  public setViewportHeight(viewportHeightPx: number): void {
    const height = Number.isFinite(viewportHeightPx) ? Math.max(0, Math.floor(viewportHeightPx)) : 0;
    if (height === this.state.viewportHeightPx) return;
    this.reposition({ viewportHeightPx: height });
  }

  public setTotalRows(totalRows: number): void {
    const total = Number.isFinite(totalRows) ? Math.max(0, Math.floor(totalRows)) : 0;
    if (total === this.state.totalRows) return;
    this.reposition({ totalRows: total });
  }

  public scrollToPixels(scrollTopPx: number): void {
    this.reposition({ scrollTopPx });
  }

  public scrollByPixels(deltaPx: number, deltaMode: ScrollDeltaMode = "pixel"): void {
    if (!Number.isFinite(deltaPx)) return;
    let pixels = deltaPx;
    if (deltaMode === "line") pixels *= this.rowHeightPx;
    else if (deltaMode === "page") pixels *= Math.max(1, this.state.viewportHeightPx);
    this.scrollToPixels(this.state.scrollTopPx + pixels);
  }

  public selectRow(rowIndex: number): void {
    if (!Number.isFinite(rowIndex)) return;
    const index = Math.max(0, Math.floor(rowIndex));
    const frame = this.rowCache.get(index) ?? this.state.rows.find((row) => row.rowIndex === index) ?? null;
    this.patch({ selectedRowIndex: index, selectedFrame: frame });
  }

  public setAutoScroll(enabled: boolean): void {
    if (enabled === this.state.autoScroll) return;
    this.reposition({ autoScroll: enabled });
  }

  /** Force a re-read of the current window (also recovers from error status). */
  public refresh(): void {
    if (this.disposed) return;
    this.ensureFetch(true);
  }

  public dispose(): void {
    this.disposed = true;
    this.requestGate.invalidate();
    this.listeners.clear();
    this.rowCache.clear();
  }

  /* ------------------------------------------------------------------ */
  /* Internals                                                           */
  /* ------------------------------------------------------------------ */

  private patch(patch: Partial<ViewportSessionState>): void {
    let changed = false;
    for (const key of Object.keys(patch) as Array<keyof ViewportSessionState>) {
      if (patch[key] !== this.state[key]) {
        changed = true;
        break;
      }
    }
    if (!changed) return;
    this.state = { ...this.state, ...patch };
    for (const listener of Array.from(this.listeners)) listener();
  }

  private patchRows(rows: ViewportRowData[]): void {
    const current = this.state.rows;
    if (
      rows.length === current.length &&
      rows.every((row, index) => row === current[index])
    ) {
      return;
    }
    this.patch({ rows });
  }

  /**
   * Applies a position/total/height/auto-scroll change: clamp, recompute both
   * windows, refresh visible rows from cache and fetch if the render window
   * needs rows the cache cannot satisfy.
   */
  private reposition(change: {
    scrollTopPx?: number;
    viewportHeightPx?: number;
    totalRows?: number;
    autoScroll?: boolean;
  }): void {
    if (this.disposed) return;

    const totalRows = change.totalRows ?? this.state.totalRows;
    const viewportHeightPx = change.viewportHeightPx ?? this.state.viewportHeightPx;
    const autoScroll = change.autoScroll ?? this.state.autoScroll;
    const maxScrollPx = computeScrollMax(totalRows, this.rowHeightPx, viewportHeightPx);
    const requestedScroll = autoScroll ? maxScrollPx : (change.scrollTopPx ?? this.state.scrollTopPx);
    const scrollTopPx = clampScrollTop(
      requestedScroll,
      viewportHeightPx,
      this.rowHeightPx,
      totalRows
    );

    if (
      scrollTopPx === this.state.scrollTopPx &&
      totalRows === this.state.totalRows &&
      viewportHeightPx === this.state.viewportHeightPx &&
      autoScroll === this.state.autoScroll
    ) {
      return;
    }

    const calc = computeWindows(
      scrollTopPx,
      viewportHeightPx,
      this.rowHeightPx,
      totalRows,
      this.overscanRows
    );

    this.patch({
      totalRows,
      viewportHeightPx,
      scrollTopPx,
      autoScroll,
      visibleWindow: calc.visible,
      renderWindow: calc.render,
    });

    this.syncRowsFromCache();
    this.ensureFetch();
  }

  private bootstrapRowCount(): number {
    if (this.state.viewportHeightPx <= 0) return 0;
    return Math.ceil(this.state.viewportHeightPx / this.rowHeightPx) + this.overscanRows;
  }

  private isWindowCached(start: number, count: number): boolean {
    if (count <= 0) return false;
    for (let index = start; index < start + count; index++) {
      if (!this.rowCache.has(index)) return false;
    }
    return true;
  }

  private syncRowsFromCache(): void {
    const { renderWindow } = this.state;
    if (renderWindow.count <= 0) {
      this.patchRows([]);
      return;
    }
    const rows: ViewportRowData[] = [];
    for (let index = renderWindow.start; index < renderWindow.start + renderWindow.count; index++) {
      const row = this.rowCache.get(index);
      if (row) rows.push(row);
    }
    // Nothing cached for the new window yet: keep the previous rows array
    // (status already says "loading") instead of flickering to empty. The
    // renderer maps rows by absolute rowIndex, so stale rows are not painted
    // into the new window; they are replaced as soon as the fetch lands.
    if (rows.length === 0) return;
    this.patchRows(rows);
  }

  private cacheRow(rowIndex: number, row: ViewportRowData): void {
    this.rowCache.delete(rowIndex);
    this.rowCache.set(rowIndex, row);
    if (this.rowCache.size > ROW_CACHE_CAPACITY) {
      const oldest = this.rowCache.keys().next().value;
      if (oldest !== undefined) this.rowCache.delete(oldest);
    }
  }

  /**
   * Fetches the render window unless the current window is unchanged or fully
   * served from the row cache.
   */
  private ensureFetch(force = false): void {
    if (this.disposed) return;

    const { renderWindow } = this.state;
    const start = renderWindow.count > 0 ? renderWindow.start : 0;
    const count = renderWindow.count > 0 ? renderWindow.count : this.bootstrapRowCount();
    if (count <= 0) return;

    const key = `${start}:${count}`;
    if (!force) {
      if (key === this.lastFetchKey) return;
      if (this.isWindowCached(start, count)) {
        this.lastFetchKey = key;
        this.syncRowsFromCache();
        return;
      }
    }

    this.lastFetchKey = key;
    void this.fetchWindow(start, count);
  }

  private async fetchWindow(startRow: number, rowCount: number): Promise<void> {
    const ticket = this.requestGate.begin();

    this.patch({ status: "loading", error: null });

    try {
      const batch = await this.source.read(startRow, rowCount);
      // A newer request (or dispose) supersedes this one: drop the stale reply.
      if (this.disposed || !this.requestGate.isCurrent(ticket)) return;

      for (let index = 0; index < batch.rows.length; index++) {
        const row = batch.rows[index];
        this.cacheRow(startRow + index, {
          rowIndex: startRow + index,
          timestampNs: row.timestampNs,
          endpointId: row.endpointId,
          flags: row.flags,
          pins: row.pins,
          payload: row.payload,
        });
      }

      this.epochNs = nextSessionEpochNs(this.epochNs, batchEpochNs(batch));
      const totalRows = Number(batch.totalPackets);
      const nextTotal = Number.isFinite(totalRows) && totalRows >= 0 ? totalRows : this.state.totalRows;
      const totalChanged = nextTotal !== this.state.totalRows;

      const maxScrollPx = computeScrollMax(nextTotal, this.rowHeightPx, this.state.viewportHeightPx);
      const requestedScroll = this.state.autoScroll
        ? maxScrollPx
        : this.state.scrollTopPx;
      const scrollTopPx = clampScrollTop(
        requestedScroll,
        this.state.viewportHeightPx,
        this.rowHeightPx,
        nextTotal
      );
      const calc = computeWindows(
        scrollTopPx,
        this.state.viewportHeightPx,
        this.rowHeightPx,
        nextTotal,
        this.overscanRows
      );

      this.patch({
        totalRows: nextTotal,
        scrollTopPx,
        visibleWindow: calc.visible,
        renderWindow: calc.render,
        rowCacheSize: this.rowCache.size,
        sessionEpochNs: this.epochNs ?? 0n,
        status: "ready",
        error: null,
      });

      this.syncRowsFromCache();

      // Totals (or the follow-bottom position) may have moved the render
      // window; fetch the slice we now need if it differs from this read.
      if (totalChanged || renderWindowKey(this.state.renderWindow) !== `${startRow}:${rowCount}`) {
        this.ensureFetch();
      }
    } catch (err) {
      if (this.disposed || !this.requestGate.isCurrent(ticket)) return;
      this.patch({
        status: "error",
        error: err instanceof Error ? err.message : String(err),
      });
    }
  }
}
