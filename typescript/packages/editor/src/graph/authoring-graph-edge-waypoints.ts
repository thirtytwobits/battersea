/**
 * Copyright (c) Scott A Dixon
 *
 * Builds and mutates editable bezier waypoint geometry for authoring-graph edges.
 */
import { Position } from "@xyflow/react";

import type {
  AuthoringGraphEdgeLayout,
  AuthoringGraphPoint,
  AuthoringGraphWaypoint,
} from "./authoring-graph-edge-types.js";
import {
  updateBridgeIndicesForWaypointInsertion,
  updateBridgeIndicesForWaypointRemoval,
} from "./authoring-graph-edge-bridges.js";

export interface AuthoringGraphWaypointSegment {
  controlA: AuthoringGraphPoint;
  controlB: AuthoringGraphPoint;
  end: AuthoringGraphPoint;
  start: AuthoringGraphPoint;
}

export interface BuildAuthoringGraphWaypointSegmentsOptions {
  source: AuthoringGraphPoint;
  sourcePosition: Position;
  target: AuthoringGraphPoint;
  targetPosition: Position;
  waypoints: readonly AuthoringGraphWaypoint[];
}

const DEFAULT_BEZIER_CURVATURE = 0.25;
const INSERT_HANDLE_DISTANCE_PX = 48;
const SEGMENT_SAMPLE_COUNT = 18;

export function buildAuthoringGraphWaypointSegments(
  options: BuildAuthoringGraphWaypointSegmentsOptions,
): AuthoringGraphWaypointSegment[] {
  const anchors = [
    options.source,
    ...options.waypoints.map((waypoint) => waypoint.position),
    options.target,
  ];

  return anchors.slice(0, -1).map((start, index) => {
    const nextWaypoint = options.waypoints[index];
    const previousWaypoint = options.waypoints[index - 1];
    const end = anchors[index + 1]!;

    return {
      controlA: previousWaypoint
        ? previousWaypoint.outHandle
        : resolveEndpointHandle(start, end, options.sourcePosition),
      controlB: nextWaypoint
        ? nextWaypoint.inHandle
        : resolveEndpointHandle(end, start, options.targetPosition),
      end,
      start,
    };
  });
}

export function buildAuthoringGraphWaypointPath(
  options: BuildAuthoringGraphWaypointSegmentsOptions,
): string {
  const segments = buildAuthoringGraphWaypointSegments(options);
  if (segments.length === 0) {
    return "";
  }

  return segments.reduce(
    (path, segment, index) =>
      index === 0
        ? `M ${formatPoint(segment.start)} C ${formatPoint(segment.controlA)}, ${formatPoint(segment.controlB)}, ${formatPoint(segment.end)}`
        : `${path} C ${formatPoint(segment.controlA)}, ${formatPoint(segment.controlB)}, ${formatPoint(segment.end)}`,
    "",
  );
}

export function evaluateAuthoringGraphCubicBezierPoint(
  segment: AuthoringGraphWaypointSegment,
  t: number,
): AuthoringGraphPoint {
  const mt = 1 - t;
  const mt2 = mt * mt;
  const t2 = t * t;

  return {
    x:
      mt2 * mt * segment.start.x +
      3 * mt2 * t * segment.controlA.x +
      3 * mt * t2 * segment.controlB.x +
      t2 * t * segment.end.x,
    y:
      mt2 * mt * segment.start.y +
      3 * mt2 * t * segment.controlA.y +
      3 * mt * t2 * segment.controlB.y +
      t2 * t * segment.end.y,
  };
}

export function resolveNearestWaypointSegmentIndex(
  segments: readonly AuthoringGraphWaypointSegment[],
  point: AuthoringGraphPoint,
): number | null {
  if (segments.length === 0) {
    return null;
  }

  let bestIndex = 0;
  let bestDistance = Number.POSITIVE_INFINITY;

  segments.forEach((segment, index) => {
    for (
      let sampleIndex = 0;
      sampleIndex <= SEGMENT_SAMPLE_COUNT;
      sampleIndex += 1
    ) {
      const sample = evaluateAuthoringGraphCubicBezierPoint(
        segment,
        sampleIndex / SEGMENT_SAMPLE_COUNT,
      );
      const distance = squaredDistance(sample, point);
      if (distance < bestDistance) {
        bestDistance = distance;
        bestIndex = index;
      }
    }
  });

  return bestIndex;
}

