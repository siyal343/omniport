import React, { useCallback, useEffect, useRef, useState } from "react";
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  Panel,
  useNodesState,
  useEdgesState,
  useNodesInitialized,
  useReactFlow,
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
import {
  initialNodes,
  initialEdges,
  createVirtualPairNode,
  createFaultNode,
  createRouteEdgeForConnection,
} from "./topologyData.ts";
import { type MonitorTheme, darkTheme, lightTheme } from "../../utils/theme.ts";

const nodeTypes: NodeTypes = {
  endpoint: EndpointNode,
  splitter: SplitterNode,
  merger: MergerNode,
  fault: FaultNode,
};

const edgeTypes: EdgeTypes = {
  animated: AnimatedRouteEdge,
};

const FIT_OPTIONS = {
  padding: 0.18,
  duration: 0,
  minZoom: 0.15,
  maxZoom: 1.1,
} as const;

/**
 * Fits the graph into its own container after node dimensions are measured and
 * whenever the container resizes. Without this the split-workbench panel (which
 * mounts at ~960x280, much smaller than the full-tab view) kept the initial
 * zoom and clipped the right-hand endpoint nodes behind the telemetry panel.
 */
const FitViewController: React.FC<{ width: number; height: number; nodeCount: number }> = ({
  width,
  height,
  nodeCount,
}) => {
  const flow = useReactFlow();
  const nodesInitialized = useNodesInitialized();

  useEffect(() => {
    if (!nodesInitialized || width <= 0 || height <= 0) return;
    const raf = requestAnimationFrame(() => {
      flow.fitView(FIT_OPTIONS);
    });
    return () => cancelAnimationFrame(raf);
  }, [flow, nodesInitialized, width, height, nodeCount]);

  return null;
};

export interface TopologyCanvasProps {
  onRouteCreated?: (sourceId: string, targetId: string) => void;
  /** Theme tokens so the graph follows the workbench dark/light toggle. */
  theme?: MonitorTheme;
}

export const TopologyCanvas: React.FC<TopologyCanvasProps> = ({
  onRouteCreated,
  theme = darkTheme,
}) => {
  const [nodes, setNodes, onNodesChange] = useNodesState<OmniPortNode>(initialNodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState<OmniPortEdge>(initialEdges);
  const containerRef = useRef<HTMLDivElement>(null);
  const [containerSize, setContainerSize] = useState({ width: 0, height: 0 });

  // Track container size: drives fit-on-resize and minimap visibility.
  useEffect(() => {
    const element = containerRef.current;
    if (!element) return;
    let raf = 0;
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (!entry) return;
      const width = Math.floor(entry.contentRect.width);
      const height = Math.floor(entry.contentRect.height);
      if (width <= 0 || height <= 0) return;
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        setContainerSize((prev) =>
          prev.width === width && prev.height === height ? prev : { width, height }
        );
      });
    });
    observer.observe(element);
    return () => {
      observer.disconnect();
      cancelAnimationFrame(raf);
    };
  }, []);

  const onConnect = useCallback(
    (connection: Connection) => {
      const routeEdge = createRouteEdgeForConnection(connection, nodes);
      if (!routeEdge) return;
      setEdges((eds) =>
        addEdge(
          {
            ...connection,
            ...routeEdge,
          },
          eds
        )
      );
      if (connection.source && connection.target) {
        onRouteCreated?.(connection.source, connection.target);
      }
    },
    [nodes, setEdges, onRouteCreated]
  );

  const addVirtualPair = useCallback(() => {
    setNodes((current) => [...current, createVirtualPairNode(current)]);
  }, [setNodes]);

  const addFaultInjector = useCallback(() => {
    setNodes((current) => [...current, createFaultNode(current)]);
  }, [setNodes]);

  const isLight = theme === lightTheme;
  // Decorative minimap only where it cannot cover nodes (full-tab view);
  // the 280px split panel always hides it.
  const showMinimap = containerSize.width >= 1000 && containerSize.height >= 500;
  // The floating "add node" panel would cover fitted nodes in the short
  // split-workbench strip; the Visual Topology tab keeps it available.
  const showAddPanel = containerSize.height >= 240;

  const graphStyle = {
    width: "100%",
    height: "100%",
    backgroundColor: theme.background,
    "--tp-bg": theme.background,
    "--tp-node-bg": theme.nodeBackground,
    "--tp-node-header": theme.nodeHeaderBackground,
    "--tp-border": theme.borderColor,
    "--tp-text": theme.asciiText,
    "--tp-muted": theme.headerText,
    "--tp-shadow": isLight
      ? "0 1px 3px rgba(31, 35, 40, 0.14)"
      : "0 4px 12px rgba(0, 0, 0, 0.3)",
  } as React.CSSProperties;

  return (
    <div ref={containerRef} data-testid="topology-canvas" style={graphStyle}>
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
        fitViewOptions={{ padding: 0.18 }}
        minZoom={0.15}
        maxZoom={1.1}
      >
        <FitViewController
          width={containerSize.width}
          height={containerSize.height}
          nodeCount={nodes.length}
        />
        <Background variant={BackgroundVariant.Dots} gap={16} size={1} color={theme.borderColor} />
        <Controls />
        {showMinimap && (
          <MiniMap
            nodeColor={theme.borderColor}
            maskColor={isLight ? "rgba(246, 248, 250, 0.75)" : "rgba(13, 17, 23, 0.75)"}
            pannable={false}
            zoomable={false}
            aria-hidden
          />
        )}

        {/* Top Control Panel */}
        {showAddPanel && (
          <Panel position="top-right" style={{ display: "flex", gap: 8 }}>
            <button
              type="button"
              className="op-btn"
              onClick={addVirtualPair}
              style={{
                padding: "5px 10px",
                backgroundColor: theme.nodeHeaderBackground,
                border: `1px solid ${theme.borderColor}`,
                borderRadius: 6,
                color: "#58a6ff",
                fontSize: 11,
                fontWeight: 600,
              }}
            >
              + Add Port Endpoint
            </button>
            <button
              type="button"
              className="op-btn"
              onClick={addFaultInjector}
              style={{
                padding: "5px 10px",
                backgroundColor: theme.nodeHeaderBackground,
                border: `1px solid ${theme.borderColor}`,
                borderRadius: 6,
                color: "#f85149",
                fontSize: 11,
                fontWeight: 600,
              }}
            >
              + Add Fault Injector
            </button>
          </Panel>
        )}
      </ReactFlow>
    </div>
  );
};
