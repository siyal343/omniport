/**
 * Default routing topology fixtures and pure node factories.
 *
 * Deliberately free of React / @xyflow/react runtime imports so `node:test`
 * can load this module directly (the previous topology test only asserted
 * properties of inline literals that no production code ever imported).
 */

import type { OmniPortNode, OmniPortEdge } from "./topologyTypes.ts";
import type {
  CanonicalRoute,
  CanonicalTopologySnapshot,
  EndpointKind,
  FaultConfig,
  WireDuration,
  WireFaultConfig,
} from "../../types/topology.ts";

export const initialNodes: OmniPortNode[] = [
  {
    id: "ep-1",
    type: "endpoint",
    position: { x: 50, y: 100 },
    data: {
      endpointId: 1,
      label: "HOST UART",
      kind: "uart",
      portPath: "COM1",
      baudRate: 115200,
      dataBits: 8,
      parity: "none",
      stopBits: 1,
      status: "connected",
      rxBytes: 142080,
      txBytes: 89450,
      errorCount: 0,
    },
  },
  {
    id: "splitter-1",
    type: "splitter",
    position: { x: 330, y: 100 },
    data: {
      label: "ROUTE SPLITTER",
      fanoutCount: 2,
      queueCapacity: 1024,
      dropPolicy: "drop_oldest",
    },
  },
  {
    id: "fault-1",
    type: "fault",
    position: { x: 580, y: 60 },
    data: {
      label: "HIL JITTER INJECTOR",
      config: {
        enabled: true,
        fixedDelayMs: 5,
        gaussianJitterMeanMs: 2,
        gaussianJitterStdDevMs: 1.5,
        bitErrorRate: 1e-4,
        burstErrorProb: 0.01,
        corruptNthCrc: 0,
      },
    },
  },
  {
    id: "ep-2",
    type: "endpoint",
    position: { x: 860, y: 60 },
    data: {
      endpointId: 2,
      label: "DUT VIRTUAL PAIR",
      kind: "virtual_pair",
      portPath: "COM3",
      baudRate: 115200,
      dataBits: 8,
      parity: "none",
      stopBits: 1,
      status: "connected",
      rxBytes: 89450,
      txBytes: 142080,
      errorCount: 2,
    },
  },
  {
    id: "ep-3",
    type: "endpoint",
    position: { x: 860, y: 220 },
    data: {
      endpointId: 3,
      label: "WIRESHARK EXTCAP",
      kind: "pty",
      portPath: "\\\\.\\pipe\\omniport_extcap",
      baudRate: 921600,
      dataBits: 8,
      parity: "none",
      stopBits: 1,
      status: "connected",
      rxBytes: 231530,
      txBytes: 0,
      errorCount: 0,
    },
  },
];

export const initialRoutes: CanonicalRoute[] = [
  {
    route_id: 100,
    source_id: 1,
    target_ids: [2, 3],
    bidirectional: false,
    fault: null,
  },
];

export const initialEdges: OmniPortEdge[] = [
  {
    id: "e-ep1-split",
    source: "ep-1",
    target: "splitter-1",
    type: "animated",
    data: {
      routeId: 100,
      sourceEndpointId: 1,
      targetEndpointId: 2,
      bidirectional: false,
      packetRate: 140,
      bandwidthBps: 112000,
      hasErrors: false,
      active: true,
    },
  },
  {
    id: "e-split-fault",
    source: "splitter-1",
    target: "fault-1",
    type: "animated",
    data: {
      routeId: 100,
      sourceEndpointId: 1,
      targetEndpointId: 2,
      bidirectional: false,
      packetRate: 140,
      bandwidthBps: 112000,
      hasErrors: false,
      active: true,
    },
  },
  {
    id: "e-fault-ep2",
    source: "fault-1",
    target: "ep-2",
    type: "animated",
    data: {
      routeId: 100,
      sourceEndpointId: 1,
      targetEndpointId: 2,
      bidirectional: false,
      packetRate: 138,
      bandwidthBps: 110400,
      hasErrors: true,
      active: true,
    },
  },
  {
    id: "e-split-ep3",
    source: "splitter-1",
    target: "ep-3",
    type: "animated",
    data: {
      routeId: 100,
      sourceEndpointId: 1,
      targetEndpointId: 3,
      bidirectional: false,
      packetRate: 140,
      bandwidthBps: 112000,
      hasErrors: false,
      active: true,
    },
  },
];

