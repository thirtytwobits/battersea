/**
 * Copyright (c) Scott A Dixon
 *
 * Builds and mutates editable bezier waypoint geometry for flow-studio edges.
 */
import type {
  BuildAuthoringGraphWaypointSegmentsOptions,
  AuthoringGraphWaypointSegment,
} from "../graph.js";
import {
  buildAuthoringGraphWaypointPath,
  buildAuthoringGraphWaypointSegments,
  createAuthoringGraphWaypointForSegment,
  createAuthoringGraphWaypointForSegmentAt,
  evaluateAuthoringGraphCubicBezierPoint,
  insertWaypointIntoLayout,
  moveWaypointHandleInLayout,
  moveWaypointInLayout,
  removeWaypointFromLayout,
  resolveNearestWaypointSegmentIndex as resolveSharedNearestWaypointSegmentIndex,
} from "../graph.js";

import type {
  FlowStudioEdge,
  FlowStudioEdgeData,
  FlowStudioEdgeWaypoint,
  FlowStudioPoint,
} from "./dataflow-editor-state.js";

export type FlowEdgeWaypointSegment = AuthoringGraphWaypointSegment;

export type BuildFlowEdgeWaypointSegmentsOptions =
  BuildAuthoringGraphWaypointSegmentsOptions;

export function buildFlowEdgeWaypointSegments(
  options: BuildFlowEdgeWaypointSegmentsOptions,
): FlowEdgeWaypointSegment[] {
  return buildAuthoringGraphWaypointSegments(options);
}

export function buildFlowEdgeWaypointPath(
  options: BuildFlowEdgeWaypointSegmentsOptions,
): string {
  return buildAuthoringGraphWaypointPath(options);
}

export function evaluateCubicBezierPoint(
  segment: FlowEdgeWaypointSegment,
  t: number,
): FlowStudioPoint {
  return evaluateAuthoringGraphCubicBezierPoint(segment, t);
}

export function resolveNearestWaypointSegmentIndex(
  segments: readonly FlowEdgeWaypointSegment[],
  point: FlowStudioPoint,
): number | null {
  return resolveSharedNearestWaypointSegmentIndex(segments, point);
}

export function createWaypointForSegment(
  segment: FlowEdgeWaypointSegment,
): FlowStudioEdgeWaypoint {
  return createAuthoringGraphWaypointForSegment(segment);
}

export function createWaypointForSegmentAt(
  segment: FlowEdgeWaypointSegment,
  t: number,
): FlowStudioEdgeWaypoint {
  return createAuthoringGraphWaypointForSegmentAt(segment, t);
}

export function insertWaypointIntoEdge(
  edge: FlowStudioEdge,
  segmentIndex: number,
  waypoint: FlowStudioEdgeWaypoint,
  segmentT = 0.5,
): FlowStudioEdge {
  return {
    ...edge,
    data: {
      ...resolveEdgeData(edge),
      ...insertWaypointIntoLayout(edge.data, segmentIndex, waypoint, segmentT),
    },
  };
}

export function moveWaypointInEdge(
  edge: FlowStudioEdge,
  waypointIndex: number,
  position: FlowStudioPoint,
): FlowStudioEdge {
  return {
    ...edge,
    data: {
      ...resolveEdgeData(edge),
      ...moveWaypointInLayout(edge.data, waypointIndex, position),
    },
  };
}

export function moveWaypointHandleInEdge(
  edge: FlowStudioEdge,
  waypointIndex: number,
  handleKind: "inHandle" | "outHandle",
  independent: boolean,
  position: FlowStudioPoint,
): FlowStudioEdge {
  return {
    ...edge,
    data: {
      ...resolveEdgeData(edge),
      ...moveWaypointHandleInLayout(
        edge.data,
        waypointIndex,
        handleKind,
        independent,
        position,
      ),
    },
  };
}

export function removeWaypointFromEdge(
  edge: FlowStudioEdge,
  waypointIndex: number,
): FlowStudioEdge {
  return {
    ...edge,
    data: {
      ...resolveEdgeData(edge),
      ...removeWaypointFromLayout(edge.data, waypointIndex),
    },
  };
}

function resolveEdgeData(edge: FlowStudioEdge): FlowStudioEdgeData {
  return (
    edge.data ?? {
      kind: "token",
      order: 0,
    }
  );
}
