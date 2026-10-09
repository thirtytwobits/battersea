/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises one-click variadic port deletion: shifting aliases down, severing
 * the deleted port's edge, remapping surviving edges, decrementing the count,
 * and guarding the count's lower bound.
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  canDeleteVariadicPort,
  deleteVariadicPortInWorkspace
} from "@battersea/editor/core/flow-port-delete";
import { resolveFlowStudioResolvedPorts } from "@battersea/editor/core/flow-node-ports";
import type { FlowStudioWorkspaceState } from "@battersea/editor/core/flow-persistence";

const CONCATENATE_DEFINITION = {
  action_ports: [],
  activation_parameters: [],
  class_name: "Concatenate",
  dynamic_input_ports: [{
    count_parameter: "input_ports",
    name_template: "input-{index}",
    token_type: "prompt.fragment"
  }],
  dynamic_output_ports: [],
  handler_id: "battersea.concatenate",
  input_ports: [],
  interfaces: [],
  kind: "inline" as const,
  long_description: "Concatenates input fragments.",
  output_ports: [{
    kind: "output" as const,
    name: "output",
    token_type: "prompt.fragment"
  }],
  parameters: [{
    datatype: { kind: "int" as const },
    editor: {
      default_value: 3,
      kind: "input_port_count" as const,
      min: 1
    },
    name: "input_ports"
  }],
  short_description: "Combines inputs.",
  signal_ports: []
};

function buildWorkspace(inputCount: number, options?: {
  portNames?: Record<string, string>;
  selectedPortId?: string;
}): FlowStudioWorkspaceState {
  const inputPorts = Array.from({ length: inputCount }, (_, index) => ({
    displayClass: "inline" as const,
    id: `input-${index}`,
    label: `Input ${index}`,
    side: "input" as const
  }));
  const edges = Array.from({ length: inputCount }, (_, index) => ({
    data: { kind: "token" as const, order: index },
    id: `edge-${index}`,
    source: "source-1",
    sourceHandle: "output-0",
    target: "concatenate-1",
    targetHandle: `input-${index}`
  }));

  return {
    baselineFlow: null,
    description: "",
    draftFlowKey: "draft",
    outputEncoding: "xml",
    plainFragmentDelimiter: "blank_line",
    whitespaceMode: "trim",
    edges,
    nodes: [{
      className: "flow-studio-node flow-studio-node--inline",
      data: {
        actionPorts: [],
        automationPorts: [],
        controllerPortPlacement: "default",
        definitionName: "Concatenate",
        inputPorts,
        instanceName: "Concatenate-1",
        longDescription: "Concatenates input fragments.",
        nodeClass: "inline",
        parameterValues: { input_ports: inputCount },
        outputPorts: [{ displayClass: "inline", id: "output", label: "Output", side: "output" }],
        portNames: options?.portNames ? { input: options.portNames } : undefined,
        signalPorts: [],
        shortDescription: "Combines inputs."
      },
      id: "concatenate-1",
      position: { x: 0, y: 0 },
      type: "flowStudio"
    }, {
      className: "flow-studio-node flow-studio-node--source",
      data: {
        actionPorts: [],
        automationPorts: [],
        controllerPortPlacement: "default",
        definitionName: "Source",
        inputPorts: [],
        instanceName: "Source-1",
        longDescription: "",
        nodeClass: "source",
        parameterValues: {},
        outputPorts: [{ displayClass: "source", id: "output", label: "Output", side: "output" }],
        portNames: undefined,
        signalPorts: [],
        shortDescription: ""
      },
      id: "source-1",
      position: { x: 0, y: 0 },
      type: "flowStudio"
    }],
    selectedFlowKey: "draft",
    selectedTarget: options?.selectedPortId
      ? { kind: "port", nodeId: "concatenate-1", portId: options.selectedPortId, side: "input" }
      : { kind: "none" },
    title: "Draft"
  };
}

test("deleting a middle variadic port renumbers survivors and decrements the count", () => {
  const workspace = buildWorkspace(3, { portNames: { "input-1": "Middle", "input-2": "Third" } });

  const next = deleteVariadicPortInWorkspace({
    definition: CONCATENATE_DEFINITION,
    nodeId: "concatenate-1",
    portId: "input-1",
    side: "input",
    workspace
  });

  const node = next.nodes.find((candidate) => candidate.id === "concatenate-1");
  assert.equal(node?.data.parameterValues.input_ports, 2);
  assert.deepEqual(
    resolveFlowStudioResolvedPorts(node?.data.inputPorts ?? [], "input").map((port) => port.id),
    ["input-0", "input-1"]
  );
  // "Third" (was input-2) shifts down onto the new input-1; "Middle" is gone.
  assert.deepEqual(node?.data.portNames?.input, { "input-1": "Third" });
});

test("deleting a port severs its edge and shifts higher edges down", () => {
  const workspace = buildWorkspace(3);

  const next = deleteVariadicPortInWorkspace({
    definition: CONCATENATE_DEFINITION,
    nodeId: "concatenate-1",
    portId: "input-1",
    side: "input",
    workspace
  });

  const handles = next.edges
    .filter((edge) => edge.target === "concatenate-1")
    .map((edge) => edge.targetHandle)
    .sort();
  // edge-1 (input-1) dropped; edge-0 stays input-0; edge-2 remaps to input-1.
  assert.deepEqual(handles, ["input-0", "input-1"]);
  assert.equal(next.edges.some((edge) => edge.id === "edge-1"), false);
});

test("a port selection is dropped back to the node when its port is deleted", () => {
  const workspace = buildWorkspace(3, { selectedPortId: "input-1" });

  const next = deleteVariadicPortInWorkspace({
    definition: CONCATENATE_DEFINITION,
    nodeId: "concatenate-1",
    portId: "input-1",
    side: "input",
    workspace
  });

  assert.deepEqual(next.selectedTarget, { kind: "node", nodeId: "concatenate-1" });
});

test("deletion is refused at the count's minimum", () => {
  const workspace = buildWorkspace(1);

  const next = deleteVariadicPortInWorkspace({
    definition: CONCATENATE_DEFINITION,
    nodeId: "concatenate-1",
    portId: "input-0",
    side: "input",
    workspace
  });

  assert.equal(next, workspace);
});

test("fixed (non-variadic) ports cannot be deleted", () => {
  const workspace = buildWorkspace(3);

  assert.equal(
    canDeleteVariadicPort({
      definition: CONCATENATE_DEFINITION,
      parameterValues: { input_ports: 3 },
      portId: "output",
      side: "output"
    }),
    false
  );

  const next = deleteVariadicPortInWorkspace({
    definition: CONCATENATE_DEFINITION,
    nodeId: "concatenate-1",
    portId: "output",
    side: "output",
    workspace
  });
  assert.equal(next, workspace);
});

test("canDeleteVariadicPort reflects the count's lower bound", () => {
  assert.equal(
    canDeleteVariadicPort({
      definition: CONCATENATE_DEFINITION,
      parameterValues: { input_ports: 3 },
      portId: "input-1",
      side: "input"
    }),
    true
  );
  assert.equal(
    canDeleteVariadicPort({
      definition: CONCATENATE_DEFINITION,
      parameterValues: { input_ports: 1 },
      portId: "input-0",
      side: "input"
    }),
    false
  );
});
