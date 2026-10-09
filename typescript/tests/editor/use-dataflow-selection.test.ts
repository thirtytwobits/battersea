/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises dataflow canvas selection helper behaviour in the editor's dataflow workspace.
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  buildRenderedFlowStudioNodes,
  findSelectedFlowStudioNode
} from "@battersea/editor/core/dataflow-workspace-selectors";
import type { FlowStudioWorkspaceState } from "@battersea/editor/core/flow-persistence";
import { resolveSelectionKeyboardAction, selectFlowInWorkspace, shouldClearSelectionForEdgeChanges, shouldDismissNodeContextMenu } from "@battersea/editor/hooks/use-dataflow-selection";

test("resolveSelectionKeyboardAction clears selection on escape outside editable fields", () => {
  assert.equal(resolveSelectionKeyboardAction({
    hasNodeContextMenu: false,
    isEditableTarget: false,
    key: "Escape",
    repeat: false,
    selectedTarget: { kind: "node", nodeId: "chat-api-1" }
  }), "clear-selection");
});

test("resolveSelectionKeyboardAction removes the selected node on delete outside editable fields", () => {
  assert.equal(resolveSelectionKeyboardAction({
    hasNodeContextMenu: false,
    isEditableTarget: false,
    key: "Delete",
    repeat: false,
    selectedTarget: { kind: "node", nodeId: "chat-api-1" }
  }), "remove-node-selection");
});

test("resolveSelectionKeyboardAction treats flow selection as clearable but not removable", () => {
  assert.equal(resolveSelectionKeyboardAction({
    hasNodeContextMenu: false,
    isEditableTarget: false,
    key: "Escape",
    repeat: false,
    selectedTarget: { kind: "flow" }
  }), "clear-selection");
  assert.equal(resolveSelectionKeyboardAction({
    hasNodeContextMenu: false,
    isEditableTarget: false,
    key: "Delete",
    repeat: false,
    selectedTarget: { kind: "flow" }
  }), "ignore");
});

test("flow selection does not select or highlight a node", () => {
  const workspace: FlowStudioWorkspaceState = {
    baselineFlow: null,
    description: "",
    draftFlowKey: "",
    outputEncoding: "xml",
    plainFragmentDelimiter: "blank_line",
    whitespaceMode: "trim",
    edges: [],
    nodes: [{
      data: {
        actionPorts: [],
        automationPorts: [],
        definitionName: "ChatAPI",
        inputPorts: [],
        instanceName: "Chat API-1",
        longDescription: "",
        nodeClass: "inline",
        outputPorts: [],
        parameterValues: {},
        signalPorts: [],
        shortDescription: ""
      },
      id: "chat-api-1",
      position: { x: 0, y: 0 },
      type: "flowStudio"
    }],
    selectedFlowKey: "default-session-activation",
    selectedTarget: { kind: "flow" },
    title: "Default Session Activation"
  };

  assert.equal(findSelectedFlowStudioNode(workspace), null);
  assert.deepEqual(buildRenderedFlowStudioNodes(workspace).map((node) => node.selected), [false]);
});

test("selectFlowInWorkspace selects the flow document for empty canvas clicks", () => {
  const workspace: FlowStudioWorkspaceState = {
    baselineFlow: null,
    description: "",
    draftFlowKey: "",
    outputEncoding: "xml",
    plainFragmentDelimiter: "blank_line",
    whitespaceMode: "trim",
    edges: [],
    nodes: [],
    selectedFlowKey: "default-session-activation",
    selectedTarget: {
      kind: "node",
      nodeId: "chat-api-1"
    },
    title: "Default Session Activation"
  };

  assert.deepEqual(selectFlowInWorkspace(workspace).selectedTarget, { kind: "flow" });
});

test("resolveSelectionKeyboardAction removes the selected edge waypoint before node selection", () => {
  assert.equal(resolveSelectionKeyboardAction({
    hasNodeContextMenu: false,
    isEditableTarget: false,
    key: "Backspace",
    repeat: false,
    selectedEdgeWaypoint: {
      edgeId: "edge-1",
      waypointIndex: 1
    },
    selectedTarget: { kind: "node", nodeId: "chat-api-1" }
  }), "remove-waypoint-selection");
});

test("resolveSelectionKeyboardAction removes the selected edge bridge before node selection", () => {
  assert.equal(resolveSelectionKeyboardAction({
    hasNodeContextMenu: false,
    isEditableTarget: false,
    key: "Delete",
    repeat: false,
    selectedEdgeBridge: {
      bridgeIndex: 0,
      edgeId: "edge-1"
    },
    selectedTarget: { kind: "node", nodeId: "chat-api-1" }
  }), "remove-bridge-selection");
});

test("resolveSelectionKeyboardAction maps ArrowUp to move the selected port earlier", () => {
  assert.equal(resolveSelectionKeyboardAction({
    hasNodeContextMenu: false,
    isEditableTarget: false,
    key: "ArrowUp",
    repeat: false,
    selectedTarget: {
      kind: "port",
      nodeId: "chat-api-1",
      portId: "response",
      side: "output"
    }
  }), "move-port-toward-start");
});

test("resolveSelectionKeyboardAction maps ArrowDown to move the selected port later", () => {
  assert.equal(resolveSelectionKeyboardAction({
    hasNodeContextMenu: false,
    isEditableTarget: false,
    key: "ArrowDown",
    repeat: false,
    selectedTarget: {
      kind: "port",
      nodeId: "chat-api-1",
      portId: "response",
      side: "output"
    }
  }), "move-port-toward-end");
});

test("shouldClearSelectionForEdgeChanges clears the selected node when an edge becomes selected", () => {
  assert.equal(shouldClearSelectionForEdgeChanges([
    {
      id: "edge-1",
      selected: true,
      type: "select"
    }
  ]), true);
});

test("shouldClearSelectionForEdgeChanges ignores edge updates that do not select an edge", () => {
  assert.equal(shouldClearSelectionForEdgeChanges([
    {
      id: "edge-1",
      type: "remove"
    }
  ]), false);
  assert.equal(shouldClearSelectionForEdgeChanges([
    {
      id: "edge-1",
      selected: false,
      type: "select"
    }
  ]), false);
});

test("shouldDismissNodeContextMenu ignores pointer events inside the context menu", () => {
  assert.equal(shouldDismissNodeContextMenu({
    closest: (selector: string) => selector === ".flow-studio-node-context-menu"
      ? {}
      : null
  } as unknown as EventTarget), false);
  assert.equal(shouldDismissNodeContextMenu({
    closest: () => null
  } as unknown as EventTarget), true);
});
