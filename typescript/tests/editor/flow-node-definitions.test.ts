/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises flow node definitions behaviour in the editor's dataflow workspace.
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  buildResolvedNodeData,
  definitionHasControllers,
  formatFlowDefinitionTitle
} from "@battersea/editor/core/flow-node-definitions";

test("formatFlowDefinitionTitle humanises node class names for detail-pane subtitles", () => {
  assert.equal(formatFlowDefinitionTitle("ChatAPI"), "Chat API");
  assert.equal(formatFlowDefinitionTitle("UserPrompt"), "User Prompt");
});

test("definitionHasControllers treats controller actions as UI controllers", () => {
  assert.equal(definitionHasControllers({
    action_ports: [],
    activation_parameters: [],
    class_name: "SessionCompactionControl",
    controller_actions: [{
      kind: "compact_now",
      name: "compact_now"
    }],
    dynamic_input_ports: [],
    dynamic_output_ports: [],
    handler_id: "primrose.session-compaction-control",
    input_ports: [],
    interfaces: [],
    kind: "control",
    long_description: "Queues session memory compaction.",
    output_ports: [],
    parameters: [],
    short_description: "Session compaction control.",
    signal_ports: []
  }), true);
});

test("buildResolvedNodeData applies node-level port aliases while keeping canonical ids", () => {
  const data = buildResolvedNodeData({
    definition: {
      action_ports: [{ name: "commit" }],
      activation_parameters: [],
      class_name: "Concatenate",
      dynamic_input_ports: [{
        mode: "final_value",
        phase: "execution",
        count_parameter: "input_ports",
        name_template: "input-{index}",
        token_type: "prompt.fragment"
      }],
      dynamic_output_ports: [],
      handler_id: "battersea.concatenate",
      input_ports: [],
      interfaces: [],
      kind: "inline",
      long_description: "Concatenate inputs.",
      output_ports: [{
        mode: "final_value",
        phase: "execution",
        kind: "output",
        name: "output",
        token_type: "prompt.fragment"
      }],
      parameters: [{
        datatype: { kind: "int" },
        editor: {
          default_value: 2,
          kind: "input_port_count",
          min: 1
        },
        name: "input_ports"
      }],
      short_description: "Combines inputs.",
      signal_ports: [{ name: "post_activate" }]
    },
    instanceName: "Concatenate-1",
    parameterValues: { input_ports: 2 },
    portNames: {
      input: {
        "input-0": "Prompt",
        "input-1": "Context"
      },
      output: {
        output: "Combined Prompt"
      }
    }
  });

  assert.deepEqual(
    data.inputPorts.map((port) => ({ id: typeof port === "string" ? port : port.id, label: typeof port === "string" ? port : port.label })),
    [
      { id: "input-0", label: "Prompt" },
      { id: "input-1", label: "Context" }
    ]
  );
  assert.deepEqual(
    data.outputPorts.map((port) => ({ id: typeof port === "string" ? port : port.id, label: typeof port === "string" ? port : port.label })),
    [{ id: "output", label: "Combined Prompt" }]
  );
});

test("buildResolvedNodeData preserves accepted token types on input ports", () => {
  const data = buildResolvedNodeData({
    definition: {
      action_ports: [],
      activation_parameters: [],
      class_name: "Concatenate",
      dynamic_input_ports: [{
        mode: "final_value",
        phase: "execution",
        accepted_token_types: ["prompt.fragment", "prompt.fragmentArray"],
        count_parameter: "input_ports",
        name_template: "input-{index}",
        token_type: "prompt.fragment"
      }],
      dynamic_output_ports: [],
      handler_id: "battersea.concatenate",
      input_ports: [],
      interfaces: [],
      kind: "inline",
      long_description: "Concatenate inputs.",
      output_ports: [],
      parameters: [{
        datatype: { kind: "int" },
        editor: {
          default_value: 1,
          kind: "input_port_count",
          min: 1
        },
        name: "input_ports"
      }],
      short_description: "Combines inputs.",
      signal_ports: []
    },
    instanceName: "Concatenate-1",
    parameterValues: { input_ports: 1 }
  });

  assert.deepEqual(
    typeof data.inputPorts[0] === "string" ? undefined : data.inputPorts[0]?.acceptedTokenTypes,
    ["prompt.fragment", "prompt.fragmentArray"]
  );
});

