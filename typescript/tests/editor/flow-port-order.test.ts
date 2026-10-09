/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises node port ordering helpers in the editor's dataflow workspace.
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  applyFlowPortOrder,
  normalizeFlowStudioPortOrder,
  remapNodeSideEdgeHandles,
  resolveReorderedFlowStudioPortOrder
} from "@battersea/editor/core/flow-port-order";
import type { FlowStudioResolvedPort } from "@battersea/editor/core/flow-node-ports";

function makePort(id: string, side: FlowStudioResolvedPort["side"]): FlowStudioResolvedPort {
  return {
    displayClass: "inline",
    id,
    label: id,
    side
  };
}

test("applyFlowPortOrder respects saved ids and leaves unknown ports in default order afterwards", () => {
  const ports = [
    makePort("input-0", "input"),
    makePort("input-1", "input"),
    makePort("input-2", "input")
  ];

  const reordered = applyFlowPortOrder(ports, "input", {
    input: ["input-2", "missing", "input-0"]
  });

  assert.deepEqual(reordered.map((port) => port.id), ["input-2", "input-0", "input-1"]);
});

test("normalizeFlowStudioPortOrder preserves survivor order and appends new dynamic ports", () => {
  const normalized = normalizeFlowStudioPortOrder({
    actionPorts: [],
    automationPorts: [],
    inputPorts: [
      makePort("input-0", "input"),
      makePort("input-1", "input"),
      makePort("input-2", "input"),
      makePort("input-3", "input")
    ],
    outputPorts: [makePort("output", "output")],
    portOrder: {
      input: ["input-2", "input-0", "input-1"]
    },
    signalPorts: [makePort("post_activate", "signal")]
  });

  assert.deepEqual(normalized, {
    input: ["input-2", "input-0", "input-1", "input-3"]
  });
});

test("resolveReorderedFlowStudioPortOrder moves the active port before the hovered port", () => {
  const nextOrder = resolveReorderedFlowStudioPortOrder({
    activePortId: "input-2",
    currentPorts: [
      makePort("input-0", "input"),
      makePort("input-1", "input"),
      makePort("input-2", "input")
    ],
    overPortId: "input-0",
    portOrder: undefined,
    side: "input"
  });

  assert.deepEqual(nextOrder, {
    input: ["input-2", "input-0", "input-1"]
  });
});

test("remapNodeSideEdgeHandles retargets source handles to the reordered port indices", () => {
  const nextEdges = remapNodeSideEdgeHandles({
    edges: [{
      data: {
        kind: "token",
        order: 0
      },
      id: "edge-1",
      source: "node-1",
      sourceHandle: "output-0",
      target: "node-2",
      targetHandle: "input-0"
    }, {
      data: {
        kind: "token",
        order: 1
      },
      id: "edge-2",
      source: "node-1",
      sourceHandle: "output-2",
      target: "node-3",
      targetHandle: "input-0"
    }],
    nextPorts: [
      makePort("output-2", "output"),
      makePort("output-0", "output"),
      makePort("output-1", "output")
    ],
    nodeId: "node-1",
    previousPorts: [
      makePort("output-0", "output"),
      makePort("output-1", "output"),
      makePort("output-2", "output")
    ],
    side: "output"
  });

  assert.deepEqual(
    nextEdges.map((edge) => edge.sourceHandle),
    ["output-1", "output-0"]
  );
});