/** Returns the next unused `<prefix>-<n>` id for the given topology. */
export function nextNodeId(prefix: string, nodes: ReadonlyArray<OmniPortNode>): string {
  const used = new Set(nodes.map((node) => node.id));
  let n = 1;
  while (used.has(`${prefix}-${n}`)) n += 1;
  return `${prefix}-${n}`;
}

function nextEndpointId(nodes: ReadonlyArray<OmniPortNode>): number {
  let maxId = 0;
  for (const node of nodes) {
    if (node.type === "endpoint" && typeof node.data.endpointId === "number") {
      maxId = Math.max(maxId, node.data.endpointId);
    }
  }
  return maxId + 1;
}

/**
 * Creates a new virtual-pair endpoint node without mutating the existing graph.
 * Previously this logic was inlined in the component and could mint duplicate
 * ids / duplicate COM port paths after mixed node additions.
 */
export function createVirtualPairNode(nodes: ReadonlyArray<OmniPortNode>): OmniPortNode {
  const endpointId = nextEndpointId(nodes);
  const usedPorts = new Set(
    nodes
      .filter((node) => node.type === "endpoint")
      .map((node) => node.data.portPath)
  );
  let virtualComNumber = nodes.filter((node) => node.type === "endpoint").length + 4;
  while (usedPorts.has(`COM${virtualComNumber}`)) virtualComNumber += 1;

  return {
    id: nextNodeId("ep", nodes),
    type: "endpoint",
    position: {
      x: 100 + (nodes.length % 4) * 110,
      y: 150 + (nodes.length % 3) * 70,
    },
    data: {
      endpointId,
      label: `VIRTUAL COM${virtualComNumber}`,
      kind: "virtual_pair",
      portPath: `COM${virtualComNumber}`,
      baudRate: 115200,
      dataBits: 8,
      parity: "none",
      stopBits: 1,
      status: "connected",
      rxBytes: 0,
      txBytes: 0,
      errorCount: 0,
    },
  };
}

/** Creates a new fault-injector node without mutating the existing graph. */
export function createFaultNode(nodes: ReadonlyArray<OmniPortNode>): OmniPortNode {
  return {
    id: nextNodeId("fault", nodes),
    type: "fault",
    position: {
      x: 450 + (nodes.length % 3) * 90,
      y: 150 + (nodes.length % 2) * 90,
    },
    data: {
      label: "FAULT INJECTOR",
      config: {
        enabled: true,
        fixedDelayMs: 10,
        gaussianJitterMeanMs: 5,
        gaussianJitterStdDevMs: 2,
        bitErrorRate: 1e-3,
        burstErrorProb: 0.05,
        corruptNthCrc: 10,
      },
    },
  };
}

function wireDurationToMs(d: WireDuration): number {
  return d.secs * 1000 + d.nanos / 1_000_000;
}

export function wireFaultToUiConfig(fault: WireFaultConfig): FaultConfig {
  const fixedDelayMs = wireDurationToMs(fault.fixed_delay);
  const gaussianJitterMeanMs = wireDurationToMs(fault.jitter_mean);
  const gaussianJitterStdDevMs = wireDurationToMs(fault.jitter_std_dev);
  const burstErrorProb = fault.gilbert_elliott?.p_gb ?? 0;
  const corruptNthCrc = fault.corrupt_nth_crc ?? 0;
  const enabled =
    fault.disconnected ||
    fixedDelayMs > 0 ||
    gaussianJitterMeanMs > 0 ||
    gaussianJitterStdDevMs > 0 ||
    fault.bit_error_rate > 0 ||
    burstErrorProb > 0 ||
    corruptNthCrc > 0 ||
    fault.rate_limit_bps != null;
  return {
    enabled,
    fixedDelayMs,
    gaussianJitterMeanMs,
    gaussianJitterStdDevMs,
    bitErrorRate: fault.bit_error_rate,
    burstErrorProb,
    corruptNthCrc,
  };
}

