import type { Node, Edge } from "@xyflow/react";
import type {
  EndpointNodeData,
  SplitterNodeData,
  MergerNodeData,
  FaultNodeData,
  RouteEdgeData,
} from "../../types/topology.ts";

export type EndpointNodeType = Node<EndpointNodeData, "endpoint">;
export type SplitterNodeType = Node<SplitterNodeData, "splitter">;
export type MergerNodeType = Node<MergerNodeData, "merger">;
export type FaultNodeType = Node<FaultNodeData, "fault">;

export type OmniPortNode =
  | EndpointNodeType
  | SplitterNodeType
  | MergerNodeType
  | FaultNodeType;

export type OmniPortEdge = Edge<RouteEdgeData>;
