import React from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import type { FaultNodeType } from "../topologyTypes.ts";

export const FaultNode: React.FC<NodeProps<FaultNodeType>> = ({ data }) => {
  const cfg = data.config;
  const isEnabled = cfg.enabled;

  return (
    <div
      style={{
        width: 200,
        backgroundColor: "var(--tp-node-bg, #161b22)",
        border: `1px solid ${isEnabled ? "#f85149" : "var(--tp-border, #30363d)"}`,
        borderRadius: 8,
        color: "var(--tp-text, #e6edf3)",
        fontSize: 11,
        fontFamily: 'SFMono-Regular, Consolas, "Liberation Mono", Menlo, monospace',
        boxShadow: isEnabled ? "0 0 12px rgba(248, 81, 73, 0.2)" : "0 4px 12px rgba(0,0,0,0.3)",
      }}
    >
      <Handle
        type="target"
        position={Position.Left}
        style={{ width: 10, height: 10, background: "#f85149", border: "2px solid #0d1117" }}
      />

      <div
        style={{
          padding: "6px 10px",
          backgroundColor: isEnabled ? "rgba(248, 81, 73, 0.15)" : "var(--tp-node-header, #21262d)",
          borderTopLeftRadius: 7,
          borderTopRightRadius: 7,
          borderBottom: "1px solid #30363d",
          fontWeight: 600,
          color: isEnabled ? "#f85149" : "var(--tp-muted, #8b949e)",
          display: "flex",
          justifyContent: "space-between",
        }}
      >
        <span>⚡ {data.label}</span>
        <span style={{ fontSize: 9 }}>{isEnabled ? "ACTIVE" : "BYPASS"}</span>
      </div>

      <div style={{ padding: "8px 10px", display: "flex", flexDirection: "column", gap: 3 }}>
        <div style={{ display: "flex", justifyContent: "space-between", color: "var(--tp-muted, #8b949e)" }}>
          <span>Delay / Jitter:</span>
          <span style={{ color: "var(--tp-text, #e6edf3)" }}>
            {cfg.fixedDelayMs}ms ± {cfg.gaussianJitterStdDevMs}ms
          </span>
        </div>
        <div style={{ display: "flex", justifyContent: "space-between", color: "var(--tp-muted, #8b949e)" }}>
          <span>Bit Error Rate:</span>
          <span style={{ color: cfg.bitErrorRate > 0 ? "#f85149" : "var(--tp-muted, #8b949e)" }}>
            {cfg.bitErrorRate > 0 ? cfg.bitErrorRate.toExponential(1) : "0"}
          </span>
        </div>
        <div style={{ display: "flex", justifyContent: "space-between", color: "var(--tp-muted, #8b949e)" }}>
          <span>CRC Corrupt:</span>
          <span style={{ color: cfg.corruptNthCrc > 0 ? "#f85149" : "var(--tp-muted, #8b949e)" }}>
            {cfg.corruptNthCrc > 0 ? `1 in ${cfg.corruptNthCrc}` : "OFF"}
          </span>
        </div>
      </div>

      <Handle
        type="source"
        position={Position.Right}
        style={{ width: 10, height: 10, background: "#f85149", border: "2px solid #0d1117" }}
      />
    </div>
  );
};