function inferEndpointKind(portType: string): EndpointKind {
  const lower = portType.toLowerCase();
  if (lower.includes("virtual")) return "virtual_pair";
  if (lower.includes("pty") || lower.includes("extcap")) return "pty";
  if (lower.includes("tcp")) return "tcp";
  if (lower.includes("udp")) return "udp";
  return "uart";
}

/**
 * Projects a canonical `CanonicalTopologySnapshot` (matching Rust `TopologySnapshot` /
 * `Response::Topology`) into React Flow nodes and directed route edges, skipping any
 * route references to non-live endpoints or self-loops.
 */
export function projectTopologySnapshot(snapshot: CanonicalTopologySnapshot): {
  nodes: OmniPortNode[];
  edges: OmniPortEdge[];
} {
  const endpointIds = new Set(snapshot.endpoints.map((ep) => ep.id));
  const nodes: OmniPortNode[] = snapshot.endpoints.map((ep, index) => ({
    id: `ep-${ep.id}`,
    type: "endpoint",
    position: {
      x: 80 + (index % 3) * 300,
      y: 80 + Math.floor(index / 3) * 180,
    },
    data: {
      endpointId: ep.id,
      label: ep.name,
      kind: inferEndpointKind(ep.port_type),
      portPath: ep.name,
      baudRate: ep.baud_rate,
      dataBits: 8,
      parity: "none",
      stopBits: 1,
      status: ep.is_active ? "connected" : "disconnected",
      rxBytes: 0,
      txBytes: 0,
      errorCount: 0,
    },
  }));

  const edges: OmniPortEdge[] = [];
  for (const route of snapshot.routes) {
    if (!endpointIds.has(route.source_id)) continue;
    const faultConfig = route.fault ? wireFaultToUiConfig(route.fault) : undefined;
    for (const targetId of route.target_ids) {
      if (!endpointIds.has(targetId) || targetId === route.source_id) continue;
      edges.push({
        id: `route-${route.route_id}-${route.source_id}-${targetId}`,
        source: `ep-${route.source_id}`,
        target: `ep-${targetId}`,
        type: "animated",
        data: {
          routeId: route.route_id,
          sourceEndpointId: route.source_id,
          targetEndpointId: targetId,
          bidirectional: route.bidirectional,
          faultConfig,
          packetRate: 0,
          bandwidthBps: 0,
          hasErrors: Boolean(faultConfig?.enabled),
          active: true,
        },
      });
      if (route.bidirectional) {
        edges.push({
          id: `route-${route.route_id}-${targetId}-${route.source_id}`,
          source: `ep-${targetId}`,
          target: `ep-${route.source_id}`,
          type: "animated",
          data: {
            routeId: route.route_id,
            sourceEndpointId: targetId,
            targetEndpointId: route.source_id,
            bidirectional: true,
            faultConfig,
            packetRate: 0,
            bandwidthBps: 0,
            hasErrors: Boolean(faultConfig?.enabled),
            active: true,
          },
        });
      }
    }
  }

  return { nodes, edges };
}

/**
 * Resolves real endpoint IDs when creating an edge in the interactive canvas,
 * rejecting self-loops and connections to non-existent nodes instead of inventing
 * hardcoded fallback IDs.
 */
