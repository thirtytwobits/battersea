/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises canvas edge presentation for the editor's dataflow workspace.
 */
import assert from "node:assert/strict";
import test from "node:test";
import type { Node } from "@xyflow/react";

import type { FlowStudioEdge } from "@battersea/editor/core/dataflow-editor-state";
import type { FlowStudioNodeData } from "@battersea/editor/core/flow-drag";
import { buildResolvedFlowPort } from "@battersea/editor/core/flow-node-ports";
import { isArrayTokenType, resolveFlowCanvasEdge } from "@battersea/editor/core/flow-canvas-edges";

function makeNode(options: {
  id: string;
  inputTokenType?: string;
  outputTokenType?: string;
}): Node<FlowStudioNodeData> {
  return {
    id: options.id,
    className: "flow-studio-node flow-studio-node--inline",
    data: {
      actionPorts: [],
      automationPorts: [],
      definitionName: "TestNode",
      inputPorts: options.inputTokenType
        ? [buildResolvedFlowPort({
            id: "input",
            side: "input",
            tokenType: options.inputTokenType
          })]
        : [],
      instanceName: options.id,
      longDescription: "",
      nodeClass: "inline",
      outputPorts: options.outputTokenType
        ? [buildResolvedFlowPort({
            id: "output",
            side: "output",
            tokenType: options.outputTokenType
          })]
        : [],
      parameterValues: {},
      signalPorts: [],
      shortDescription: ""
    },
    position: { x: 0, y: 0 },
    type: "flowStudio"
  };
}

test("resolveFlowCanvasEdge keeps token edges solid", () => {
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
  const nodes = [
    makeNode({ id: "source-1", outputTokenType: "prompt.fragment" }),
    makeNode({ id: "target-1", inputTokenType: "prompt.fragment" })
  ];

  const resolved = resolveFlowCanvasEdge(edge, nodes);
  assert.equal(resolved.className, "flow-studio-edge");
  assert.equal(resolved.animated, false);
  assert.equal(resolved.selectable, undefined);
  assert.equal(resolved.type, "flowStudio");
});

test("resolveFlowCanvasEdge marks signal edges with the dashed edge class", () => {
  const edge: FlowStudioEdge = {
    data: {
      kind: "signal",
      order: 1
    },
    id: "edge-1",
    source: "source-1",
    sourceHandle: "signal-0",
    target: "target-1",
    targetHandle: "action-0"
  };
  const nodes = [makeNode({ id: "source-1" }), makeNode({ id: "target-1" })];

  const resolved = resolveFlowCanvasEdge(edge, nodes);
  assert.equal(resolved.className, "flow-studio-edge flow-studio-edge--signal");
  assert.equal(resolved.animated, false);
});

test("resolveFlowCanvasEdge preserves existing edge classes", () => {
  const edge: FlowStudioEdge = {
    className: "selected",
    data: {
      kind: "signal",
      order: 1
    },
    id: "edge-1",
    source: "source-1",
    sourceHandle: "signal-0",
    target: "target-1",
    targetHandle: "action-0"
  };
  const nodes = [makeNode({ id: "source-1" }), makeNode({ id: "target-1" })];

  const resolved = resolveFlowCanvasEdge(edge, nodes);
  assert.equal(resolved.className, "flow-studio-edge selected flow-studio-edge--signal");
  assert.equal(resolved.animated, false);
});

test("resolveFlowCanvasEdge marks active edges for glow treatment", () => {
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
  const nodes = [
    makeNode({ id: "source-1", outputTokenType: "prompt.fragment" }),
    makeNode({ id: "target-1", inputTokenType: "prompt.fragment" })
  ];

  const resolved = resolveFlowCanvasEdge(edge, nodes, "active");
  assert.equal(resolved.className, "flow-studio-edge flow-studio-edge--active");
  assert.equal(resolved.animated, true);
});

test("resolveFlowCanvasEdge marks fading edges separately from active ones", () => {
  const edge: FlowStudioEdge = {
    data: {
      kind: "signal",
      order: 1
    },
    id: "edge-1",
    source: "source-1",
    sourceHandle: "signal-0",
    target: "target-1",
    targetHandle: "action-0"
  };
  const nodes = [makeNode({ id: "source-1" }), makeNode({ id: "target-1" })];

  const resolved = resolveFlowCanvasEdge(edge, nodes, "fading");
  assert.equal(resolved.className, "flow-studio-edge flow-studio-edge--signal flow-studio-edge--fading");
  assert.equal(resolved.animated, false);
});

test("resolveFlowCanvasEdge marks fragmentArray token edges for the double-line renderer", () => {
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
  const nodes = [
    makeNode({ id: "source-1", outputTokenType: "prompt.fragmentArray" }),
    makeNode({ id: "target-1", inputTokenType: "prompt.fragment" })
  ];

  const resolved = resolveFlowCanvasEdge(edge, nodes);
  assert.equal(resolved.className, "flow-studio-edge flow-studio-edge--fragment-array");
  assert.equal(resolved.data?.tokenType, "prompt.fragmentArray");
  assert.equal(resolved.type, "flowStudio");
});

test("resolveFlowCanvasEdge marks _ids and _list token edges for the double-line renderer", () => {
  const baseEdge: FlowStudioEdge = {
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
  for (const tokenType of ["story.character_ids", "media.asset_list"] as const) {
    const nodes = [
      makeNode({ id: "source-1", outputTokenType: tokenType }),
      makeNode({ id: "target-1", inputTokenType: tokenType })
    ];

    const resolved = resolveFlowCanvasEdge(baseEdge, nodes);
    assert.equal(
      resolved.className,
      "flow-studio-edge flow-studio-edge--fragment-array",
      `expected double-line styling for ${tokenType}`
    );
  }
});

test("isArrayTokenType identifies array-shaped token types and rejects scalar ones", () => {
  // Array shapes (camelCase suffix + snake_case suffixes).
  assert.equal(isArrayTokenType("prompt.fragmentArray"), true);
  assert.equal(isArrayTokenType("story.character_ids"), true);
  assert.equal(isArrayTokenType("media.asset_list"), true);
  assert.equal(isArrayTokenType("future.tag_array"), true);

  // Scalar shapes.
  assert.equal(isArrayTokenType("prompt.fragment"), false);
  assert.equal(isArrayTokenType("chat.response"), false);
  assert.equal(isArrayTokenType("world.place_uid"), false);
  assert.equal(isArrayTokenType(null), false);
  assert.equal(isArrayTokenType(""), false);
});

test("resolveFlowCanvasEdge preserves editor-local waypoint geometry", () => {
  const edge: FlowStudioEdge = {
    data: {
      bridges: [{
        gap: 18,
        segmentIndex: 0,
        t: 0.5
      }],
      kind: "token",
      order: 1,
      waypoints: [{
        inHandle: { x: 120, y: 20 },
        outHandle: { x: 180, y: 20 },
        position: { x: 150, y: 20 }
      }]
    },
    id: "edge-1",
    source: "source-1",
    sourceHandle: "output-0",
    target: "target-1",
    targetHandle: "input-0"
  };
  const nodes = [
    makeNode({ id: "source-1", outputTokenType: "prompt.fragment" }),
    makeNode({ id: "target-1", inputTokenType: "prompt.fragment" })
  ];

  const resolved = resolveFlowCanvasEdge(edge, nodes);
  assert.deepEqual(resolved.data?.bridges, edge.data?.bridges);
  assert.deepEqual(resolved.data?.waypoints, edge.data?.waypoints);
  assert.equal(resolved.selectable, undefined);
});
