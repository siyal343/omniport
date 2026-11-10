import React from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import type { SplitterNodeType } from "../topologyTypes.ts";

export const SplitterNode: React.FC<NodeProps<SplitterNodeType>> = ({ data }) => {
  return (
    <div
      style={{
        width: 180,
        backgroundColor: "var(--tp-node-bg, #161b22)",
        border: "1px solid #30363d",
        borderRadius: 8,
        color: "var(--tp-text, #e6edf3)",
        fontSize: 11,
        fontFamily: 'SFMono-Regular, Consolas, "Liberation Mono", Menlo, monospace',
        boxShadow: "var(--tp-shadow, 0 4px 12px rgba(0,0,0,0.3))",
      }}
    >
      <Handle
        type="target"
        position={Position.Left}
        style={{ width: 10, height: 10, background: "#58a6ff", border: "2px solid #0d1117" }}
      />

      <div
        style={{
          padding: "6px 10px",
          backgroundColor: "var(--tp-node-header, #21262d)",
          borderTopLeftRadius: 7,
          borderTopRightRadius: 7,
          borderBottom: "1px solid #30363d",
          fontWeight: 600,
          color: "#d29922",
          display: "flex",
          justifyContent: "space-between",
        }}
      >
        <span>{data.label}</span>
        <span style={{ fontSize: 9, color: "var(--tp-muted, #8b949e)" }}>1:{data.fanoutCount} SPLIT</span>
      </div>

      <div style={{ padding: "8px 10px", display: "flex", flexDirection: "column", gap: 4 }}>
        <div style={{ display: "flex", justifyContent: "space-between", color: "var(--tp-muted, #8b949e)" }}>
          <span>Queue:</span>
          <span style={{ color: "var(--tp-text, #e6edf3)" }}>{data.queueCapacity} pkts</span>
        </div>
        <div style={{ display: "flex", justifyContent: "space-between", color: "var(--tp-muted, #8b949e)" }}>
          <span>Policy:</span>
          <span style={{ color: "#d29922" }}>{data.dropPolicy}</span>
        </div>
      </div>

      <Handle
        type="source"
        position={Position.Right}
        style={{ width: 10, height: 10, background: "#d29922", border: "2px solid #0d1117" }}
      />
    </div>
  );
};
