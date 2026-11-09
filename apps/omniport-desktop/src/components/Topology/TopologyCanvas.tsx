import React, { useState, useCallback } from "react";
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  Panel,
  useNodesState,
  useEdgesState,
  addEdge,
  type Connection,
  type EdgeTypes,
  type NodeTypes,
  BackgroundVariant,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";

import { EndpointNode } from "./nodes/EndpointNode.tsx";
import { SplitterNode } from "./nodes/SplitterNode.tsx";
import { MergerNode } from "./nodes/MergerNode.tsx";
import { FaultNode } from "./nodes/FaultNode.tsx";
import { AnimatedRouteEdge } from "./edges/AnimatedRouteEdge.tsx";
import type { OmniPortNode, OmniPortEdge } from "./topologyTypes.ts";

const nodeTypes: NodeTypes = {
  endpoint: EndpointNode,
  splitter: SplitterNode,
  merger: MergerNode,
  fault: FaultNode,
};

const edgeTypes: EdgeTypes = {
  animated: AnimatedRouteEdge,
};

// Default initial routing topology
const initialNodes: OmniPortNode[] = [
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

const initialEdges: OmniPortEdge[] = [
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

export interface TopologyCanvasProps {
  onRouteCreated?: (sourceId: string, targetId: string) => void;
}

export const TopologyCanvas: React.FC<TopologyCanvasProps> = ({ onRouteCreated }) => {
  const [nodes, setNodes, onNodesChange] = useNodesState<OmniPortNode>(initialNodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState<OmniPortEdge>(initialEdges);

  const onConnect = useCallback(
    (connection: Connection) => {
      setEdges((eds) =>
        addEdge(
          {
            ...connection,
            type: "animated",
            data: {
              sourceEndpointId: 1,
              targetEndpointId: 2,
              packetRate: 0,
              bandwidthBps: 0,
              hasErrors: false,
              active: true,
            },
          },
          eds
        )
      );
      if (connection.source && connection.target) {
        onRouteCreated?.(connection.source, connection.target);
      }
    },
    [setEdges, onRouteCreated]
  );

  const addVirtualPair = () => {
    const id = `ep-${nodes.length + 1}`;
    const newNode: OmniPortNode = {
      id,
      type: "endpoint",
      position: { x: 100 + Math.random() * 200, y: 150 + Math.random() * 150 },
      data: {
        label: `VIRTUAL COM${nodes.length + 4}`,
        kind: "virtual_pair",
        portPath: `COM${nodes.length + 4}`,
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
    setNodes((nds) => [...nds, newNode]);
  };

  const addFaultNode = () => {
    const id = `fault-${nodes.length + 1}`;
    const newNode: OmniPortNode = {
      id,
      type: "fault",
      position: { x: 450 + Math.random() * 100, y: 150 + Math.random() * 100 },
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
    setNodes((nds) => [...nds, newNode]);
  };

  return (
    <div style={{ width: "100%", height: "100%", backgroundColor: "#0d1117" }}>
      <style>{`
        @keyframes dashdraw {
          from { stroke-dashoffset: 20; }
          to { stroke-dashoffset: 0; }
        }
      `}</style>
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onConnect={onConnect}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        fitView
      >
        <Background variant={BackgroundVariant.Dots} gap={16} size={1} color="#21262d" />
        <Controls
          style={{
            backgroundColor: "#161b22",
            border: "1px solid #30363d",
            borderRadius: 6,
            color: "#e6edf3",
          }}
        />
        <MiniMap
          nodeColor="#30363d"
          maskColor="rgba(13, 17, 23, 0.75)"
          style={{
            backgroundColor: "#161b22",
            border: "1px solid #30363d",
            borderRadius: 6,
          }}
        />

        {/* Top Control Panel */}
        <Panel position="top-right" style={{ display: "flex", gap: 8 }}>
          <button
            onClick={addVirtualPair}
            style={{
              padding: "5px 10px",
              backgroundColor: "#21262d",
              border: "1px solid #30363d",
              borderRadius: 6,
              color: "#58a6ff",
              fontSize: 11,
              fontWeight: 600,
              cursor: "pointer",
            }}
          >
            + Add Port Endpoint
          </button>
          <button
            onClick={addFaultNode}
            style={{
              padding: "5px 10px",
              backgroundColor: "#21262d",
              border: "1px solid #30363d",
              borderRadius: 6,
              color: "#f85149",
              fontSize: 11,
              fontWeight: 600,
              cursor: "pointer",
            }}
          >
            + Add Fault Injector
          </button>
        </Panel>
      </ReactFlow>
    </div>
  );
};
