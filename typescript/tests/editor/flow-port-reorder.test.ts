/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises committed port reorder mutations in the editor's dataflow workspace.
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  reorderNodePortsInWorkspace,
  setNodeControllerPortPlacementInWorkspace,
  swapNodeControllerPortPlacementInWorkspace
} from "@battersea/editor/core/flow-port-reorder";
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

test("reorderNodePortsInWorkspace renames dynamic input port ids so the saved flow's logical order matches the visual order", () => {
  // Three edges fan into the Concatenate node, each one wired to a distinct
  // dynamic input port. The user drags input-0 to the last visual position.
  // Under Version A (visual == logical), the ports get renumbered so that
  // the content previously on input-0 lives on input-2 in the saved flow,
  // and the runtime — which orders by numeric suffix — places it last.
  const workspace: FlowStudioWorkspaceState = {
    baselineFlow: null,
    description: "",
    draftFlowKey: "draft",
    outputEncoding: "xml",
    plainFragmentDelimiter: "blank_line",
    whitespaceMode: "trim",
    edges: [
      {
        data: { kind: "token", order: 0 },
        id: "edge-to-0",
        source: "source-a",
        sourceHandle: "output-0",
        target: "concatenate-1",
        targetHandle: "input-0"
      },
      {
        data: { kind: "token", order: 0 },
        id: "edge-to-1",
        source: "source-b",
        sourceHandle: "output-0",
        target: "concatenate-1",
        targetHandle: "input-1"
      },
      {
        data: { kind: "token", order: 0 },
        id: "edge-to-2",
        source: "source-c",
        sourceHandle: "output-0",
        target: "concatenate-1",
        targetHandle: "input-2"
      }
    ],
    nodes: [{
      className: "flow-studio-node flow-studio-node--inline",
      data: {
        actionPorts: [],
        automationPorts: [],
        controllerPortPlacement: "default",
        definitionName: "Concatenate",
        inputPorts: [
          { displayClass: "inline", id: "input-0", label: "Scene", side: "input" },
          { displayClass: "inline", id: "input-1", label: "Context", side: "input" },
          { displayClass: "inline", id: "input-2", label: "Notes", side: "input" }
        ],
        instanceName: "Concatenate-1",
        longDescription: "Concatenates input fragments.",
        nodeClass: "inline",
        parameterValues: {
          input_ports: 3
        },
        outputPorts: [{
          displayClass: "inline",
          id: "output",
          label: "Output",
          side: "output"
        }],
        portNames: {
          input: {
            "input-0": "Scene",
            "input-1": "Context",
            "input-2": "Notes"
          }
        },
        portParameterValues: {
          input: {
            "input-0": { array_delimiter: "\n--SCENE--\n" },
            "input-2": { array_delimiter: "\n--NOTES--\n" }
          }
        },
        signalPorts: [],
        shortDescription: "Combines inputs."
      },
      id: "concatenate-1",
      position: { x: 0, y: 0 },
      type: "flowStudio"
    }],
    selectedFlowKey: "draft",
    selectedTarget: { kind: "none" },
    title: "Draft"
  };

  const reordered = reorderNodePortsInWorkspace({
    activePortId: "input-0",
    definition: CONCATENATE_DEFINITION,
    nodeId: "concatenate-1",
    previewIndex: 2,
    side: "input",
    workspace
  });

  // Visual position 0 now holds what was "Context" (originally input-1) — the
  // runtime sorts dynamic ports by their numeric suffix, so position 0 must
  // be the new logical "input-0". The port_order field is redundant once
  // the renumber takes effect: visual order == numeric-suffix order.
  assert.equal(reordered.nodes[0]?.data.portOrder, undefined);

  // Aliases follow content. "Scene" was dragged to last; it should now be
  // keyed under input-2. "Context" (was input-1) becomes input-0; "Notes"
  // (was input-2) becomes input-1.
  assert.deepEqual(
    reordered.nodes[0]?.data.portNames?.input,
    {
      "input-0": "Context",
      "input-1": "Notes",
      "input-2": "Scene"
    }
  );

  // Per-port parameters follow the content too. The Scene delimiter rides
  // along to its new id (input-2); the Notes delimiter rides to input-1.
  assert.deepEqual(
    reordered.nodes[0]?.data.portParameterValues?.input,
    {
      "input-1": { array_delimiter: "\n--NOTES--\n" },
      "input-2": { array_delimiter: "\n--SCENE--\n" }
    }
  );

  // Edges follow their attached port. The edge that targeted "Scene"
  // (originally input-0) now targets input-2; "Context" is at input-0 and
  // "Notes" at input-1.
  const edgeById = new Map(reordered.edges.map((edge) => [edge.id, edge.targetHandle]));
  assert.equal(edgeById.get("edge-to-0"), "input-2");
  assert.equal(edgeById.get("edge-to-1"), "input-0");
  assert.equal(edgeById.get("edge-to-2"), "input-1");
});

