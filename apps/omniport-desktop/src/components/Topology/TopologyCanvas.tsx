import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
  type Connection,
  type EdgeTypes,
  type NodeTypes,
  type OnSelectionChangeParams,
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
  mergeNodePositions,
  projectTopologySnapshot,
  createRouteEdgeForConnection,
  uiFaultToWireConfig,
  nextVirtualPairPortNames,
  ROUTE_FAULT_PRESET,
} from "./topologyData.ts";
import {
  addRoute,
  createVirtualPair,
  fetchTopology,
  removeEndpoint,
  removeRoute,
} from "../../ipc/topologyClient.ts";
import type { CanonicalTopologySnapshot } from "../../types/topology.ts";
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

const FAULT_PRESET_TOOLTIP =
  `Attach HIL fault to the selected route: ${ROUTE_FAULT_PRESET.fixedDelayMs}ms delay, ` +
  `${ROUTE_FAULT_PRESET.gaussianJitterMeanMs}ms mean / ±${ROUTE_FAULT_PRESET.gaussianJitterStdDevMs}ms jitter, ` +
  `BER ${ROUTE_FAULT_PRESET.bitErrorRate.toExponential(1)}, burst ${(ROUTE_FAULT_PRESET.burstErrorProb * 100).toFixed(0)}%. ` +
  "Daemon-owned: removes and re-adds the route with this fault config.";

function errorMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  return String(err);
}

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

/**
 * Live topology view + mutation surface.
 *
 * The canonical `{endpoints, routes}` snapshot is owned by the Tauri host (or
 * the browser demo adapter). Every mutation replaces the graph with the
 * snapshot the daemon returns; there is no frontend-only graph state and no
 * fake fault node. Node drag positions are merged by node id so a mutation
 * refresh never makes the graph jump.
 */
