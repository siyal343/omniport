/**
 * Pure canonical-topology mutation model (no React, DOM, Tauri or fetch).
 *
 * This is the single tested source of truth for the browser demo adapter and
 * for the mutation semantics the Tauri host is expected to mirror:
 *   - `createVirtualPair` allocates two endpoint ids + one bidirectional pair route
 *   - `addRoute` validates known/live endpoints, self-loops and duplicates
 *   - `removeRoute` removes exactly one route
 *   - `removeEndpoint` prunes the endpoint and every referencing route/target
 *
 * Every reducer returns a brand-new snapshot; the input is never mutated and
 * the result shares no arrays/objects with it.
 */

import type {
  CanonicalEndpoint,
  CanonicalRoute,
  CanonicalTopologySnapshot,
  WireFaultConfig,
} from "../types/topology.ts";

export interface CreateVirtualPairRequest {
  portA: string;
  portB: string;
  baudRate?: number;
  dataBits?: number;
  parity?: number;
  stopBits?: number;
}

export interface AddRouteRequest {
  sourceId: number;
  targetIds: number[];
  bidirectional?: boolean;
  fault?: WireFaultConfig;
}

export interface RemoveRouteRequest {
  routeId: number;
}

export interface RemoveEndpointRequest {
  endpointId: number;
}

export const TOPOLOGY_DEMO_VIRTUAL_PAIR_BAUD = 115200;

export class TopologyMutationError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "TopologyMutationError";
  }
}

function cloneFault(fault: WireFaultConfig | null | undefined): WireFaultConfig | null {
  if (!fault) return null;
  return {
    fixed_delay: { ...fault.fixed_delay },
    jitter_mean: { ...fault.jitter_mean },
    jitter_std_dev: { ...fault.jitter_std_dev },
    rate_limit_bps: fault.rate_limit_bps ?? null,
    disconnected: fault.disconnected,
    bit_error_rate: fault.bit_error_rate,
    gilbert_elliott: fault.gilbert_elliott ? { ...fault.gilbert_elliott } : null,
    corrupt_nth_crc: fault.corrupt_nth_crc ?? null,
  };
}

/** Deep copy of a canonical snapshot (no shared mutation with callers). */
export function cloneTopologySnapshot(
  snapshot: CanonicalTopologySnapshot
): CanonicalTopologySnapshot {
  return {
    endpoints: snapshot.endpoints.map((endpoint) => ({ ...endpoint })),
    routes: snapshot.routes.map((route) => ({
      ...route,
      target_ids: [...route.target_ids],
      fault: cloneFault(route.fault),
    })),
  };
}

/** Demo fault preset mirrored onto route 1<->2 (5 ms delay, 1.5 ms jitter, BER 1e-4). */
export function demoWireFault(): WireFaultConfig {
  return {
    fixed_delay: { secs: 0, nanos: 5_000_000 },
    jitter_mean: { secs: 0, nanos: 2_000_000 },
    jitter_std_dev: { secs: 0, nanos: 1_500_000 },
    rate_limit_bps: null,
    disconnected: false,
    bit_error_rate: 1e-4,
    gilbert_elliott: { p_gb: 0.01, p_bg: 0.1, p_g: 0, p_b: 0.5 },
    corrupt_nth_crc: null,
  };
}

/**
 * Demo seed used only by the browser `vite dev` adapter (never by Tauri):
 * HOST UART, DUT VIRTUAL PAIR, WIRESHARK EXTCAP + a bidirectional 1<->2 route
 * carrying the demo fault and a 1->3 route.
 */
export function createDemoTopologySnapshot(): CanonicalTopologySnapshot {
  return {
    endpoints: [
      { id: 1, name: "HOST UART", port_type: "Uart", baud_rate: 115200, is_active: true },
      { id: 2, name: "DUT VIRTUAL PAIR", port_type: "VirtualPair", baud_rate: 115200, is_active: true },
      { id: 3, name: "WIRESHARK EXTCAP", port_type: "Pty", baud_rate: 921600, is_active: true },
    ],
    routes: [
      {
        route_id: 100,
        source_id: 1,
        target_ids: [2],
        bidirectional: true,
        fault: demoWireFault(),
      },
      {
        route_id: 101,
        source_id: 1,
        target_ids: [3],
        bidirectional: false,
        fault: null,
      },
    ],
  };
}

function nextEndpointId(snapshot: CanonicalTopologySnapshot): number {
  return snapshot.endpoints.reduce((max, endpoint) => Math.max(max, endpoint.id), 0) + 1;
}

function nextRouteId(snapshot: CanonicalTopologySnapshot): number {
  return snapshot.routes.reduce((max, route) => Math.max(max, route.route_id), 0) + 1;
}

function endpointExists(snapshot: CanonicalTopologySnapshot, id: number): boolean {
  return snapshot.endpoints.some((endpoint) => endpoint.id === id);
}

/** True when `route` connects `a -> b` in either stored direction. */
function routeConnectsPair(route: CanonicalRoute, a: number, b: number): boolean {
  if (route.source_id === a && route.target_ids.includes(b)) return true;
  if (route.bidirectional && route.source_id === b && route.target_ids.includes(a)) return true;
  return false;
}