export function createAuthoringGraphWaypointForSegment(
  segment: AuthoringGraphWaypointSegment,
): AuthoringGraphWaypoint {
  return createAuthoringGraphWaypointForSegmentAt(segment, 0.5);
}

export function createAuthoringGraphWaypointForSegmentAt(
  segment: AuthoringGraphWaypointSegment,
  t: number,
): AuthoringGraphWaypoint {
  const position = evaluateAuthoringGraphCubicBezierPoint(segment, t);
  const tangent = normalisePoint({
    x: cubicDerivative(segment, t).x,
    y: cubicDerivative(segment, t).y,
  });

  return {
    inHandle: {
      x: Number(
        (position.x - tangent.x * INSERT_HANDLE_DISTANCE_PX).toFixed(3),
      ),
      y: Number(
        (position.y - tangent.y * INSERT_HANDLE_DISTANCE_PX).toFixed(3),
      ),
    },
    outHandle: {
      x: Number(
        (position.x + tangent.x * INSERT_HANDLE_DISTANCE_PX).toFixed(3),
      ),
      y: Number(
        (position.y + tangent.y * INSERT_HANDLE_DISTANCE_PX).toFixed(3),
      ),
    },
    position: {
      x: Number(position.x.toFixed(3)),
      y: Number(position.y.toFixed(3)),
    },
  };
}

export function insertWaypointIntoLayout(
  layout: AuthoringGraphEdgeLayout | undefined,
  segmentIndex: number,
  waypoint: AuthoringGraphWaypoint,
  segmentT = 0.5,
): AuthoringGraphEdgeLayout {
  const bridges = layout?.bridges;
  const waypoints = layout?.waypoints ?? [];

  return {
    bridges: updateBridgeIndicesForWaypointInsertion(
      bridges,
      segmentIndex,
      segmentT,
    ),
    waypoints: [
      ...waypoints.slice(0, segmentIndex),
      waypoint,
      ...waypoints.slice(segmentIndex),
    ],
  };
}

export function moveWaypointInLayout(
  layout: AuthoringGraphEdgeLayout | undefined,
  waypointIndex: number,
  position: AuthoringGraphPoint,
): AuthoringGraphEdgeLayout {
  const waypoint = layout?.waypoints?.[waypointIndex];
  if (!waypoint) {
    return layout ?? {};
  }

  const delta = {
    x: position.x - waypoint.position.x,
    y: position.y - waypoint.position.y,
  };

  return patchWaypoint(layout, waypointIndex, {
    inHandle: {
      x: Number((waypoint.inHandle.x + delta.x).toFixed(3)),
      y: Number((waypoint.inHandle.y + delta.y).toFixed(3)),
    },
    outHandle: {
      x: Number((waypoint.outHandle.x + delta.x).toFixed(3)),
      y: Number((waypoint.outHandle.y + delta.y).toFixed(3)),
    },
    position: roundPoint(position),
  });
}

export function moveWaypointHandleInLayout(
  layout: AuthoringGraphEdgeLayout | undefined,
  waypointIndex: number,
  handleKind: "inHandle" | "outHandle",
  independent: boolean,
  position: AuthoringGraphPoint,
): AuthoringGraphEdgeLayout {
  const waypoint = layout?.waypoints?.[waypointIndex];
  if (!waypoint) {
    return layout ?? {};
  }

  const nextHandlePosition = roundPoint(position);
  if (independent) {
    return patchWaypoint(layout, waypointIndex, {
      ...waypoint,
      [handleKind]: nextHandlePosition,
    });
  }

  const anchor = waypoint.position;
  const oppositeHandleKind =
    handleKind === "inHandle" ? "outHandle" : "inHandle";
  const delta = {
    x: nextHandlePosition.x - anchor.x,
    y: nextHandlePosition.y - anchor.y,
  };

  return patchWaypoint(layout, waypointIndex, {
    ...waypoint,
    [handleKind]: nextHandlePosition,
    [oppositeHandleKind]: {
      x: Number((anchor.x - delta.x).toFixed(3)),
      y: Number((anchor.y - delta.y).toFixed(3)),
    },
  });
}