export const TopologyCanvas: React.FC<TopologyCanvasProps> = ({
  onRouteCreated,
  theme = darkTheme,
}) => {
  const [nodes, setNodes, onNodesChange] = useNodesState<OmniPortNode>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<OmniPortEdge>([]);
  const [snapshot, setSnapshot] = useState<CanonicalTopologySnapshot | null>(null);
  const [status, setStatus] = useState<"loading" | "ready" | "error">("loading");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [selectedEdgeIds, setSelectedEdgeIds] = useState<string[]>([]);
  const containerRef = useRef<HTMLDivElement>(null);
  const [containerSize, setContainerSize] = useState({ width: 0, height: 0 });

  const snapshotRef = useRef<CanonicalTopologySnapshot | null>(null);
  const nodesRef = useRef(nodes);
  nodesRef.current = nodes;
  const mutationQueueRef = useRef<Promise<void>>(Promise.resolve());
  const initialLoadStartedRef = useRef(false);

  const pendingEndpointDeletes = useRef(new Set<number>());
  const pendingRouteDeletes = useRef(new Set<number>());
  const deleteFlushScheduled = useRef(false);

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

  /** Adopt a daemon snapshot, keeping user-dragged node positions by id. */
  const applySnapshot = useCallback(
    (next: CanonicalTopologySnapshot, options: { clearError?: boolean } = {}) => {
      snapshotRef.current = next;
      setSnapshot(next);
      const projected = projectTopologySnapshot(next);
      setNodes((previous) => mergeNodePositions(projected.nodes, previous));
      setEdges(projected.edges);
      setStatus("ready");
      if (options.clearError !== false) setError(null);
    },
    [setNodes, setEdges]
  );

  const load = useCallback(async () => {
    if (snapshotRef.current === null) setStatus("loading");
    setError(null);
    try {
      applySnapshot(await fetchTopology());
    } catch (err) {
      setStatus("error");
      setError(errorMessage(err));
    }
  }, [applySnapshot]);

  useEffect(() => {
    if (initialLoadStartedRef.current) return;
    initialLoadStartedRef.current = true;
    void load();
  }, [load]);

  /** Keep the last good graph visible after a failed mutation, then reconcile. */
  const restoreGraph = useCallback(() => {
    const current = snapshotRef.current;
    if (!current) return;
    const projected = projectTopologySnapshot(current);
    setNodes((previous) => mergeNodePositions(projected.nodes, previous));
    setEdges(projected.edges);
  }, [setNodes, setEdges]);

  /**
   * Serializes all mutations so concurrent delete events / clicks cannot
   * interleave snapshots. On failure the previous graph stays and the error is
   * surfaced; a background re-read reconciles if a compound action partially
   * applied daemon-side.
   */
  const runMutation = useCallback(
    (operation: (start: CanonicalTopologySnapshot) => Promise<CanonicalTopologySnapshot>) => {
      const run = async () => {
        const start = snapshotRef.current;
        if (!start) return;
        setBusy(true);
        try {
          applySnapshot(await operation(start));
        } catch (err) {
          setError(errorMessage(err));
          setStatus("ready");
          restoreGraph();
          // Reconcile a possibly partially-applied compound action without
          // clearing the user-facing mutation error.
          fetchTopology()
            .then((next) => applySnapshot(next, { clearError: false }))
            .catch(() => {
              /* keep last good graph + error banner */
            });
        } finally {
          setBusy(false);
        }
      };
      mutationQueueRef.current = mutationQueueRef.current.then(run, run);
      return mutationQueueRef.current;
    },
    [applySnapshot, restoreGraph]
  );

  const scheduleDeleteFlush = useCallback(() => {
    if (deleteFlushScheduled.current) return;
    deleteFlushScheduled.current = true;
    queueMicrotask(() => {
      deleteFlushScheduled.current = false;
      const endpointIds = Array.from(pendingEndpointDeletes.current);
      pendingEndpointDeletes.current.clear();
      const routeIds = Array.from(pendingRouteDeletes.current);
      pendingRouteDeletes.current.clear();
      if (endpointIds.length === 0 && routeIds.length === 0) return;

      void runMutation(async (start) => {
        let latest = start;
        for (const endpointId of endpointIds) {
          latest = await removeEndpoint(endpointId);
        }
        const liveRouteIds = new Set(latest.routes.map((route) => route.route_id));
        for (const routeId of routeIds) {
          // Endpoint deletion already prunes connected routes on the daemon.
          if (!liveRouteIds.has(routeId)) continue;
          latest = await removeRoute(routeId);
          liveRouteIds.delete(routeId);
        }
        return latest;
      });
    });
  }, [runMutation]);

  const onNodesDelete = useCallback(
    (deletedNodes: OmniPortNode[]) => {
      for (const node of deletedNodes) {
        if (node.type === "endpoint" && typeof node.data.endpointId === "number") {
          pendingEndpointDeletes.current.add(node.data.endpointId);
        }
      }
      scheduleDeleteFlush();
    },
    [scheduleDeleteFlush]
  );

  const onEdgesDelete = useCallback(
    (deletedEdges: OmniPortEdge[]) => {
      for (const edge of deletedEdges) {
        const routeId = edge.data?.routeId;
        if (typeof routeId === "number") pendingRouteDeletes.current.add(routeId);
      }
      scheduleDeleteFlush();
    },
    [scheduleDeleteFlush]
  );

  const onConnect = useCallback(
    (connection: Connection) => {
      const routeEdge = createRouteEdgeForConnection(connection, nodesRef.current);
      const routeData = routeEdge?.data;
      if (
        !routeEdge ||
        !routeData ||
        routeData.sourceEndpointId <= 0 ||
        routeData.targetEndpointId <= 0
      ) {
        setError("Cannot create a route between the selected handles.");
        return;
      }
      const sourceId = routeData.sourceEndpointId;
      const targetId = routeData.targetEndpointId;
      void runMutation(() =>
        addRoute({ sourceId, targetIds: [targetId], bidirectional: false })
      );
      onRouteCreated?.(connection.source ?? "", connection.target ?? "");
    },
    [runMutation, onRouteCreated]
  );

  const onSelectionChange = useCallback((params: OnSelectionChangeParams) => {
    setSelectedEdgeIds(params.edges.map((edge) => edge.id));
  }, []);

  const selectedRoute = useMemo(() => {
    if (!snapshot || selectedEdgeIds.length !== 1) return null;
    const selectedEdge = edges.find((edge) => edge.id === selectedEdgeIds[0]);
    const routeId = selectedEdge?.data?.routeId;
    if (typeof routeId !== "number") return null;
    return snapshot.routes.find((route) => route.route_id === routeId) ?? null;
  }, [snapshot, selectedEdgeIds, edges]);

  const addVirtualPair = useCallback(() => {
    void runMutation(async (start) => {
      const ports = nextVirtualPairPortNames(start);
      return createVirtualPair({
        ...ports,
        baudRate: 115200,
        dataBits: 8,
        parity: 0,
        stopBits: 1,
      });
    });
  }, [runMutation]);

  const attachFault = useCallback(() => {
    if (!selectedRoute) return;
    const routeId = selectedRoute.route_id;
    const fault = uiFaultToWireConfig(ROUTE_FAULT_PRESET);
    void runMutation(async (start) => {
      const route = start.routes.find((candidate) => candidate.route_id === routeId);
      if (!route) throw new Error(`Route ${routeId} no longer exists`);
      // No update_route RPC: the daemon owns the change as remove + add.
      await removeRoute(route.route_id);
      return addRoute({
        sourceId: route.source_id,
        targetIds: route.target_ids,
        bidirectional: route.bidirectional,
        fault,
      });
    });
  }, [runMutation, selectedRoute]);

  const isLight = theme === lightTheme;
  // Decorative minimap only where it cannot cover nodes (full-tab view);
  // the 280px split panel always hides it.
  const showMinimap = containerSize.width >= 1000 && containerSize.height >= 500;
  // The floating "add node" panel would cover fitted nodes in the short
  // split-workbench strip; the Visual Topology tab keeps it available.
  const showAddPanel = containerSize.height >= 240;
  const selectedRouteCount = selectedRoute ? 1 : 0;

  const graphStyle = {
    width: "100%",
    height: "100%",
    position: "relative",
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
        onNodesDelete={onNodesDelete}
        onEdgesDelete={onEdgesDelete}
        onSelectionChange={onSelectionChange}
        onConnect={onConnect}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        fitView
        fitViewOptions={{ padding: 0.18 }}
        minZoom={0.15}
        maxZoom={1.1}
        deleteKeyCode={["Backspace", "Delete"]}
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
              data-testid="topology-add-pair"
              onClick={addVirtualPair}
              disabled={busy || status !== "ready"}
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
              data-testid="topology-attach-fault"
              onClick={attachFault}
              disabled={busy || selectedRouteCount !== 1}
              title={FAULT_PRESET_TOOLTIP}
              style={{
                padding: "5px 10px",
                backgroundColor: theme.nodeHeaderBackground,
                border: `1px solid ${selectedRouteCount === 1 ? "#d29922" : theme.borderColor}`,
                borderRadius: 6,
                color: selectedRouteCount === 1 ? "#d29922" : theme.headerText,
                fontSize: 11,
                fontWeight: 600,
              }}
            >
              ⚡ Attach Fault
            </button>
          </Panel>
        )}
      </ReactFlow>

      {status === "loading" && snapshot === null && (
        <div className="op-topology-status" data-testid="topology-status">
          Loading live topology…
        </div>
      )}
      {error !== null && (
        <div className="op-topology-error" data-testid="topology-error" role="alert">
          <span className="op-topology-error-text">{error}</span>
          <button type="button" className="op-btn" onClick={() => void load()}>
            Retry
          </button>
        </div>
      )}
      {busy && (
        <div className="op-topology-busy" data-testid="topology-busy">
          SYNCING…
        </div>
      )}
    </div>
  );
};
