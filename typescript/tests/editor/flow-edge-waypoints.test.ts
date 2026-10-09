/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises editable edge waypoint geometry for the editor's dataflow workspace.
 */
import assert from "node:assert/strict";
import test from "node:test";
import { Position, getBezierPath } from "@xyflow/react";

import type { FlowStudioEdge } from "@battersea/editor/core/dataflow-editor-state";
import {
  buildFlowEdgeWaypointSegments,
  createWaypointForSegment,
  evaluateCubicBezierPoint,
  insertWaypointIntoEdge,
  moveWaypointHandleInEdge,
  moveWaypointInEdge,
  removeWaypointFromEdge,
  resolveNearestWaypointSegmentIndex
} from "@battersea/editor/core/flow-edge-waypoints";

test("buildFlowEdgeWaypointSegments creates one segment per edge interval", () => {
  const firstWaypoint = {
    inHandle: { x: 120, y: 20 },
    outHandle: { x: 180, y: 20 },
    position: { x: 150, y: 20 }
  };
  const secondWaypoint = {
    inHandle: { x: 220, y: 80 },
    outHandle: { x: 280, y: 80 },
    position: { x: 250, y: 80 }
  };

  const segments = buildFlowEdgeWaypointSegments({
    source: { x: 0, y: 0 },
    sourcePosition: Position.Right,
    target: { x: 400, y: 100 },
    targetPosition: Position.Left,
    waypoints: [firstWaypoint, secondWaypoint]
  });

  assert.equal(segments.length, 3);
  assert.deepEqual(segments[0]?.start, { x: 0, y: 0 });
  assert.deepEqual(segments[0]?.end, firstWaypoint.position);
  assert.deepEqual(segments[1]?.controlA, firstWaypoint.outHandle);
  assert.deepEqual(segments[1]?.controlB, secondWaypoint.inHandle);
  assert.deepEqual(segments[2]?.end, { x: 400, y: 100 });
});

test("createWaypointForSegment places a new anchor midway along the segment tangent", () => {
  const segment = {
    controlA: { x: 80, y: 0 },
    controlB: { x: 120, y: 0 },
    end: { x: 200, y: 0 },
    start: { x: 0, y: 0 }
  };

  const waypoint = createWaypointForSegment(segment);

  assert.equal(waypoint.position.y, 0);
  assert.equal(waypoint.inHandle.y, 0);
  assert.equal(waypoint.outHandle.y, 0);
  assert.ok(waypoint.inHandle.x < waypoint.position.x);
  assert.ok(waypoint.outHandle.x > waypoint.position.x);
});

test("buildFlowEdgeWaypointSegments matches the default bezier midpoint for edges without waypoints", () => {
  const sourceX = 40;
  const sourceY = 120;
  const targetX = 360;
  const targetY = 260;
  const segments = buildFlowEdgeWaypointSegments({
    source: { x: sourceX, y: sourceY },
    sourcePosition: Position.Right,
    target: { x: targetX, y: targetY },
    targetPosition: Position.Left,
    waypoints: []
  });
  const [, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    sourcePosition: Position.Right,
    targetX,
    targetY,
    targetPosition: Position.Left
  });
  const midpoint = evaluateCubicBezierPoint(segments[0]!, 0.5);

  assert.equal(segments.length, 1);
  assert.equal(midpoint.x, labelX);
  assert.equal(midpoint.y, labelY);
});

test("resolveNearestWaypointSegmentIndex returns the closest segment to a pointer", () => {
  const segments = buildFlowEdgeWaypointSegments({
    source: { x: 0, y: 0 },
    sourcePosition: Position.Right,
    target: { x: 400, y: 0 },
    targetPosition: Position.Left,
    waypoints: [{
      inHandle: { x: 120, y: 0 },
      outHandle: { x: 180, y: 0 },
      position: { x: 150, y: 0 }
    }]
  });

  assert.equal(resolveNearestWaypointSegmentIndex(segments, { x: 40, y: 2 }), 0);
  assert.equal(resolveNearestWaypointSegmentIndex(segments, { x: 340, y: -2 }), 1);
});

test("waypoint anchor moves preserve both handle offsets relative to the anchor", () => {
  const edge: FlowStudioEdge = {
    data: {
      kind: "token",
      order: 1
    },
    id: "edge-1",
    source: "source-1",
    sourceHandle: "output-0",
    target: "target-1",
    targetHandle: "input-0"
  };
  const inserted = insertWaypointIntoEdge(edge, 0, {
    inHandle: { x: 120, y: 10 },
    outHandle: { x: 180, y: 10 },
    position: { x: 150, y: 10 }
  });
  const movedAnchor = moveWaypointInEdge(inserted, 0, { x: 170, y: 40 });

  assert.equal(inserted.data?.waypoints?.length, 1);
  assert.deepEqual(movedAnchor.data?.waypoints?.[0]?.position, { x: 170, y: 40 });
  assert.deepEqual(movedAnchor.data?.waypoints?.[0]?.inHandle, { x: 140, y: 40 });
  assert.deepEqual(movedAnchor.data?.waypoints?.[0]?.outHandle, { x: 200, y: 40 });
});

test("waypoint handle drags mirror the opposite handle unless independent mode is requested", () => {
  const edge: FlowStudioEdge = {
    data: {
      kind: "token",
      order: 1,
      waypoints: [{
        inHandle: { x: 140, y: 40 },
        outHandle: { x: 200, y: 40 },
        position: { x: 170, y: 40 }
      }]
    },
    id: "edge-1",
    source: "source-1",
    sourceHandle: "output-0",
    target: "target-1",
    targetHandle: "input-0"
  };

  const movedHandleLinked = moveWaypointHandleInEdge(edge, 0, "outHandle", false, { x: 240, y: 60 });
  const movedHandleIndependent = moveWaypointHandleInEdge(
    movedHandleLinked,
    0,
    "inHandle",
    true,
    { x: 130, y: 55 }
  );

  assert.deepEqual(movedHandleLinked.data?.waypoints?.[0]?.outHandle, { x: 240, y: 60 });
  assert.deepEqual(movedHandleLinked.data?.waypoints?.[0]?.inHandle, { x: 100, y: 20 });
  assert.deepEqual(movedHandleIndependent.data?.waypoints?.[0]?.inHandle, { x: 130, y: 55 });
  assert.deepEqual(movedHandleIndependent.data?.waypoints?.[0]?.outHandle, { x: 240, y: 60 });
});

test("removing a waypoint clears it without disturbing the rest of the edge", () => {
  const edge: FlowStudioEdge = {
    data: {
      kind: "token",
      order: 1,
      waypoints: [{
        inHandle: { x: 130, y: 55 },
        outHandle: { x: 240, y: 60 },
        position: { x: 170, y: 40 }
      }]
    },
    id: "edge-1",
    source: "source-1",
    sourceHandle: "output-0",
    target: "target-1",
    targetHandle: "input-0"
  };

  const removed = removeWaypointFromEdge(edge, 0);

  assert.deepEqual(removed.data?.waypoints, []);
});