test("buildResolvedNodeData preserves fixed input and output port parameters", () => {
  const formatterParameter = {
    controller: { kind: "text_input" },
    datatype: { kind: "string" },
    editor: {
      default_value: "Default format",
      kind: "text"
    },
    name: "formatter"
  } as const;
  const inputHintParameter = {
    datatype: { kind: "string" },
    editor: {
      default_value: "Default input hint",
      kind: "text"
    },
    name: "hint"
  } as const;

  const data = buildResolvedNodeData({
    definition: {
      action_ports: [],
      activation_parameters: [],
      class_name: "ChatResponseParser",
      dynamic_input_ports: [],
      dynamic_output_ports: [],
      handler_id: "primrose.chat-response-parser",
      input_ports: [{
        mode: "final_value",
        phase: "execution",
        kind: "input",
        name: "response",
        parameters: [inputHintParameter],
        token_type: "chat.response"
      }],
      interfaces: [],
      kind: "inline",
      long_description: "Parses responses.",
      output_ports: [{
        mode: "final_value",
        phase: "execution",
        kind: "output",
        name: "format_rule",
        parameters: [formatterParameter],
        token_type: "prompt.fragment"
      }],
      parameters: [],
      short_description: "Parse chat responses.",
      signal_ports: []
    },
    instanceName: "Chat Response Parser-1",
    parameterValues: {}
  });

  assert.deepEqual(
    typeof data.inputPorts[0] === "string" ? undefined : data.inputPorts[0]?.parameters,
    [inputHintParameter]
  );
  assert.deepEqual(
    typeof data.outputPorts[0] === "string" ? undefined : data.outputPorts[0]?.parameters,
    [formatterParameter]
  );
});

test("buildResolvedNodeData expands dynamic output ports from output_port_count parameters", () => {
  const data = buildResolvedNodeData({
    definition: {
      action_ports: [],
      activation_parameters: [],
      class_name: "Multiplexer",
      dynamic_input_ports: [],
      dynamic_output_ports: [{
        mode: "final_value",
        phase: "execution",
        count_parameter: "output_ports",
        name_template: "output-{index}",
        token_type: "prompt.fragment"
      }],
      handler_id: "battersea.multiplexer",
      input_ports: [{
        mode: "final_value",
        phase: "execution",
        kind: "input",
        name: "input",
        token_type: "prompt.fragment"
      }],
      interfaces: [],
      kind: "inline",
      long_description: "Duplicates one prompt fragment onto many outputs.",
      output_ports: [],
      parameters: [{
        datatype: { kind: "int" },
        editor: {
          default_value: 2,
          kind: "output_port_count",
          min: 1
        },
        name: "output_ports"
      }],
      short_description: "Duplicates one input to many outputs.",
      signal_ports: []
    },
    instanceName: "Multiplexer-1",
    parameterValues: { output_ports: 3 }
  });

  assert.deepEqual(
    data.outputPorts.map((port) => ({ id: typeof port === "string" ? port : port.id, label: typeof port === "string" ? port : port.label })),
    [
      { id: "output-0", label: "Output 0" },
      { id: "output-1", label: "Output 1" },
      { id: "output-2", label: "Output 2" }
    ]
  );
});