export function removeWaypointFromLayout(
  layout: AuthoringGraphEdgeLayout | undefined,
  waypointIndex: number,
): AuthoringGraphEdgeLayout {
  const waypoints = layout?.waypoints ?? [];
  if (!waypoints[waypointIndex]) {
    return layout ?? {};
  }

  return {
    bridges: updateBridgeIndicesForWaypointRemoval(
      layout?.bridges,
      waypointIndex,
    ),
    waypoints: [
      ...waypoints.slice(0, waypointIndex),
      ...waypoints.slice(waypointIndex + 1),
    ],
  };
}

function patchWaypoint(
  layout: AuthoringGraphEdgeLayout | undefined,
  waypointIndex: number,
  waypoint: AuthoringGraphWaypoint,
): AuthoringGraphEdgeLayout {
  const waypoints = layout?.waypoints ?? [];

  return {
    bridges: layout?.bridges,
    waypoints: waypoints.map((entry, index) =>
      index === waypointIndex ? waypoint : entry,
    ),
  };
}

function resolveEndpointHandle(
  anchor: AuthoringGraphPoint,
  other: AuthoringGraphPoint,
  position: Position,
): AuthoringGraphPoint {
  if (position === Position.Left) {
    return {
      x: Number(
        (anchor.x - calculateBezierControlOffset(anchor.x - other.x)).toFixed(
          3,
        ),
      ),
      y: anchor.y,
    };
  }

  if (position === Position.Right) {
    return {
      x: Number(
        (anchor.x + calculateBezierControlOffset(other.x - anchor.x)).toFixed(
          3,
        ),
      ),
      y: anchor.y,
    };
  }

  if (position === Position.Top) {
    return {
      x: anchor.x,
      y: Number(
        (anchor.y - calculateBezierControlOffset(anchor.y - other.y)).toFixed(
          3,
        ),
      ),
    };
  }

  return {
    x: anchor.x,
    y: Number(
      (anchor.y + calculateBezierControlOffset(other.y - anchor.y)).toFixed(3),
    ),
  };
}

function calculateBezierControlOffset(distance: number): number {
  if (distance >= 0) {
    return 0.5 * distance;
  }

  return DEFAULT_BEZIER_CURVATURE * 25 * Math.sqrt(-distance);
}

function cubicDerivative(
  segment: AuthoringGraphWaypointSegment,
  t: number,
): AuthoringGraphPoint {
  const mt = 1 - t;

  return {
    x:
      3 * mt * mt * (segment.controlA.x - segment.start.x) +
      6 * mt * t * (segment.controlB.x - segment.controlA.x) +
      3 * t * t * (segment.end.x - segment.controlB.x),
    y:
      3 * mt * mt * (segment.controlA.y - segment.start.y) +
      6 * mt * t * (segment.controlB.y - segment.controlA.y) +
      3 * t * t * (segment.end.y - segment.controlB.y),
  };
}

function normalisePoint(point: AuthoringGraphPoint): AuthoringGraphPoint {
  const magnitude = Math.hypot(point.x, point.y);
  if (magnitude <= 0.0001) {
    return { x: 1, y: 0 };
  }

  return {
    x: Number((point.x / magnitude).toFixed(6)),
    y: Number((point.y / magnitude).toFixed(6)),
  };
}

function roundPoint(point: AuthoringGraphPoint): AuthoringGraphPoint {
  return {
    x: Number(point.x.toFixed(3)),
    y: Number(point.y.toFixed(3)),
  };
}

function formatPoint(point: AuthoringGraphPoint): string {
  return `${point.x} ${point.y}`;
}

function squaredDistance(
  left: AuthoringGraphPoint,
  right: AuthoringGraphPoint,
): number {
  const deltaX = left.x - right.x;
  const deltaY = left.y - right.y;
  return deltaX * deltaX + deltaY * deltaY;
}
