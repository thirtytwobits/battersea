/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises live edge activation helpers for the editor's dataflow workspace.
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  FLOW_STUDIO_EDGE_MIN_ACTIVE_MS,
  pruneFlowStudioEdgeActivationState,
  resolveFlowStudioEdgeFadeDelayMs,
  resolveFlowStudioEdgeActivityEvent
} from "@battersea/editor/core/flow-edge-activation";

test("resolveFlowStudioEdgeActivityEvent activates every emitted target edge", () => {
  assert.deepEqual(resolveFlowStudioEdgeActivityEvent({
    detail: {
      signalPort: "post_activate",
      targetCount: 2,
      targets: [
        { edgeId: "edge-1", targetFlowKey: "flow-a", targetNodeId: "b", targetPort: "action" },
        { edgeId: "edge-2", targetFlowKey: "flow-a", targetNodeId: "c", targetPort: "action" }
      ]
    },
    id: "record-1",
    path: "/dev/sessions/session-1/flows/current/raw",
    phase: "flow.signal.emit",
    stream: "flow",
    summary: "Emitted signal post_activate.",
    timestamp: "2026-03-21T10:00:00Z"
  }), {
    edgeIds: ["edge-1", "edge-2"],
    transition: "activate"
  });
});

test("resolveFlowStudioEdgeActivityEvent fades an edge once the receive arrives", () => {
  assert.deepEqual(resolveFlowStudioEdgeActivityEvent({
    detail: {
      edgeId: "edge-9",
      sourceNodeId: "a",
      sourcePort: "output",
      targetPort: "input",
      tokenType: "prompt.fragment"
    },
    id: "record-2",
    path: "/dev/sessions/session-1/flows/current/raw",
    phase: "flow.token.receive",
    stream: "flow",
    summary: "Received prompt.fragment on input.",
    timestamp: "2026-03-21T10:00:01Z"
  }), {
    edgeIds: ["edge-9"],
    transition: "fade"
  });
});

test("resolveFlowStudioEdgeActivityEvent ignores unrelated records and malformed detail", () => {
  assert.equal(resolveFlowStudioEdgeActivityEvent({
    detail: {
      edgeId: 42
    },
    id: "record-3",
    path: "/dev/sessions/session-1/flows/current/raw",
    phase: "flow.node.complete",
    stream: "flow",
    summary: "Completed flow node.",
    timestamp: "2026-03-21T10:00:02Z"
  }), null);
  assert.equal(resolveFlowStudioEdgeActivityEvent({
    detail: {
      targets: [{ targetFlowKey: "flow-a", targetNodeId: "b" }]
    },
    id: "record-4",
    path: "/dev/sessions/session-1/flows/current/raw",
    phase: "flow.token.emit",
    stream: "flow",
    summary: "Emitted prompt.fragment on output.",
    timestamp: "2026-03-21T10:00:03Z"
  }), null);
});

test("pruneFlowStudioEdgeActivationState removes edges that are no longer visible", () => {
  assert.deepEqual(
    pruneFlowStudioEdgeActivationState(
      {
        "edge-1": "active",
        "edge-2": "fading"
      },
      new Set(["edge-2", "edge-3"])
    ),
    {
      "edge-2": "fading"
    }
  );
});

test("resolveFlowStudioEdgeFadeDelayMs keeps fast activations visibly active for a minimum time", () => {
  assert.equal(resolveFlowStudioEdgeFadeDelayMs({
    activatedAtMs: 1000,
    nowMs: 1000
  }), FLOW_STUDIO_EDGE_MIN_ACTIVE_MS);
  assert.equal(resolveFlowStudioEdgeFadeDelayMs({
    activatedAtMs: 1000,
    nowMs: 1000 + FLOW_STUDIO_EDGE_MIN_ACTIVE_MS - 10
  }), 10);
  assert.equal(resolveFlowStudioEdgeFadeDelayMs({
    activatedAtMs: 1000,
    nowMs: 1000 + FLOW_STUDIO_EDGE_MIN_ACTIVE_MS + 10
  }), 0);
});
