import test from "node:test";
import assert from "node:assert/strict";
import {
  initialNodes,
  initialEdges,
  initialRoutes,
  nextNodeId,
  createVirtualPairNode,
  createFaultNode,
  projectTopologySnapshot,
  createRouteEdgeForConnection,
  mergeNodePositions,
  uiFaultToWireConfig,
  wireFaultToUiConfig,
  ROUTE_FAULT_PRESET,
} from "./topologyData.ts";
import { createDemoTopologySnapshot } from "../../ipc/topologyModel.ts";

const NODE_TYPES = new Set(["endpoint", "splitter", "merger", "fault"]);

test("initial topology graph is internally consistent", () => {
  assert.ok(initialNodes.length >= 5);
  assert.equal(new Set(initialNodes.map((node) => node.id)).size, initialNodes.length);

  for (const node of initialNodes) {
    assert.ok(NODE_TYPES.has(node.type), `unknown node type ${node.type}`);
    assert.ok(Number.isFinite(node.position.x) && Number.isFinite(node.position.y));
    assert.ok(typeof node.data.label === "string" && node.data.label.length > 0);
  }

  const nodeIds = new Set(initialNodes.map((node) => node.id));
  assert.equal(new Set(initialEdges.map((edge) => edge.id)).size, initialEdges.length);
  for (const edge of initialEdges) {
    assert.ok(nodeIds.has(edge.source), `edge ${edge.id} source must exist`);
    assert.ok(nodeIds.has(edge.target), `edge ${edge.id} target must exist`);
    assert.equal(edge.type, "animated");
    assert.ok(edge.data, `edge ${edge.id} needs RouteEdgeData`);
    assert.equal(typeof edge.data.active, "boolean");
    assert.equal(typeof edge.data.hasErrors, "boolean");
    assert.ok(edge.data.packetRate >= 0);
    assert.ok(edge.data.bandwidthBps >= 0);
  }
});

test("endpoint nodes expose the serial configuration required by the UI", () => {
  const endpoints = initialNodes.filter((node) => node.type === "endpoint");
  assert.ok(endpoints.length >= 3);
  for (const node of endpoints) {
    const data = node.data;
    assert.ok(Number.isInteger(data.endpointId) && data.endpointId >= 1);
    assert.ok(typeof data.portPath === "string" && data.portPath.length > 0);
    assert.ok([9600, 19200, 38400, 57600, 115200, 921600].includes(data.baudRate));
    assert.equal(data.dataBits, 8);
    assert.ok(["none", "even", "odd"].includes(data.parity));
    assert.ok([1, 2].includes(data.stopBits));
    assert.ok(["connected", "disconnected", "error"].includes(data.status));
    assert.ok(data.rxBytes >= 0 && data.txBytes >= 0 && data.errorCount >= 0);
  }
});

test("fault node configuration carries jitter, BER and CRC corruption parameters", () => {
  const faults = initialNodes.filter((node) => node.type === "fault");
  assert.ok(faults.length >= 1);
  for (const node of faults) {
    const cfg = node.data.config;
    assert.equal(cfg.enabled, true);
    assert.ok(cfg.fixedDelayMs >= 0);
    assert.ok(cfg.gaussianJitterStdDevMs >= 0);
    assert.ok(cfg.bitErrorRate >= 0 && cfg.bitErrorRate < 1);
    assert.ok(cfg.burstErrorProb >= 0 && cfg.burstErrorProb <= 1);
    assert.ok(Number.isInteger(cfg.corruptNthCrc) && cfg.corruptNthCrc >= 0);
  }
});

test("nextNodeId avoids collisions with existing ids", () => {
  assert.equal(nextNodeId("ep", initialNodes), "ep-4");
  assert.equal(nextNodeId("fault", initialNodes), "fault-2");
  assert.equal(
    nextNodeId("ep", [...initialNodes, createVirtualPairNode(initialNodes)]),
    "ep-5"
  );
});

