/** Copyright (c) Scott A Dixon */
import type { Edge, Node } from "@xyflow/react";
import type { FlowEdge as WireFlowEdge } from "@battersea/flow";
import type {
  AuthoringEditableEdgeData,
  AuthoringGraphBridge,
  AuthoringGraphPoint,
  AuthoringGraphWaypoint,
} from "../graph.js";
import type { FlowStudioNodeData } from "./flow-drag.js";
import type { FlowPortSide } from "./flow-node-ports.js";

export interface NodeContextMenuState {
  nodeId: string;
  x: number;
  y: number;
}

export interface PendingNodeDropState {
  status: "idle" | "pending" | "failed" | "succeeded";
  title: string;
}

export type FlowStudioPoint = AuthoringGraphPoint;

export type FlowStudioEdgeWaypoint = AuthoringGraphWaypoint;

export type FlowStudioEdgeBridge = AuthoringGraphBridge;

export interface FlowStudioSelectedEdgeWaypoint {
  edgeId: string;
  waypointIndex: number;
}

export interface FlowStudioSelectedEdgeBridge {
  bridgeIndex: number;
  edgeId: string;
}

export type FlowStudioEdgeData = AuthoringEditableEdgeData<{
  edgeClassName?: string;
  kind: WireFlowEdge["kind"];
  order: number;
  sourceHandleIndex?: number;
  sourceSideCount?: number;
  targetHandleIndex?: number;
  targetSideCount?: number;
  tokenType?: string;
}>;

export interface FlowStudioNodeSelectionTarget {
  kind: "node";
  nodeId: string;
}

export interface FlowStudioNoSelectionTarget {
  kind: "none";
}

export interface FlowStudioFlowSelectionTarget {
  kind: "flow";
}

export interface FlowStudioPortSelectionTarget {
  kind: "port";
  nodeId: string;
  portId: string;
  side: FlowPortSide;
}

export type FlowStudioSelectionTarget =
  | FlowStudioNoSelectionTarget
  | FlowStudioFlowSelectionTarget
  | FlowStudioNodeSelectionTarget
  | FlowStudioPortSelectionTarget;

export type FlowStudioNode = Node<FlowStudioNodeData>;
export type FlowStudioEdge = Edge<FlowStudioEdgeData>;
