/**
 * Copyright (c) Scott A Dixon
 *
 * Builds and mutates manual edge bridge geometry for flow-studio edges.
 */
import type {
  AuthoringGraphBridgeGapInterval,
  AuthoringGraphBridgeLocation,
  AuthoringGraphInsertionInterval,
  ResolvedAuthoringGraphBridgeGeometry,
} from "../graph.js";
import {
  buildEdgeInsertionIntervals as buildSharedEdgeInsertionIntervals,
  buildVisibleEdgePathWithBridges as buildSharedVisibleEdgePathWithBridges,
  createBridgeForInterval as createSharedBridgeForInterval,
  createBridgeForSegment as createSharedBridgeForSegment,
  insertBridgeIntoLayout,
  moveBridgeInLayout,
  removeBridgeFromLayout,
  resolveBridgeGapFromHandle as resolveSharedBridgeGapFromHandle,
  resolveBridgeGapIntervals as resolveSharedBridgeGapIntervals,
  resolveBridgeGeometry as resolveSharedBridgeGeometry,
  resolveInsertionIntervalMidpoint as resolveSharedInsertionIntervalMidpoint,
  resolveNearestBridgeLocation as resolveSharedNearestBridgeLocation,
  resolveNearestInsertionInterval as resolveSharedNearestInsertionInterval,
  resizeBridgeInLayout,
  updateBridgeIndicesForWaypointInsertion,
  updateBridgeIndicesForWaypointRemoval,
} from "../graph.js";

import type {
  FlowStudioEdge,
  FlowStudioEdgeData,
  FlowStudioEdgeBridge,
  FlowStudioPoint,
} from "./dataflow-editor-state.js";
import type { FlowEdgeWaypointSegment } from "./flow-edge-waypoints.js";

export type FlowEdgeBridgeLocation = AuthoringGraphBridgeLocation;
export type FlowEdgeInsertionInterval = AuthoringGraphInsertionInterval;
export type FlowEdgeBridgeGapInterval = AuthoringGraphBridgeGapInterval;
export type ResolvedFlowEdgeBridgeGeometry =
  ResolvedAuthoringGraphBridgeGeometry;

export {
  updateBridgeIndicesForWaypointInsertion,
  updateBridgeIndicesForWaypointRemoval,
};

export function createBridgeForSegment(
  segmentIndex: number,
): FlowStudioEdgeBridge {
  return createSharedBridgeForSegment(segmentIndex);
}

export function createBridgeForInterval(
  interval: FlowEdgeInsertionInterval,
): FlowStudioEdgeBridge {
  return createSharedBridgeForInterval(interval);
}

export function insertBridgeIntoEdge(
  edge: FlowStudioEdge,
  bridge: FlowStudioEdgeBridge,
): FlowStudioEdge {
  return {
    ...edge,
    data: {
      ...resolveEdgeData(edge),
      ...insertBridgeIntoLayout(edge.data, bridge),
    },
  };
}

export function moveBridgeInEdge(
  edge: FlowStudioEdge,
  bridgeIndex: number,
  location: FlowEdgeBridgeLocation,
): FlowStudioEdge {
  return {
    ...edge,
    data: {
      ...resolveEdgeData(edge),
      ...moveBridgeInLayout(edge.data, bridgeIndex, location),
    },
  };
}

export function resizeBridgeInEdge(
  edge: FlowStudioEdge,
  bridgeIndex: number,
  gap: number,
): FlowStudioEdge {
  return {
    ...edge,
    data: {
      ...resolveEdgeData(edge),
      ...resizeBridgeInLayout(edge.data, bridgeIndex, gap),
    },
  };
}

export function removeBridgeFromEdge(
  edge: FlowStudioEdge,
  bridgeIndex: number,
): FlowStudioEdge {
  return {
    ...edge,
    data: {
      ...resolveEdgeData(edge),
      ...removeBridgeFromLayout(edge.data, bridgeIndex),
    },
  };
}

export function resolveNearestBridgeLocation(
  segments: readonly FlowEdgeWaypointSegment[],
  point: FlowStudioPoint,
): FlowEdgeBridgeLocation | null {
  return resolveSharedNearestBridgeLocation(segments, point);
}

export function resolveBridgeGeometry(
  segments: readonly FlowEdgeWaypointSegment[],
  bridge: FlowStudioEdgeBridge,
): ResolvedFlowEdgeBridgeGeometry | null {
  return resolveSharedBridgeGeometry(segments, bridge);
}

export function buildVisibleEdgePathWithBridges(
  segments: readonly FlowEdgeWaypointSegment[],
  bridges: readonly FlowStudioEdgeBridge[],
): string {
  return buildSharedVisibleEdgePathWithBridges(segments, bridges);
}

export function resolveBridgeGapIntervals(
  segments: readonly FlowEdgeWaypointSegment[],
  bridges: readonly FlowStudioEdgeBridge[],
): FlowEdgeBridgeGapInterval[] {
  return resolveSharedBridgeGapIntervals(segments, bridges);
}

export function buildEdgeInsertionIntervals(
  segments: readonly FlowEdgeWaypointSegment[],
  bridges: readonly FlowStudioEdgeBridge[],
): FlowEdgeInsertionInterval[] {
  return buildSharedEdgeInsertionIntervals(segments, bridges);
}

export function resolveInsertionIntervalMidpoint(
  segments: readonly FlowEdgeWaypointSegment[],
  interval: FlowEdgeInsertionInterval,
): FlowStudioPoint | null {
  return resolveSharedInsertionIntervalMidpoint(segments, interval);
}

export function resolveNearestInsertionInterval(
  segments: readonly FlowEdgeWaypointSegment[],
  bridges: readonly FlowStudioEdgeBridge[],
  point: FlowStudioPoint,
): FlowEdgeInsertionInterval | null {
  return resolveSharedNearestInsertionInterval(segments, bridges, point);
}

export function resolveBridgeGapFromHandle(
  geometry: ResolvedFlowEdgeBridgeGeometry,
  handlePosition: FlowStudioPoint,
): number {
  return resolveSharedBridgeGapFromHandle(geometry, handlePosition);
}

function resolveEdgeData(edge: FlowStudioEdge): FlowStudioEdgeData {
  return (
    edge.data ?? {
      kind: "token",
      order: 0,
    }
  );
}
