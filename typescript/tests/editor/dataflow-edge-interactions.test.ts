/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises parity-critical edge interaction state for the Dataflow editor.
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveDataflowEdgeBridgeInsertion,
  resolveDataflowEdgeBridgeSelection,
  resolveDataflowEdgeWaypointInsertion,
  resolveDataflowEdgeWaypointSelection,
  updateDataflowEdgeBridgeGap,
  updateDataflowEdgeBridgePosition
} from "@battersea/editor/core/dataflow-edge-interactions";
import type { FlowStudioWorkspaceState } from "@battersea/editor/core/flow-persistence";

function buildWorkspace(): FlowStudioWorkspaceState {
  return {
    baselineFlow: null,
    description: "",
    draftFlowKey: "draft-flow",
    outputEncoding: "xml",
    plainFragmentDelimiter: "blank_line",
    whitespaceMode: "trim",
    edges: [
      {
        data: {
          bridges: [{ gap: 14, segmentIndex: 0, t: 0.5 }],
          kind: "token",
          order: 1,
          waypoints: [{
            inHandle: { x: 120, y: 20 },
            outHandle: { x: 180, y: 20 },
            position: { x: 150, y: 20 }
          }]
        },
        id: "edge-1",
        selected: true,
        source: "source-1",
        sourceHandle: "output-0",
        target: "target-1",
        targetHandle: "input-0"
      },
      {
        data: {
          bridges: [{ gap: 16, segmentIndex: 1, t: 0.25 }],
          kind: "token",
          order: 2
        },
        id: "edge-2",
        source: "source-2",
        sourceHandle: "output-0",
        target: "target-2",
        targetHandle: "input-0"
      }
    ],
    nodes: [],
    selectedFlowKey: "draft-flow",
    selectedTarget: {
      kind: "node",
      nodeId: "source-1"
    },
    title: "Draft Flow"
  };
}

test("selecting a bridge clears node and native edge selection and records the selected bridge", () => {
  const result = resolveDataflowEdgeBridgeSelection(buildWorkspace(), "edge-1", 0);

  assert.deepEqual(result.selectedEdgeBridge, {
    bridgeIndex: 0,
    edgeId: "edge-1"
  });
  assert.equal(result.selectedEdgeWaypoint, null);
  assert.deepEqual(result.workspace.selectedTarget, { kind: "none" });
  assert.equal(result.workspace.edges[0]?.selected, false);
});

test("selecting a waypoint clears node and native edge selection and records the selected waypoint", () => {
  const result = resolveDataflowEdgeWaypointSelection(buildWorkspace(), "edge-1", 0);

  assert.equal(result.selectedEdgeBridge, null);
  assert.deepEqual(result.selectedEdgeWaypoint, {
    edgeId: "edge-1",
    waypointIndex: 0
  });
  assert.deepEqual(result.workspace.selectedTarget, { kind: "none" });
  assert.equal(result.workspace.edges[0]?.selected, false);
});

test("inserting a bridge clears waypoint selection and selects the new bridge on that edge", () => {
  const result = resolveDataflowEdgeBridgeInsertion(buildWorkspace(), "edge-1", {
    gap: 20,
    segmentIndex: 0,
    t: 0.75
  });

  assert.deepEqual(result.selectedEdgeBridge, {
    bridgeIndex: 1,
    edgeId: "edge-1"
  });
  assert.equal(result.selectedEdgeWaypoint, null);
  assert.deepEqual(result.workspace.selectedTarget, { kind: "none" });
  assert.deepEqual(result.workspace.edges[0]?.data?.bridges, [
    { gap: 14, segmentIndex: 0, t: 0.5 },
    { gap: 20, segmentIndex: 0, t: 0.75 }
  ]);
});

test("inserting a waypoint clears bridge selection and selects the inserted waypoint index", () => {
  const result = resolveDataflowEdgeWaypointInsertion(buildWorkspace(), "edge-1", 0, 0.5, {
    inHandle: { x: 200, y: 40 },
    outHandle: { x: 260, y: 40 },
    position: { x: 230, y: 40 }
  });

  assert.equal(result.selectedEdgeBridge, null);
  assert.deepEqual(result.selectedEdgeWaypoint, {
    edgeId: "edge-1",
    waypointIndex: 0
  });
  assert.deepEqual(result.workspace.selectedTarget, { kind: "none" });
  assert.equal(result.workspace.edges[0]?.data?.waypoints?.length, 2);
  assert.deepEqual(result.workspace.edges[0]?.data?.waypoints?.[0], {
    inHandle: { x: 200, y: 40 },
    outHandle: { x: 260, y: 40 },
    position: { x: 230, y: 40 }
  });
});

test("updating bridge position only rewrites the targeted bridge on the targeted edge", () => {
  const workspace = updateDataflowEdgeBridgePosition(buildWorkspace(), "edge-2", 0, 3, 0.9);

  assert.deepEqual(workspace.edges[0]?.data?.bridges, [{ gap: 14, segmentIndex: 0, t: 0.5 }]);
  assert.deepEqual(workspace.edges[1]?.data?.bridges, [{ gap: 16, segmentIndex: 3, t: 0.9 }]);
});

test("updating bridge gap only rewrites the targeted bridge gap on the targeted edge", () => {
  const workspace = updateDataflowEdgeBridgeGap(buildWorkspace(), "edge-2", 0, 28);

  assert.deepEqual(workspace.edges[0]?.data?.bridges, [{ gap: 14, segmentIndex: 0, t: 0.5 }]);
  assert.deepEqual(workspace.edges[1]?.data?.bridges, [{ gap: 28, segmentIndex: 1, t: 0.25 }]);
});