export function createRouteEdgeForConnection(
  connection: { source: string | null; target: string | null },
  nodes: ReadonlyArray<OmniPortNode>
): OmniPortEdge | null {
  const { source, target } = connection;
  if (!source || !target || source === target) return null;

  const sourceNode = nodes.find((node) => node.id === source);
  const targetNode = nodes.find((node) => node.id === target);
  if (!sourceNode || !targetNode) return null;

  const sourceEndpointId =
    sourceNode.type === "endpoint" && typeof sourceNode.data.endpointId === "number"
      ? sourceNode.data.endpointId
      : 0;
  const targetEndpointId =
    targetNode.type === "endpoint" && typeof targetNode.data.endpointId === "number"
      ? targetNode.data.endpointId
      : 0;

  if (
    sourceNode.type === "endpoint" &&
    targetNode.type === "endpoint" &&
    (sourceEndpointId <= 0 || targetEndpointId <= 0 || sourceEndpointId === targetEndpointId)
  ) {
    return null;
  }

  return {
    id: `edge-${source}-${target}`,
    source,
    target,
    type: "animated",
    data: {
      sourceEndpointId,
      targetEndpointId,
      bidirectional: false,
      packetRate: 0,
      bandwidthBps: 0,
      hasErrors: false,
      active: true,
    },
  };
}

/** HIL fault preset used by the route-scoped "Attach Fault" action. */
export const ROUTE_FAULT_PRESET: FaultConfig = {
  enabled: true,
  fixedDelayMs: 5,
  gaussianJitterMeanMs: 2,
  gaussianJitterStdDevMs: 1.5,
  bitErrorRate: 1e-4,
  burstErrorProb: 0.01,
  corruptNthCrc: 0,
};

/** UI ms-based fault config -> canonical wire fault (inverse of wireFaultToUiConfig). */
export function uiFaultToWireConfig(config: FaultConfig): WireFaultConfig {
  const toDuration = (ms: number): WireDuration => ({
    secs: Math.floor(ms / 1000),
    nanos: Math.round((ms % 1000) * 1_000_000),
  });
  return {
    fixed_delay: toDuration(config.fixedDelayMs),
    jitter_mean: toDuration(config.gaussianJitterMeanMs),
    jitter_std_dev: toDuration(config.gaussianJitterStdDevMs),
    rate_limit_bps: null,
    disconnected: false,
    bit_error_rate: config.bitErrorRate,
    gilbert_elliott:
      config.burstErrorProb > 0
        ? { p_gb: config.burstErrorProb, p_bg: 0.1, p_g: 0, p_b: 0.5 }
        : null,
    corrupt_nth_crc: config.corruptNthCrc > 0 ? config.corruptNthCrc : null,
  };
}

/**
 * Re-projects a canonical snapshot onto the interactive canvas while keeping
 * every user-dragged node position (matched by node id). Nodes that no longer
 * exist in the snapshot are dropped; brand-new nodes get a non-overlapping
 * default slot. This prevents the graph from jumping after a mutation.
 */
export function mergeNodePositions(
  projected: ReadonlyArray<OmniPortNode>,
  previous: ReadonlyArray<OmniPortNode>
): OmniPortNode[] {
  const previousById = new Map(previous.map((node) => [node.id, node.position]));
  const occupied = new Set(previous.map((node) => `${node.position.x}:${node.position.y}`));

  return projected.map((node) => {
    const kept = previousById.get(node.id);
    if (kept) return { ...node, position: { ...kept } };

    let { x, y } = node.position;
    while (occupied.has(`${x}:${y}`)) x += 260;
    occupied.add(`${x}:${y}`);
    return { ...node, position: { x, y } };
  });
}

/**
 * Next free COM port names for a virtual pair: COM10/COM11, then COM12/COM13…
 * (skips any pair already present in the canonical snapshot).
 *
 * Lives here (not in the demo adapter) because the live canvas computes the
 * request for `create_virtual_pair`; the demo topology module must stay
 * lazily-loaded in production builds.
 */
export function nextVirtualPairPortNames(snapshot: CanonicalTopologySnapshot): {
  portA: string;
  portB: string;
} {
  const used = new Set(snapshot.endpoints.map((endpoint) => endpoint.name));
  let base = 10;
  while (used.has(`COM${base}`) || used.has(`COM${base + 1}`)) base += 2;
  return { portA: `COM${base}`, portB: `COM${base + 1}` };
}