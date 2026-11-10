/**
 * Tauri/browser topology RPC client for Step 7.1.
 *
 * Tauri mode calls the Lead-owned commands:
 *   get_topology, create_virtual_pair({request}), add_route({request}),
 *   remove_route({routeId}), remove_endpoint({endpointId})
 * — all return a canonical `{ endpoints, routes }` snapshot.
 *
 * Browser `vite dev` (non-Tauri) mode uses the in-memory demo adapter from
 * `topologyModel.ts` (dynamic import, so production bundles do not load a demo
 * topology). Any production non-Tauri call fails loudly instead of showing
 * fake routes.
 */

import { invoke } from "@tauri-apps/api/core";
import type { CanonicalTopologySnapshot } from "../types/topology.ts";
import type {
  AddRouteRequest,
  CreateVirtualPairRequest,
  DemoTopologyStore,
  RemoveEndpointRequest,
  RemoveRouteRequest,
} from "./topologyModel.ts";

export type {
  AddRouteRequest,
  CreateVirtualPairRequest,
  RemoveEndpointRequest,
  RemoveRouteRequest,
} from "./topologyModel.ts";

export type TopologySnapshot = CanonicalTopologySnapshot;

/** Same runtime detection pattern used by App.tsx / viewportClient.ts. */
export function isTauriRuntime(): boolean {
  return (
    typeof window !== "undefined" &&
    ("__TAURI_INTERNALS__" in window || "__TAURI__" in window)
  );
}

let demoStorePromise: Promise<DemoTopologyStore> | null = null;

async function demoStore(): Promise<DemoTopologyStore> {
  if (!import.meta.env.DEV) {
    throw new Error(
      "DEMO topology is only available in vite dev mode. Run the OmniPort desktop app under Tauri for the live topology RPC."
    );
  }
  demoStorePromise ??= import("./topologyModel.ts").then(
    (model) => new model.DemoTopologyStore()
  );
  return demoStorePromise;
}

export async function fetchTopology(): Promise<TopologySnapshot> {
  if (isTauriRuntime()) {
    return invoke<TopologySnapshot>("get_topology");
  }
  return (await demoStore()).getSnapshot();
}

export async function createVirtualPair(
  request: CreateVirtualPairRequest
): Promise<TopologySnapshot> {
  if (isTauriRuntime()) {
    return invoke<TopologySnapshot>("create_virtual_pair", { request });
  }
  return (await demoStore()).createVirtualPair(request);
}

export async function addRoute(request: AddRouteRequest): Promise<TopologySnapshot> {
  if (isTauriRuntime()) {
    return invoke<TopologySnapshot>("add_route", { request });
  }
  return (await demoStore()).addRoute(request);
}

export async function removeRoute(routeId: number): Promise<TopologySnapshot> {
  if (isTauriRuntime()) {
    return invoke<TopologySnapshot>("remove_route", { routeId });
  }
  return (await demoStore()).removeRoute(routeId);
}

export async function removeEndpoint(endpointId: number): Promise<TopologySnapshot> {
  if (isTauriRuntime()) {
    return invoke<TopologySnapshot>("remove_endpoint", { endpointId });
  }
  return (await demoStore()).removeEndpoint(endpointId);
}
