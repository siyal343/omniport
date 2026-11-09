/**
 * Topology Graph Types & Telemetry Models
 * Aligned with OmniPort routing matrix (FR-202) and fault injection (FR-301/302).
 */

export type EndpointKind = "uart" | "pty" | "virtual_pair" | "tcp" | "udp";

export interface EndpointNodeData extends Record<string, unknown> {
  label: string;
  kind: EndpointKind;
  portPath: string;
  baudRate: number;
  dataBits: number;
  parity: "none" | "even" | "odd";
  stopBits: 1 | 2;
  status: "connected" | "disconnected" | "error";
  rxBytes: number;
  txBytes: number;
  errorCount: number;
}

export interface SplitterNodeData extends Record<string, unknown> {
  label: string;
  fanoutCount: number;
  queueCapacity: number;
  dropPolicy: "drop_newest" | "drop_oldest";
}

export interface MergerNodeData extends Record<string, unknown> {
  label: string;
  inputsCount: number;
  atomicFraming: boolean; // FR-203: Atomic packet serialization
}

export interface FaultConfig extends Record<string, unknown> {
  enabled: boolean;
  fixedDelayMs: number;
  gaussianJitterMeanMs: number;
  gaussianJitterStdDevMs: number;
  bitErrorRate: number; // e.g. 1e-4
  burstErrorProb: number;
  corruptNthCrc: number; // 0 = disabled, >0 = corrupt every Nth CRC
}

export interface FaultNodeData extends Record<string, unknown> {
  label: string;
  config: FaultConfig;
}

export interface DissectorNodeData extends Record<string, unknown> {
  label: string;
  structName: string;
  cStructDef: string;
  extractedFields: Array<{
    name: string;
    type: string;
    lastValue: number | string;
    unit?: string;
  }>;
}

export interface RouteEdgeData extends Record<string, unknown> {
  sourceEndpointId: number;
  targetEndpointId: number;
  faultConfig?: FaultConfig;
  packetRate: number; // packets/second
  bandwidthBps: number;
  hasErrors: boolean;
  active: boolean;
}

export interface TelemetrySeriesPoint {
  timestampSec: number;
  packetRate: number;
  throughputKbps: number;
  jitterMs: number;
  crcErrorCount: number;
}
