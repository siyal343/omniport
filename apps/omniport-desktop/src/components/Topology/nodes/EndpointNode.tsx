import React from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import type { EndpointNodeType } from "../topologyTypes.ts";

export const EndpointNode: React.FC<NodeProps<EndpointNodeType>> = ({ data }) => {
  const isConnected = data.status === "connected";
  const statusColor = isConnected ? "#3fb950" : data.status === "error" ? "#f85149" : "#8b949e";

  return (
    <div
      style={{
        width: 220,
        backgroundColor: "#161b22",
        border: "1px solid #30363d",
        borderRadius: 8,
        color: "#e6edf3",
        fontSize: 11,
        fontFamily: 'SFMono-Regular, Consolas, "Liberation Mono", Menlo, monospace',
        boxShadow: "0 4px 12px rgba(0,0,0,0.3)",
      }}
    >
      {/* Target handle for receiving packets */}
      <Handle
        type="target"
        position={Position.Left}
        style={{ width: 10, height: 10, background: "#58a6ff", border: "2px solid #0d1117" }}
      />

      {/* Header */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          padding: "6px 10px",
          backgroundColor: "#21262d",
          borderTopLeftRadius: 7,
          borderTopRightRadius: 7,
          borderBottom: "1px solid #30363d",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 6, fontWeight: 600 }}>
          <span
            style={{
              width: 8,
              height: 8,
              borderRadius: "50%",
              backgroundColor: statusColor,
              display: "inline-block",
            }}
          />
          <span>{data.label}</span>
        </div>
        <span style={{ fontSize: 9, color: "#8b949e", textTransform: "uppercase" }}>
          {data.kind}
        </span>
      </div>

      {/* Body */}
      <div style={{ padding: "8px 10px", display: "flex", flexDirection: "column", gap: 4 }}>
        <div style={{ display: "flex", justifyContent: "space-between", color: "#8b949e" }}>
          <span>Path:</span>
          <span style={{ color: "#e6edf3" }}>{data.portPath}</span>
        </div>
        <div style={{ display: "flex", justifyContent: "space-between", color: "#8b949e" }}>
          <span>Config:</span>
          <span style={{ color: "#58a6ff" }}>
            {data.baudRate} {data.dataBits}{data.parity[0].toUpperCase()}{data.stopBits}
          </span>
        </div>
        <div
          style={{
            marginTop: 4,
            paddingTop: 4,
            borderTop: "1px dashed #30363d",
            display: "flex",
            justifyContent: "space-between",
            fontSize: 10,
          }}
        >
          <span style={{ color: "#3fb950" }}>RX: {data.rxBytes} B</span>
          <span style={{ color: "#58a6ff" }}>TX: {data.txBytes} B</span>
        </div>
        {data.errorCount > 0 && (
          <div style={{ color: "#f85149", fontWeight: 600, textAlign: "right", fontSize: 10 }}>
            ! {data.errorCount} Errors
          </div>
        )}
      </div>

      {/* Source handle for forwarding packets */}
      <Handle
        type="source"
        position={Position.Right}
        style={{ width: 10, height: 10, background: "#3fb950", border: "2px solid #0d1117" }}
      />
    </div>
  );
};