test("buildResolvedNodeData expands dynamic logic action and signal ports", () => {
  const data = buildResolvedNodeData({
    definition: {
      action_ports: [{
        name: "enable"
      }],
      activation_parameters: [],
      class_name: "SignalMultiplexer",
      dynamic_action_ports: [{
        count_parameter: "input_ports",
        name_template: "input-{index}"
      }],
      dynamic_input_ports: [],
      dynamic_output_ports: [],
      dynamic_signal_ports: [{
        count_parameter: "output_ports",
        name_template: "output-{index}"
      }],
      handler_id: "battersea.logic.multiplexer",
      input_ports: [],
      interfaces: [],
      kind: "logic",
      long_description: "Fans one signal out to many signal outputs.",
      output_ports: [],
      parameters: [{
        datatype: { kind: "int" },
        editor: {
          default_value: 2,
          kind: "input_port_count",
          min: 1
        },
        name: "input_ports"
      }, {
        datatype: { kind: "int" },
        editor: {
          default_value: 2,
          kind: "output_port_count",
          min: 1
        },
        name: "output_ports"
      }],
      short_description: "Signal multiplexer.",
      signal_ports: []
    },
    instanceName: "Signal Multiplexer-1",
    parameterValues: { input_ports: 2, output_ports: 2 }
  });

  assert.deepEqual(
    data.actionPorts.map((port) => ({ id: typeof port === "string" ? port : port.id, label: typeof port === "string" ? port : port.label })),
    [
      { id: "enable", label: "Enable" },
      { id: "input-0", label: "Input 0" },
      { id: "input-1", label: "Input 1" }
    ]
  );
  assert.deepEqual(
    data.signalPorts.map((port) => ({ id: typeof port === "string" ? port : port.id, label: typeof port === "string" ? port : port.label })),
    [
      { id: "output-0", label: "Output 0" },
      { id: "output-1", label: "Output 1" }
    ]
  );
});

test("buildResolvedNodeData applies saved port order without changing canonical ids", () => {
  const data = buildResolvedNodeData({
    definition: {
      action_ports: [],
      activation_parameters: [],
      class_name: "Concatenate",
      dynamic_input_ports: [{
        mode: "final_value",
        phase: "execution",
        count_parameter: "input_ports",
        name_template: "input-{index}",
        token_type: "prompt.fragment"
      }],
      dynamic_output_ports: [],
      handler_id: "battersea.concatenate",
      input_ports: [],
      interfaces: [],
      kind: "inline",
      long_description: "Concatenate inputs.",
      output_ports: [{
        mode: "final_value",
        phase: "execution",
        kind: "output",
        name: "output",
        token_type: "prompt.fragment"
      }],
      parameters: [{
        datatype: { kind: "int" },
        editor: {
          default_value: 3,
          kind: "input_port_count",
          min: 1
        },
        name: "input_ports"
      }],
      short_description: "Combines inputs.",
      signal_ports: []
    },
    instanceName: "Concatenate-1",
    parameterValues: { input_ports: 3 },
    portOrder: {
      input: ["input-2", "input-0", "input-1"]
    }
  });

  assert.deepEqual(
    data.inputPorts.map((port) => typeof port === "string" ? port : port.id),
    ["input-2", "input-0", "input-1"]
  );
  assert.deepEqual(data.portOrder?.input, ["input-2", "input-0", "input-1"]);
  assert.equal(data.portOrder?.output, undefined);
});

test("buildResolvedNodeData preserves reordered dynamic-port survivors and appends new ports in default order", () => {
  const data = buildResolvedNodeData({
    definition: {
      action_ports: [],
      activation_parameters: [],
      class_name: "Concatenate",
      dynamic_input_ports: [{
        mode: "final_value",
        phase: "execution",
        count_parameter: "input_ports",
        name_template: "input-{index}",
        token_type: "prompt.fragment"
      }],
      dynamic_output_ports: [],
      handler_id: "battersea.concatenate",
      input_ports: [],
      interfaces: [],
      kind: "inline",
      long_description: "Concatenate inputs.",
      output_ports: [{
        mode: "final_value",
        phase: "execution",
        kind: "output",
        name: "output",
        token_type: "prompt.fragment"
      }],
      parameters: [{
        datatype: { kind: "int" },
        editor: {
          default_value: 3,
          kind: "input_port_count",
          min: 1
        },
        name: "input_ports"
      }],
      short_description: "Combines inputs.",
      signal_ports: []
    },
    instanceName: "Concatenate-1",
    parameterValues: { input_ports: 4 },
    portOrder: {
      input: ["input-2", "input-0", "input-1"]
    }
  });

  assert.deepEqual(
    data.inputPorts.map((port) => typeof port === "string" ? port : port.id),
    ["input-2", "input-0", "input-1", "input-3"]
  );
  assert.deepEqual(data.portOrder?.input, ["input-2", "input-0", "input-1", "input-3"]);
});
