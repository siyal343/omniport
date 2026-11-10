/**
 * React binding for the framework-agnostic ViewportSession.
 *
 * The session owns the window; React only subscribes to its stable snapshot.
 * `useSyncExternalStore` guarantees tearing-free reads and a cached snapshot
 * object (the class replaces it only when state actually changes), so the
 * 60 FPS canvas paint loop is not forced to allocate on reads.
 */

import { useSyncExternalStore } from "react";
import { ViewportSession, type ViewportSessionOptions, type ViewportSessionState, type ViewportSource } from "./viewportSession.ts";

export function createViewportSession(
  source: ViewportSource,
  options: ViewportSessionOptions
): ViewportSession {
  return new ViewportSession(source, options);
}

export function useViewportSession(session: ViewportSession): ViewportSessionState {
  return useSyncExternalStore(
    session.subscribe,
    session.getState,
    session.getState
  );
}
