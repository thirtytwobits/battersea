/**
 * Copyright (c) Scott A Dixon
 *
 * Resolves selection and layout updates for interactive Dataflow edge affordances.
 */
import type {
  FlowStudioEdgeBridge,
  FlowStudioEdgeWaypoint,
  FlowStudioSelectedEdgeBridge,
  FlowStudioSelectedEdgeWaypoint,
} from "./dataflow-editor-state.js";
import {
  insertBridgeIntoEdge,
  moveBridgeInEdge,
  resizeBridgeInEdge,
} from "./flow-edge-bridges.js";
import { type FlowStudioWorkspaceState } from "./flow-persistence.js";
import { insertWaypointIntoEdge } from "./flow-edge-waypoints.js";

export interface DataflowEdgeInteractionResult {
  selectedEdgeBridge: FlowStudioSelectedEdgeBridge | null;
  selectedEdgeWaypoint: FlowStudioSelectedEdgeWaypoint | null;
  workspace: FlowStudioWorkspaceState;
}

export function clearNativeDataflowEdgeSelection(
  workspace: FlowStudioWorkspaceState,
): FlowStudioWorkspaceState {
  const edges = workspace.edges.map((edge) =>
    edge.selected
      ? {
          ...edge,
          selected: false,
        }
      : edge,
  );
  const changed = edges.some((edge, index) => edge !== workspace.edges[index]);

  return changed
    ? {
        ...workspace,
        edges,
      }
    : workspace;
}

export function resolveDataflowEdgeBridgeSelection(
  workspace: FlowStudioWorkspaceState,
  edgeId: string,
  bridgeIndex: number,
): DataflowEdgeInteractionResult {
  const clearedWorkspace = clearNativeDataflowEdgeSelection(workspace);
  const nextWorkspace =
    clearedWorkspace.selectedTarget.kind === "none"
      ? clearedWorkspace
      : {
          ...clearedWorkspace,
          selectedTarget: { kind: "none" } as const,
        };

  return {
    selectedEdgeBridge: {
      bridgeIndex,
      edgeId,
    },
    selectedEdgeWaypoint: null,
    workspace: nextWorkspace,
  };
}

export function resolveDataflowEdgeWaypointSelection(
  workspace: FlowStudioWorkspaceState,
  edgeId: string,
  waypointIndex: number,
): DataflowEdgeInteractionResult {
  const clearedWorkspace = clearNativeDataflowEdgeSelection(workspace);
  const nextWorkspace =
    clearedWorkspace.selectedTarget.kind === "none"
      ? clearedWorkspace
      : {
          ...clearedWorkspace,
          selectedTarget: { kind: "none" } as const,
        };

  return {
    selectedEdgeBridge: null,
    selectedEdgeWaypoint: {
      edgeId,
      waypointIndex,
    },
    workspace: nextWorkspace,
  };
}

export function resolveDataflowEdgeBridgeInsertion(
  workspace: FlowStudioWorkspaceState,
  edgeId: string,
  bridge: FlowStudioEdgeBridge,
): DataflowEdgeInteractionResult {
  const clearedWorkspace = clearNativeDataflowEdgeSelection(workspace);
  const nextWorkspace = {
    ...clearedWorkspace,
    edges: clearedWorkspace.edges.map((edge) =>
      edge.id === edgeId ? insertBridgeIntoEdge(edge, bridge) : edge,
    ),
    selectedTarget: { kind: "none" } as const,
  };
  const bridgeIndex =
    workspace.edges.find((edge) => edge.id === edgeId)?.data?.bridges?.length ??
    0;

  return {
    selectedEdgeBridge: {
      bridgeIndex,
      edgeId,
    },
    selectedEdgeWaypoint: null,
    workspace: nextWorkspace,
  };
}

export function resolveDataflowEdgeWaypointInsertion(
  workspace: FlowStudioWorkspaceState,
  edgeId: string,
  segmentIndex: number,
  segmentT: number,
  waypoint: FlowStudioEdgeWaypoint,
): DataflowEdgeInteractionResult {
  const clearedWorkspace = clearNativeDataflowEdgeSelection(workspace);

  return {
    selectedEdgeBridge: null,
    selectedEdgeWaypoint: {
      edgeId,
      waypointIndex: segmentIndex,
    },
    workspace: {
      ...clearedWorkspace,
      edges: clearedWorkspace.edges.map((edge) =>
        edge.id === edgeId
          ? insertWaypointIntoEdge(edge, segmentIndex, waypoint, segmentT)
          : edge,
      ),
      selectedTarget: { kind: "none" },
    },
  };
}

export function updateDataflowEdgeBridgePosition(
  workspace: FlowStudioWorkspaceState,
  edgeId: string,
  bridgeIndex: number,
  segmentIndex: number,
  t: number,
): FlowStudioWorkspaceState {
  return {
    ...workspace,
    edges: workspace.edges.map((edge) =>
      edge.id === edgeId
        ? moveBridgeInEdge(edge, bridgeIndex, { segmentIndex, t })
        : edge,
    ),
  };
}

export function updateDataflowEdgeBridgeGap(
  workspace: FlowStudioWorkspaceState,
  edgeId: string,
  bridgeIndex: number,
  gap: number,
): FlowStudioWorkspaceState {
  return {
    ...workspace,
    edges: workspace.edges.map((edge) =>
      edge.id === edgeId ? resizeBridgeInEdge(edge, bridgeIndex, gap) : edge,
    ),
  };
}
