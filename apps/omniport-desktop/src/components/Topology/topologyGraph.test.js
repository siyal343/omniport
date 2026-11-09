import test from "node:test";
import assert from "node:assert/strict";

test("Topology initial nodes conform to OmniPort routing traits", () => {
  const node = {
    id: "ep-1",
    type: "endpoint",
    position: { x: 0, y: 0 },
    data: {
      label: "COM1 Host",
      kind: "uart",
      portPath: "COM1",
      baudRate: 115200,
      dataBits: 8,
      parity: "none",
      stopBits: 1,
      status: "connected",
      rxBytes: 1024,
      txBytes: 2048,
      errorCount: 0,
    },
  };

  assert.equal(node.type, "endpoint");
  assert.equal(node.data.baudRate, 115200);
  assert.equal(node.data.status, "connected");
});

test("FaultNode configuration holds required jitter and BER parameters", () => {
  const faultNode = {
    id: "fault-1",
    type: "fault",
    position: { x: 100, y: 100 },
    data: {
      label: "Jitter Node",
      config: {
        enabled: true,
        fixedDelayMs: 10,
        gaussianJitterMeanMs: 2,
        gaussianJitterStdDevMs: 1.5,
        bitErrorRate: 1e-4,
        burstErrorProb: 0.05,
        corruptNthCrc: 50,
      },
    },
  };

  assert.equal(faultNode.type, "fault");
  assert.equal(faultNode.data.config.fixedDelayMs, 10);
  assert.equal(faultNode.data.config.bitErrorRate, 0.0001);
  assert.equal(faultNode.data.config.corruptNthCrc, 50);
});

test("Edge creation validates source and target endpoints", () => {
  const edge = {
    id: "e-1-2",
    source: "ep-1",
    target: "ep-2",
    type: "animated",
    data: {
      sourceEndpointId: 1,
      targetEndpointId: 2,
      packetRate: 150,
      bandwidthBps: 120000,
      hasErrors: false,
      active: true,
    },
  };

  assert.equal(edge.source, "ep-1");
  assert.equal(edge.target, "ep-2");
  assert.equal(edge.data.packetRate, 150);
  assert.equal(edge.data.active, true);
});