test("swapNodeControllerPortPlacementInWorkspace flips controller ports between top and bottom without touching edges", () => {
  const workspace: FlowStudioWorkspaceState = {
    baselineFlow: null,
    description: "",
    draftFlowKey: "draft",
    outputEncoding: "xml",
    plainFragmentDelimiter: "blank_line",
    whitespaceMode: "trim",
    edges: [{
      data: {
        kind: "signal",
        order: 0
      },
      id: "edge-1",
      source: "prompt-1",
      sourceHandle: "signal-0",
      target: "prompt-2",
      targetHandle: "action-0"
    }],
    nodes: [{
      className: "flow-studio-node flow-studio-node--source",
      data: {
        actionPorts: [{
          displayClass: "source",
          id: "clear_text",
          label: "Clear Text",
          side: "action"
        }],
        automationPorts: [],
        controllerPortPlacement: "default",
        definitionName: "UserPrompt",
        inputPorts: [],
        instanceName: "Prompt-1",
        longDescription: "",
        nodeClass: "source",
        parameterValues: {},
        outputPorts: [],
        portNames: undefined,
        signalPorts: [{
          displayClass: "source",
          id: "post_activate",
          label: "Post Activate",
          side: "signal"
        }],
        shortDescription: ""
      },
      id: "prompt-1",
      position: { x: 0, y: 0 },
      type: "flowStudio"
    }, {
      className: "flow-studio-node flow-studio-node--source",
      data: {
        actionPorts: [{
          displayClass: "source",
          id: "clear_text",
          label: "Clear Text",
          side: "action"
        }],
        automationPorts: [],
        controllerPortPlacement: "default",
        definitionName: "UserPrompt",
        inputPorts: [],
        instanceName: "Prompt-2",
        longDescription: "",
        nodeClass: "source",
        parameterValues: {},
        outputPorts: [],
        portNames: undefined,
        signalPorts: [{
          displayClass: "source",
          id: "post_activate",
          label: "Post Activate",
          side: "signal"
        }],
        shortDescription: ""
      },
      id: "prompt-2",
      position: { x: 120, y: 0 },
      type: "flowStudio"
    }],
    selectedFlowKey: "draft",
    selectedTarget: { kind: "none" },
    title: "Draft"
  };

  const swapped = swapNodeControllerPortPlacementInWorkspace({
    definition: {
      action_ports: [{ name: "clear_text" }],
      activation_parameters: ["prompt"],
      class_name: "UserPrompt",
      dynamic_input_ports: [],
      dynamic_output_ports: [],
      handler_id: "primrose.user-prompt",
      input_ports: [],
      interfaces: ["IFlowNodeActivate"],
      kind: "source",
      long_description: "",
      output_ports: [],
      parameters: [{
        datatype: { kind: "string" as const },
        editor: { kind: "string" as const },
        name: "prompt"
      }],
      short_description: "",
      signal_ports: [{ name: "post_activate" }]
    },
    nodeId: "prompt-1",
    workspace
  });

  assert.equal(swapped.nodes[0]?.data.controllerPortPlacement, "swapped");
  assert.equal(swapped.nodes[1]?.data.controllerPortPlacement, "default");
  assert.equal(swapped.edges[0]?.sourceHandle, "signal-0");
  assert.equal(swapped.edges[0]?.targetHandle, "action-0");
});

test("setNodeControllerPortPlacementInWorkspace stores logic visual direction as layout data without touching parameters", () => {
  const workspace: FlowStudioWorkspaceState = {
    baselineFlow: null,
    description: "",
    draftFlowKey: "draft",
    outputEncoding: "xml",
    plainFragmentDelimiter: "blank_line",
    whitespaceMode: "trim",
    edges: [{
      data: {
        kind: "signal",
        order: 0
      },
      id: "edge-1",
      source: "and-1",
      sourceHandle: "signal-0",
      target: "sink-1",
      targetHandle: "action-0"
    }],
    nodes: [{
      className: "flow-studio-node flow-studio-node--logic",
      data: {
        actionPorts: ["input-0", "input-1"],
        automationPorts: [],
        controllerPortPlacement: "default",
        definitionName: "AndGate",
        inputPorts: [],
        instanceName: "AND-1",
        longDescription: "",
        nodeClass: "logic",
        outputPorts: [],
        parameterValues: { not: true },
        portParameterValues: {
          input: {
            "input-0": { note: "kept" }
          }
        },
        signalPorts: ["output"],
        shortDescription: ""
      },
      id: "and-1",
      position: { x: 0, y: 0 },
      type: "flowStudio"
    }],
    selectedFlowKey: "draft",
    selectedTarget: { kind: "none" },
    title: "Draft"
  };

  const next = setNodeControllerPortPlacementInWorkspace({
    definition: {
      action_ports: [],
      activation_parameters: [],
      class_name: "AndGate",
      dynamic_action_ports: [{
        count_parameter: "input_ports",
        name_template: "input-{index}"
      }],
      dynamic_input_ports: [],
      dynamic_output_ports: [],
      dynamic_signal_ports: [],
      handler_id: "battersea.logic.and",
      input_ports: [],
      interfaces: [],
      kind: "logic",
      long_description: "",
      output_ports: [],
      parameters: [{
        datatype: { kind: "int" as const },
        editor: {
          default_value: 2,
          kind: "input_port_count" as const,
          min: 2
        },
        name: "input_ports"
      }, {
        datatype: { kind: "boolean" as const },
        editor: {
          default_value: false,
          kind: "boolean" as const
        },
        name: "not"
      }],
      short_description: "",
      signal_ports: [{ name: "output" }]
    },
    nodeId: "and-1",
    placement: "swapped",
    workspace
  });

  assert.equal(next.nodes[0]?.data.controllerPortPlacement, "swapped");
  assert.deepEqual(next.nodes[0]?.data.parameterValues, { not: true });
  assert.deepEqual(
    next.nodes[0]?.data.portParameterValues?.input,
    workspace.nodes[0]?.data.portParameterValues?.input
  );
  assert.deepEqual(next.edges, workspace.edges);
});
