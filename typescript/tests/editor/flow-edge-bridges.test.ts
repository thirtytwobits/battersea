/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises manual edge bridge geometry for the editor's dataflow workspace.
 */
import assert from "node:assert/strict";
import test from "node:test";
import { Position } from "@xyflow/react";

import type { FlowStudioEdge } from "@battersea/editor/core/dataflow-editor-state";
import {
  buildVisibleEdgePathWithBridges,
  buildEdgeInsertionIntervals,
  createBridgeForInterval,
  createBridgeForSegment,
  insertBridgeIntoEdge,
  moveBridgeInEdge,
  removeBridgeFromEdge,
  resolveBridgeGapFromHandle,
  resolveBridgeGapIntervals,
  resolveBridgeGeometry,
  resolveInsertionIntervalMidpoint,
  resolveNearestInsertionInterval,
  resolveNearestBridgeLocation,
  resizeBridgeInEdge,
  updateBridgeIndicesForWaypointInsertion,
  updateBridgeIndicesForWaypointRemoval
} from "@battersea/editor/core/flow-edge-bridges";
import { buildFlowEdgeWaypointSegments } from "@battersea/editor/core/flow-edge-waypoints";

test("bridge mutators insert, move, resize, and remove manual bridges", () => {
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
  const inserted = insertBridgeIntoEdge(edge, createBridgeForSegment(0));
  const moved = moveBridgeInEdge(inserted, 0, { segmentIndex: 1, t: 0.25 });
  const resized = resizeBridgeInEdge(moved, 0, 22);
  const removed = removeBridgeFromEdge(resized, 0);

  assert.deepEqual(inserted.data?.bridges, [{ gap: 14, segmentIndex: 0, t: 0.5 }]);
  assert.deepEqual(moved.data?.bridges, [{ gap: 14, segmentIndex: 1, t: 0.25 }]);
  assert.deepEqual(resized.data?.bridges, [{ gap: 22, segmentIndex: 1, t: 0.25 }]);
  assert.deepEqual(removed.data?.bridges, []);
});

test("bridge helpers resolve nearest path location and size handle gap", () => {
  const segments = buildFlowEdgeWaypointSegments({
    source: { x: 0, y: 0 },
    sourcePosition: Position.Right,
    target: { x: 300, y: 0 },
    targetPosition: Position.Left,
    waypoints: []
  });
  const bridge = createBridgeForSegment(0);
  const geometry = resolveBridgeGeometry(segments, bridge);

  assert.ok(geometry);
  assert.deepEqual(resolveNearestBridgeLocation(segments, geometry!.center), { segmentIndex: 0, t: 0.5 });
  assert.equal(resolveBridgeGapFromHandle(geometry!, geometry!.sizeHandle), bridge.gap);
  assert.deepEqual(geometry!.startCapStart, { x: 136, y: -5 });
  assert.deepEqual(geometry!.startCapEnd, { x: 136, y: 5 });
  assert.deepEqual(geometry!.endCapStart, { x: 164, y: -5 });
  assert.deepEqual(geometry!.endCapEnd, { x: 164, y: 5 });
});

test("bridge visible path omits the bridged span instead of painting over it", () => {
  const segments = buildFlowEdgeWaypointSegments({
    source: { x: 0, y: 0 },
    sourcePosition: Position.Right,
    target: { x: 300, y: 0 },
    targetPosition: Position.Left,
    waypoints: []
  });
  const bridge = createBridgeForSegment(0);
  const gapIntervals = resolveBridgeGapIntervals(segments, [bridge]);
  const visiblePath = buildVisibleEdgePathWithBridges(segments, [bridge]);

  assert.equal(gapIntervals.length, 1);
  assert.ok(gapIntervals[0]!.startT < 0.5);
  assert.ok(gapIntervals[0]!.endT > 0.5);
  assert.equal((visiblePath.match(/M /g) ?? []).length, 2);
});

test("bridges split insertion intervals and midpoint placement on a segment", () => {
  const segments = buildFlowEdgeWaypointSegments({
    source: { x: 0, y: 0 },
    sourcePosition: Position.Right,
    target: { x: 300, y: 0 },
    targetPosition: Position.Left,
    waypoints: []
  });
  const intervals = buildEdgeInsertionIntervals(segments, [{
    gap: 14,
    segmentIndex: 0,
    t: 0.5
  }]);

  assert.deepEqual(intervals, [
    { endT: 0.5, segmentIndex: 0, startT: 0 },
    { endT: 1, segmentIndex: 0, startT: 0.5 }
  ]);
  assert.deepEqual(createBridgeForInterval(intervals[0]!), { gap: 14, segmentIndex: 0, t: 0.25 });
  assert.deepEqual(resolveInsertionIntervalMidpoint(segments, intervals[1]!), { x: 210.9375, y: 0 });
  assert.deepEqual(resolveNearestInsertionInterval(segments, [{
    gap: 14,
    segmentIndex: 0,
    t: 0.5
  }], { x: 220, y: 0 }), intervals[1]);
});

test("bridge indices rebalance when waypoint segments are inserted or removed", () => {
  const bridges = [
    { gap: 14, segmentIndex: 0, t: 0.5 },
    { gap: 14, segmentIndex: 2, t: 0.5 }
  ];

  assert.deepEqual(updateBridgeIndicesForWaypointInsertion(bridges, 0, 0.25), [
    { gap: 14, segmentIndex: 1, t: 0.333 },
    { gap: 14, segmentIndex: 3, t: 0.5 }
  ]);
  assert.deepEqual(updateBridgeIndicesForWaypointRemoval(bridges, 0), [
    { gap: 14, segmentIndex: 0, t: 0.25 },
    { gap: 14, segmentIndex: 1, t: 0.5 }
  ]);
});
