import React from "react";
import {
  BaseEdge,
  EdgeLabelRenderer,
  getBezierPath,
  type EdgeProps,
} from "@xyflow/react";
import type { OmniPortEdge } from "../topologyTypes.ts";

export const AnimatedRouteEdge: React.FC<EdgeProps<OmniPortEdge>> = ({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  data,
  markerEnd,
}) => {
  const [edgePath, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    sourcePosition,
    targetX,
    targetY,
    targetPosition,
  });

  const isActive = data?.active ?? true;
  const hasErrors = data?.hasErrors ?? false;
  const strokeColor = hasErrors ? "#f85149" : isActive ? "#58a6ff" : "#484f58";

  return (
    <>
      <BaseEdge
        id={id}
        path={edgePath}
        markerEnd={markerEnd}
        style={{
          stroke: strokeColor,
          strokeWidth: 2,
          strokeDasharray: isActive ? "5 5" : undefined,
          animation: isActive ? "dashdraw 0.5s linear infinite" : undefined,
        }}
      />
      {data && (
        <EdgeLabelRenderer>
          <div
            style={{
              position: "absolute",
              transform: `translate(-50%, -50%) translate(${labelX}px,${labelY}px)`,
              pointerEvents: "all",
              backgroundColor: "var(--tp-node-bg, #161b22)",
              border: `1px solid ${hasErrors ? "#f85149" : "var(--tp-border, #30363d)"}`,
              borderRadius: 4,
              padding: "2px 6px",
              fontSize: 10,
              fontFamily: 'SFMono-Regular, Consolas, "Liberation Mono", Menlo, monospace',
              color: hasErrors ? "#f85149" : "var(--tp-muted, #8b949e)",
              boxShadow: "0 2px 6px rgba(0,0,0,0.4)",
            }}
          >
            <span>{data.packetRate} pkt/s</span>
            {data.faultConfig?.enabled && (
              <span style={{ marginLeft: 4, color: "#d29922" }}>[FAULT]</span>
            )}
          </div>
        </EdgeLabelRenderer>
      )}
    </>
  );
};