test("createVirtualPairNode produces a unique endpoint with its own COM path", () => {
  const first = createVirtualPairNode(initialNodes);
  assert.equal(first.type, "endpoint");
  assert.equal(first.data.kind, "virtual_pair");
  assert.equal(first.data.status, "connected");
  assert.ok(!initialNodes.some((node) => node.id === first.id));
  assert.ok(!initialNodes.some((node) => node.data.portPath === first.data.portPath));

  const graph = [...initialNodes, first];
  const second = createVirtualPairNode(graph);
  assert.notEqual(second.id, first.id);
  assert.notEqual(second.data.portPath, first.data.portPath);
});

test("createFaultNode produces a unique enabled injector", () => {
  const fault = createFaultNode(initialNodes);
  assert.equal(fault.type, "fault");
  assert.ok(!initialNodes.some((node) => node.id === fault.id));

  const graph = [...initialNodes, fault];
  const second = createFaultNode(graph);
  assert.notEqual(second.id, fault.id);

  // Repeated additions keep every id unique (the old inline factory used
  // nodes.length and could mint duplicate ids after mixed node additions).
  const allIds = [...initialNodes, fault, second].map((node) => node.id);
  assert.equal(new Set(allIds).size, allIds.length);
});

test("createVirtualPairNode assigns unique sequential endpointId values", () => {
  const first = createVirtualPairNode(initialNodes);
  const second = createVirtualPairNode([...initialNodes, first]);
  assert.equal(first.data.endpointId, 4);
  assert.equal(second.data.endpointId, 5);
});

test("initialEdges reference real endpoint ids aligned with initialRoutes", () => {
  const liveIds = new Set(
    initialNodes
      .filter((node) => node.type === "endpoint")
      .map((node) => node.data.endpointId)
  );
  assert.equal(initialRoutes.length, 1);
  assert.equal(initialRoutes[0].route_id, 100);
  for (const edge of initialEdges) {
    assert.equal(edge.data.routeId, 100);
    assert.ok(liveIds.has(edge.data.sourceEndpointId));
    assert.ok(liveIds.has(edge.data.targetEndpointId));
  }
});

test("projectTopologySnapshot expands 1:N bidirectional routes and filters dead endpoints", () => {
  const { nodes, edges } = projectTopologySnapshot({
    endpoints: [
      { id: 1, name: "COM1", port_type: "VirtualPairSideA", baud_rate: 115200, is_active: true },
      { id: 2, name: "COM2", port_type: "VirtualPairSideB", baud_rate: 115200, is_active: true },
      { id: 3, name: "EXTCAP", port_type: "Pty", baud_rate: 921600, is_active: false },
    ],
    routes: [
      {
        route_id: 100,
        source_id: 1,
        target_ids: [2, 3, 999],
        bidirectional: true,
        fault: {
          fixed_delay: { secs: 0, nanos: 5_000_000 },
          jitter_mean: { secs: 0, nanos: 0 },
          jitter_std_dev: { secs: 0, nanos: 0 },
          rate_limit_bps: null,
          disconnected: false,
          bit_error_rate: 0.001,
          gilbert_elliott: { p_gb: 0.02, p_bg: 0.1, p_g: 0, p_b: 0.5 },
          corrupt_nth_crc: 10,
        },
      },
      {
        route_id: 101,
        source_id: 999,
        target_ids: [1],
        bidirectional: false,
      },
    ],
  });

  assert.equal(nodes.length, 3);
  assert.equal(nodes[2].data.status, "disconnected");
  // Targets 2 and 3 bidirectional -> 4 directed edges; stale target 999 and orphan route 101 dropped.
  assert.equal(edges.length, 4);
  assert.equal(edges[0].data.routeId, 100);
  assert.equal(edges[0].data.sourceEndpointId, 1);
  assert.equal(edges[0].data.targetEndpointId, 2);
  assert.equal(edges[0].data.faultConfig?.enabled, true);
  assert.equal(edges[0].data.faultConfig?.fixedDelayMs, 5);
  assert.equal(edges[0].data.faultConfig?.corruptNthCrc, 10);
});

