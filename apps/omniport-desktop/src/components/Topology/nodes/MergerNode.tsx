import React from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import type { MergerNodeType } from "../topologyTypes.ts";

export const MergerNode: React.FC<NodeProps<MergerNodeType>> = ({ data }) => {
  return (
    <div
      style={{
        width: 180,
        backgroundColor: "#161b22",
        border: "1px solid #30363d",
        borderRadius: 8,
        color: "#e6edf3",
        fontSize: 11,
        fontFamily: 'SFMono-Regular, Consolas, "Liberation Mono", Menlo, monospace',
        boxShadow: "0 4px 12px rgba(0,0,0,0.3)",
      }}
    >
      <Handle
        type="target"
        position={Position.Left}
        style={{ width: 10, height: 10, background: "#bc8cff", border: "2px solid #0d1117" }}
      />

      <div
        style={{
          padding: "6px 10px",
          backgroundColor: "#1f242c",
          borderTopLeftRadius: 7,
          borderTopRightRadius: 7,
          borderBottom: "1px solid #30363d",
          fontWeight: 600,
          color: "#bc8cff",
          display: "flex",
          justifyContent: "space-between",
        }}
      >
        <span>{data.label}</span>
        <span style={{ fontSize: 9, color: "#8b949e" }}>{data.inputsCount}:1 MERGE</span>
      </div>

      <div style={{ padding: "8px 10px", display: "flex", flexDirection: "column", gap: 4 }}>
        <div style={{ display: "flex", justifyContent: "space-between", color: "#8b949e" }}>
          <span>Atomic Framing:</span>
          <span style={{ color: data.atomicFraming ? "#3fb950" : "#f85149" }}>
            {data.atomicFraming ? "ENABLED" : "OFF"}
          </span>
        </div>
      </div>

      <Handle
        type="source"
        position={Position.Right}
        style={{ width: 10, height: 10, background: "#bc8cff", border: "2px solid #0d1117" }}
      />
    </div>
  );
};
