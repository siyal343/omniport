import test from "node:test";
import assert from "node:assert/strict";
import {
  initialNodes,
  initialEdges,
  nextNodeId,
  createVirtualPairNode,
  createFaultNode,
} from "./topologyData.ts";

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