export function createVirtualPairMutation(
  snapshot: CanonicalTopologySnapshot,
  request: CreateVirtualPairRequest
): CanonicalTopologySnapshot {
  const portA = request.portA.trim();
  const portB = request.portB.trim();
  if (!portA || !portB) {
    throw new TopologyMutationError("Virtual pair requires two port names");
  }
  if (portA === portB) {
    throw new TopologyMutationError("Virtual pair port names must differ");
  }

  const next = cloneTopologySnapshot(snapshot);
  const idA = nextEndpointId(next);
  const idB = idA + 1;
  const endpointDefaults = {
    port_type: "VirtualPair",
    baud_rate: request.baudRate ?? TOPOLOGY_DEMO_VIRTUAL_PAIR_BAUD,
    is_active: true,
  };
  next.endpoints.push({ id: idA, name: portA, ...endpointDefaults });
  next.endpoints.push({ id: idB, name: portB, ...endpointDefaults });
  next.routes.push({
    route_id: nextRouteId(next),
    source_id: idA,
    target_ids: [idB],
    bidirectional: true,
    fault: null,
  });
  return next;
}

export function addRouteMutation(
  snapshot: CanonicalTopologySnapshot,
  request: AddRouteRequest
): CanonicalTopologySnapshot {
  const sourceId = request.sourceId;
  if (!endpointExists(snapshot, sourceId)) {
    throw new TopologyMutationError(`Unknown source endpoint ${sourceId}`);
  }

  const targetIds = request.targetIds ?? [];
  if (targetIds.length === 0) {
    throw new TopologyMutationError("Add route requires at least one target endpoint");
  }
  if (new Set(targetIds).size !== targetIds.length) {
    throw new TopologyMutationError("Duplicate target endpoint in route request");
  }
  for (const targetId of targetIds) {
    if (targetId === sourceId) {
      throw new TopologyMutationError(`Self-loop route ${sourceId} -> ${targetId} is not allowed`);
    }
    if (!endpointExists(snapshot, targetId)) {
      throw new TopologyMutationError(`Unknown target endpoint ${targetId}`);
    }
    const duplicate = snapshot.routes.some((route) =>
      routeConnectsPair(route, sourceId, targetId) ||
      routeConnectsPair(route, targetId, sourceId)
    );
    if (duplicate) {
      throw new TopologyMutationError(`Route ${sourceId} <-> ${targetId} already exists`);
    }
  }

  const next = cloneTopologySnapshot(snapshot);
  next.routes.push({
    route_id: nextRouteId(next),
    source_id: sourceId,
    target_ids: [...targetIds],
    bidirectional: request.bidirectional ?? false,
    fault: cloneFault(request.fault),
  });
  return next;
}

export function removeRouteMutation(
  snapshot: CanonicalTopologySnapshot,
  routeId: number
): CanonicalTopologySnapshot {
  if (!snapshot.routes.some((route) => route.route_id === routeId)) {
    throw new TopologyMutationError(`Unknown route ${routeId}`);
  }
  const next = cloneTopologySnapshot(snapshot);
  next.routes = next.routes.filter((route) => route.route_id !== routeId);
  return next;
}

export function removeEndpointMutation(
  snapshot: CanonicalTopologySnapshot,
  endpointId: number
): CanonicalTopologySnapshot {
  if (!endpointExists(snapshot, endpointId)) {
    throw new TopologyMutationError(`Unknown endpoint ${endpointId}`);
  }
  const next = cloneTopologySnapshot(snapshot);
  next.endpoints = next.endpoints.filter((endpoint) => endpoint.id !== endpointId);
  next.routes = next.routes.flatMap((route) => {
    if (route.source_id === endpointId) return [];
    if (!route.target_ids.includes(endpointId)) return [route];
    const target_ids = route.target_ids.filter((id) => id !== endpointId);
    if (target_ids.length === 0) return [];
    return [{ ...route, target_ids }];
  });
  return next;
}

/**
 * In-memory demo topology store (browser `vite dev` only). All methods return a
 * fresh snapshot so callers cannot mutate store state; errors are thrown with
 * human-readable messages, matching the Tauri commands' `Result<_, String>`.
 *
 * NOTE: this module is only imported lazily by `topologyClient.ts` in dev mode,
 * so the demo seed/store never lands in the production main bundle.
 */
export class DemoTopologyStore {
  private snapshot: CanonicalTopologySnapshot;

  constructor(initial: CanonicalTopologySnapshot = createDemoTopologySnapshot()) {
    this.snapshot = cloneTopologySnapshot(initial);
  }

  public getSnapshot(): CanonicalTopologySnapshot {
    return cloneTopologySnapshot(this.snapshot);
  }

  public createVirtualPair(request: CreateVirtualPairRequest): CanonicalTopologySnapshot {
    this.snapshot = createVirtualPairMutation(this.snapshot, request);
    return this.getSnapshot();
  }

  public addRoute(request: AddRouteRequest): CanonicalTopologySnapshot {
    this.snapshot = addRouteMutation(this.snapshot, request);
    return this.getSnapshot();
  }

  public removeRoute(routeId: number): CanonicalTopologySnapshot {
    this.snapshot = removeRouteMutation(this.snapshot, routeId);
    return this.getSnapshot();
  }

  public removeEndpoint(endpointId: number): CanonicalTopologySnapshot {
    this.snapshot = removeEndpointMutation(this.snapshot, endpointId);
    return this.getSnapshot();
  }
}
