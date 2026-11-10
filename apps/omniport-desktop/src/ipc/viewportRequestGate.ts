/**
 * Monotonic ticket gate that prevents out-of-order viewport fetches from
 * overwriting a newer batch with stale rows.
 *
 * Scenario this fixes: the user wheels from row 0 to row 2400; request A
 * (rows 0..40) resolves after request B (rows 2390..2430). Without the gate the
 * monitor would paint rows 0..40 while the scrollbar and logic strip are
 * anchored at row 2390, i.e. stale AND misaligned.
 *
 * Kept dependency-free so `node:test` can exercise the real code.
 */
export class ViewportRequestGate {
  private latestTicket = 0;

  /** Starts a new request and returns its ticket. */
  public begin(): number {
    this.latestTicket += 1;
    return this.latestTicket;
  }

  /** True when no newer request has started since `ticket` was issued. */
  public isCurrent(ticket: number): boolean {
    return ticket === this.latestTicket;
  }

  /** Invalidates any in-flight request (e.g. on unmount / dataset switch). */
  public invalidate(): void {
    this.latestTicket += 1;
  }
}