test("createRouteEdgeForConnection resolves connected endpoint ids and rejects self-loops", () => {
  const ep4 = createVirtualPairNode(initialNodes);
  const ep5 = createVirtualPairNode([...initialNodes, ep4]);
  const nodes = [...initialNodes, ep4, ep5];

  const valid = createRouteEdgeForConnection({ source: ep4.id, target: ep5.id }, nodes);
  assert.ok(valid);
  assert.equal(valid.data.sourceEndpointId, 4);
  assert.equal(valid.data.targetEndpointId, 5);

  assert.equal(createRouteEdgeForConnection({ source: ep4.id, target: ep4.id }, nodes), null);
  assert.equal(createRouteEdgeForConnection({ source: "ep-999", target: ep4.id }, nodes), null);
});

test("mergeNodePositions keeps dragged positions and drops stale node ids", () => {
  const projected = projectTopologySnapshot({
    endpoints: [
      { id: 1, name: "HOST UART", port_type: "Uart", baud_rate: 115200, is_active: true },
      { id: 2, name: "DUT VIRTUAL PAIR", port_type: "VirtualPair", baud_rate: 115200, is_active: true },
    ],
    routes: [],
  }).nodes;
  const previous = [
    { ...projected[0], position: { x: 500, y: 600 } },
    { ...projected[1], position: { x: 700, y: 620 } },
    {
      ...projected[1],
      id: "ep-9",
      position: { x: 900, y: 900 },
    },
  ];

  const merged = mergeNodePositions(projected, previous);
  assert.equal(merged.length, 2, "stale ep-9 is dropped");
  assert.deepEqual(merged[0].position, { x: 500, y: 600 }, "dragged position kept");
  assert.deepEqual(merged[1].position, { x: 700, y: 620 });
  assert.equal(merged.some((node) => node.id === "ep-9"), false);

  // New ids fall back to their projected slot instead of a dragged one.
  const withNewcomer = mergeNodePositions(
    [
      ...projected,
      {
        ...projected[0],
        id: "ep-3",
        data: { ...projected[0].data, endpointId: 3 },
        position: { x: 80, y: 80 },
      },
    ],
    previous
  );
  assert.equal(withNewcomer.length, 3);
  assert.notDeepEqual(withNewcomer[2].position, { x: 500, y: 600 });
});

test("uiFaultToWireConfig round-trips through wireFaultToUiConfig", () => {
  const wire = uiFaultToWireConfig(ROUTE_FAULT_PRESET);
  assert.deepEqual(
    { secs: wire.fixed_delay.secs, nanos: wire.fixed_delay.nanos },
    { secs: 0, nanos: 5_000_000 }
  );
  assert.equal(wire.bit_error_rate, 1e-4);
  assert.equal(wire.gilbert_elliott?.p_gb, 0.01);
  assert.equal(wire.corrupt_nth_crc, null);

  const ui = wireFaultToUiConfig(wire);
  assert.equal(ui.enabled, true);
  assert.equal(ui.fixedDelayMs, 5);
  assert.equal(ui.gaussianJitterMeanMs, 2);
  assert.equal(ui.gaussianJitterStdDevMs, 1.5);
  assert.equal(ui.bitErrorRate, 1e-4);
  assert.equal(ui.burstErrorProb, 0.01);
  assert.equal(ui.corruptNthCrc, 0);
});

test("demo canonical snapshot projects to the live 3-node / 2-route graph", () => {
  const { nodes, edges } = projectTopologySnapshot(createDemoTopologySnapshot());
  assert.equal(nodes.length, 3);
  assert.deepEqual(
    nodes.map((node) => node.data.label),
    ["HOST UART", "DUT VIRTUAL PAIR", "WIRESHARK EXTCAP"]
  );
  // 1<->2 bidirectional expands to two directed edges, 1->3 to one.
  assert.equal(edges.length, 3);
  const faulty = edges.filter((edge) => edge.data.hasErrors);
  assert.equal(faulty.length, 2, "both projections of the faulty bidirectional route");
  assert.ok(faulty.every((edge) => edge.data.routeId === 100));
  assert.ok(faulty.every((edge) => edge.data.faultConfig?.fixedDelayMs === 5));
  assert.equal(edges.filter((edge) => edge.data.routeId === 101).length, 1);
});
