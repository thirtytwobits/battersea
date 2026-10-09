/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises dataflow canvas drop helper behaviour in the editor's dataflow workspace.
 */
import assert from "node:assert/strict";
import test from "node:test";
import type { ReactFlowInstance } from "@xyflow/react";

import type { FlowNodeDefinition as WireFlowNodeDefinition } from "@battersea/flow";

import type { FlowStudioEdge, FlowStudioNode } from "@battersea/editor/core/dataflow-editor-state";
import { didDataflowPaletteDragMove, didDataflowDragResultInPlacement, resolveCanvasDropResult } from "@battersea/editor/hooks/use-dataflow-canvas-drop";
import {
  buildFlowNodePaletteDragId,
  createFlowNodeDragPayload,
  isFlowNodePaletteDragId,
  serialiseFlowNodeDragPayload
} from "@battersea/editor/core/flow-drag";

function buildNodeDefinition(): WireFlowNodeDefinition {
  return {
    action_ports: [],
    activation_parameters: [],
    class_name: "ChatAPI",
    dynamic_input_ports: [],
    dynamic_output_ports: [],
    handler_id: "primrose.chat-api",
    input_ports: [],
    interfaces: [],
    kind: "inline",
    long_description: "Talks to the model.",
    output_ports: [],
    parameters: [],
    signal_ports: [{
      name: "post_activate"
    }],
    short_description: "Talk to the model"
  };
}

test("flow node palette drag ids are explicit so canvas clicks are not treated as palette drops", () => {
  assert.equal(isFlowNodePaletteDragId(buildFlowNodePaletteDragId("ChatAPI")), true);
  assert.equal(isFlowNodePaletteDragId("flow-node:chat-api-1"), false);
  assert.equal(isFlowNodePaletteDragId("dataflow-canvas-dropzone"), false);
  assert.equal(isFlowNodePaletteDragId(42), false);
});

test("didDataflowPaletteDragMove rejects click-like zero displacement", () => {
  assert.equal(didDataflowPaletteDragMove({ delta: { x: 0, y: 0 } }), false);
  assert.equal(didDataflowPaletteDragMove({ delta: null }), false);
  assert.equal(didDataflowPaletteDragMove({ delta: { x: 6, y: 0 } }), true);
  assert.equal(didDataflowPaletteDragMove({ delta: { x: 0, y: -4 } }), true);
});

test("resolveCanvasDropResult reports invalid drag payloads", () => {
  const flowInstance = {
    screenToFlowPosition: (position: { x: number; y: number }) => position
  } as ReactFlowInstance<FlowStudioNode, FlowStudioEdge>;

  const result = resolveCanvasDropResult({
    clientX: 240,
    clientY: 180,
    dragPayload: "{invalid json",
    flowInstance,
    nextNodeIndex: 3,
    pendingDropTitle: "Chat API"
  });

  assert.deepEqual(result, {
    kind: "failure",
    reason: "invalid-payload",
    title: "Chat API"
  });
});

test("resolveCanvasDropResult creates and selects a dropped node at the resolved position", () => {
  const payload = serialiseFlowNodeDragPayload(createFlowNodeDragPayload(
    buildNodeDefinition()
  ));
  const flowInstance = {
    screenToFlowPosition: ({ x, y }: { x: number; y: number }) => ({
      x: x / 2,
      y: y / 2
    })
  } as ReactFlowInstance<FlowStudioNode, FlowStudioEdge>;

  const result = resolveCanvasDropResult({
    clientX: 240,
    clientY: 180,
    dragOffset: { x: 10, y: 15 },
    dragPayload: payload,
    flowInstance,
    nextNodeIndex: 3,
    pendingDropTitle: "Chat API"
  });

  assert.equal(result.kind, "success");
  assert.equal(result.nextNode.id, "chat-api-3");
  assert.equal(result.nextNode.data.instanceName, "Chat API-3");
  assert.deepEqual(result.nextNode.position, {
    x: 110,
    y: 75
  });
});

test("didDataflowDragResultInPlacement only treats valid canvas drops as successful placements", () => {
  assert.equal(didDataflowDragResultInPlacement({
    dropZonePresent: true,
    overId: "dataflow-canvas-dropzone",
    pointerPresent: true,
    validDragData: true
  }), true);

  assert.equal(didDataflowDragResultInPlacement({
    dropZonePresent: true,
    overId: "dataflow-canvas-dropzone",
    pointerPresent: false,
    validDragData: true
  }), false);

  assert.equal(didDataflowDragResultInPlacement({
    dropZonePresent: true,
    overId: "not-the-canvas",
    pointerPresent: true,
    validDragData: true
  }), false);
});
