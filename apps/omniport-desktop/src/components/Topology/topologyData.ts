/**
 * Default routing topology fixtures and pure node factories.
 *
 * Deliberately free of React / @xyflow/react runtime imports so `node:test`
 * can load this module directly (the previous topology test only asserted
 * properties of inline literals that no production code ever imported).
 */

import type { OmniPortNode, OmniPortEdge } from "./topologyTypes.ts";

export const initialNodes: OmniPortNode[] = [
  {
    id: "ep-1",
    type: "endpoint",
    position: { x: 50, y: 100 },
    data: {
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

export const initialEdges: OmniPortEdge[] = [
  {
    id: "e-ep1-split",
    source: "ep-1",
    target: "splitter-1",
    type: "animated",
    data: {
      sourceEndpointId: 1,
      targetEndpointId: 100,
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
      sourceEndpointId: 100,
      targetEndpointId: 200,
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
      sourceEndpointId: 200,
      targetEndpointId: 2,
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
      sourceEndpointId: 100,
      targetEndpointId: 3,
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

/**
 * Creates a new virtual-pair endpoint node without mutating the existing graph.
 * Previously this logic was inlined in the component and could mint duplicate
 * ids / duplicate COM port paths after mixed node additions.
 */
export function createVirtualPairNode(nodes: ReadonlyArray<OmniPortNode>): OmniPortNode {
  const virtualComNumber = nodes.filter((node) => node.type === "endpoint").length + 4;
  return {
    id: nextNodeId("ep", nodes),
    type: "endpoint",
    position: {
      x: 100 + (nodes.length % 4) * 110,
      y: 150 + (nodes.length % 3) * 70,
    },
    data: {
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
