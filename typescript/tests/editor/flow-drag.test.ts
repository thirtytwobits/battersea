/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises flow drag behaviour in the editor's dataflow workspace.
 */
import assert from "node:assert/strict";
import test from "node:test";
import type { FlowNodeDefinition as WireFlowNodeDefinition } from "@battersea/flow";
import {
  createDroppedFlowNode,
  createFlowNodeDragPayload,
  parseFlowNodeDragPayload,
  resolveDroppedFlowNodePosition,
  serialiseFlowNodeDragPayload
} from "@battersea/editor/core/flow-drag";

const USER_PROMPT_DEFINITION: WireFlowNodeDefinition = {
  action_ports: [{
    name: "clear_text"
  }],
  activation_parameters: ["prompt"],
  class_name: "UserPrompt",
  dynamic_input_ports: [],
  dynamic_output_ports: [],
  handler_id: "primrose.user-prompt",
  input_ports: [],
  interfaces: ["IFlowNodeActivate"],
  kind: "source",
  long_description: "Starts a flow by emitting exactly the prompt text entered for the activation.",
  output_ports: [
    {
      mode: "final_value",
      phase: "snapshot",
      kind: "output",
      name: "output",
      token_type: "prompt.fragment"
    }
  ],
  parameters: [
    {
      datatype: { kind: "string" },
      editor: { kind: "string" },
      name: "prompt"
    }
  ],
  signal_ports: [{
    name: "post_activate"
  }],
  short_description: "Reads the activation prompt into the flow."
};

const CONCATENATE_DEFINITION: WireFlowNodeDefinition = {
  action_ports: [],
  activation_parameters: [],
  class_name: "Concatenate",
  dynamic_input_ports: [
    {
      mode: "final_value",
      phase: "execution",
      count_parameter: "input_ports",
      name_template: "input-{index}",
      token_type: "prompt.fragment"
    }
  ],
  dynamic_output_ports: [],
  handler_id: "battersea.concatenate",
  input_ports: [],
  interfaces: [],
  kind: "inline",
  long_description: "Concatenates N prompt fragments with configurable ordering, whitespace cleanup, and output formatting.",
  output_ports: [
    {
      mode: "final_value",
      phase: "execution",
      kind: "output",
      name: "output",
      token_type: "prompt.fragment"
    }
  ],
  parameters: [
    {
      datatype: { kind: "int" },
      editor: {
        default_value: 2,
        kind: "input_port_count",
        min: 1
      },
      name: "input_ports"
    },
    {
      datatype: { kind: "string" },
      editor: {
        default_value: "descending",
        kind: "enum",
        values: ["descending", "ascending"]
      },
      name: "ordering"
    },
    {
      datatype: { kind: "string" },
      editor: {
        default_value: "inherit",
        kind: "enum",
        values: ["inherit", "trim", "preserve", "compact"]
      },
      name: "whitespace_mode"
    },
    {
      datatype: { kind: "string" },
      editor: {
        default_value: "inherit",
        kind: "enum",
        values: ["inherit", "markdown", "xml", "plain"]
      },
      name: "output_encoding"
    },
    {
      datatype: { kind: "string" },
      editor: {
        default_value: "blank_line",
        kind: "enum",
        values: ["comma", "blank_line", "newline", "space", "none"]
      },
      name: "plain_fragment_delimiter"
    },
    {
      datatype: { kind: "string" },
      editor: {
        default_value: "skip",
        kind: "enum",
        values: ["skip", "keep"]
      },
      name: "empty_input_rule"
    }
  ],
  signal_ports: [{
    name: "post_activate"
  }],
  short_description: "Combines inputs into a single output."
};

const CHARACTER_LIST_DEFINITION: WireFlowNodeDefinition = {
  action_ports: [],
  activation_parameters: [],
  class_name: "CharacterList",
  dynamic_input_ports: [],
  dynamic_output_ports: [],
  handler_id: "primrose.string-list",
  input_ports: [],
  interfaces: [],
  kind: "source",
  long_description: "Provides a rendered list of characters.",
  output_ports: [
    {
      mode: "final_value",
      phase: "snapshot",
      kind: "output",
      name: "output",
      token_type: "prompt.fragment"
    }
  ],
  parameters: [
    {
      controller: {
        default_value: [],
        kind: "list",
        source: "{character_picker()}"
      },
      datatype: {
        item_type: { kind: "string" },
        kind: "list"
      },
      editor: {
        default_value: [],
        kind: "list",
        source: "{character_picker()}"
      },
      name: "session_character_ids"
    },
    {
      controller: {
        default_value: "",
        kind: "string",
        source: "{markdown_editor()}"
      },
      datatype: { kind: "string" },
      editor: {
        default_value: "",
        kind: "string",
        source: "{markdown_editor()}"
      },
      name: "summary"
    }
  ],
  signal_ports: [{
    name: "post_activate"
  }],
  short_description: "Provides a rendered list of characters."
};

const RESPONSE_PARSER_DEFINITION: WireFlowNodeDefinition = {
  action_ports: [],
  activation_parameters: [],
  class_name: "ChatResponseParser",
  dynamic_input_ports: [],
  dynamic_output_ports: [],
  handler_id: "primrose.chat-response-parser",
  input_ports: [
    {
      mode: "final_value",
      phase: "execution",
      kind: "input",
      name: "response_stream",
      token_type: "chat.response_stream"
    },
    {
      mode: "final_value",
      phase: "execution",
      kind: "input",
      name: "response",
      token_type: "chat.response"
    }
  ],
  interfaces: [],
  kind: "hybrid",
  long_description: "Parses tagged chat responses into structured outputs.",
  output_ports: [
    {
      mode: "final_value",
      phase: "execution",
      display_class: "source",
      kind: "output",
      name: "format_rule",
      token_type: "prompt.fragment"
    },
    {
      mode: "final_value",
      phase: "execution",
      kind: "output",
      name: "user_response",
      token_type: "chat.response"
    }
  ],
  parameters: [],
  signal_ports: [{
    name: "post_activate"
  }],
  short_description: "Parses response envelopes."
};

test("flow node drag payload round-trips through serialisation", () => {
  const payload = createFlowNodeDragPayload(USER_PROMPT_DEFINITION, {
    x: 24,
    y: 18
  });
  const parsedPayload = parseFlowNodeDragPayload(serialiseFlowNodeDragPayload(payload));

  assert.ok(parsedPayload);
  assert.deepEqual(
    JSON.parse(serialiseFlowNodeDragPayload(parsedPayload)),
    JSON.parse(serialiseFlowNodeDragPayload(payload))
  );
});

test("parseFlowNodeDragPayload rejects invalid payloads", () => {
  assert.equal(parseFlowNodeDragPayload("{"), null);
  assert.equal(
    parseFlowNodeDragPayload(JSON.stringify({
      actionPorts: [],
      definitionName: "UserPrompt",
      hasController: "nope",
      inputPorts: ["output"],
      longDescription: "Long description",
      nodeClass: "transform",
      outputPorts: [],
      parameterValues: {},
      signalPorts: [],
      shortDescription: "Short description",
      title: "UserPrompt"
    })),
    null
  );
});

test("createFlowNodeDragPayload marks nodes with controlled parameters once", () => {
  const payload = createFlowNodeDragPayload(CHARACTER_LIST_DEFINITION);

  assert.equal(payload.hasController, true);
});

test("createFlowNodeDragPayload marks nodes with controller outputs", () => {
  const payload = createFlowNodeDragPayload({
    action_ports: [],
    activation_parameters: [],
    class_name: "SessionOutput",
    controller_outputs: [{
      kind: "response_stream",
      name: "response_stream"
    }],
    dynamic_input_ports: [],
    dynamic_output_ports: [],
    handler_id: "primrose.session-output",
    input_ports: [{
      mode: "final_value",
      phase: "execution",
      kind: "input",
      name: "response_stream",
      token_type: "chat.response_stream"
    }],
    interfaces: [],
    kind: "sink",
    long_description: "Writes the final output into the session transcript.",
    output_ports: [],
    parameters: [],
    signal_ports: [{
      name: "post_activate"
    }],
    short_description: "Writes the final output to the session."
  });

  assert.equal(payload.hasController, true);
});

test("createDroppedFlowNode creates a stable default node shape", () => {
  const node = createDroppedFlowNode({
    nextIndex: 4,
    payload: createFlowNodeDragPayload(CONCATENATE_DEFINITION),
    position: { x: 120, y: 240 }
  });

  assert.equal(node.id, "concatenate-4");
  assert.equal(node.className, "flow-studio-node flow-studio-node--inline");
  assert.deepEqual(node.position, { x: 120, y: 240 });
  assert.equal(node.type, "flowStudio");
  assert.equal(node.data.definitionName, "Concatenate");
  assert.equal(node.data.hasController, false);
  assert.equal(node.data.instanceName, "Concatenate-4");
  assert.equal(node.data.longDescription, "Concatenates N prompt fragments with configurable ordering, whitespace cleanup, and output formatting.");
  assert.equal(node.data.nodeClass, "inline");
  assert.deepEqual(node.data.parameterValues, {
    input_ports: 2,
    ordering: "descending",
    whitespace_mode: "inherit",
    output_encoding: "inherit",
    plain_fragment_delimiter: "blank_line",
    empty_input_rule: "skip"
  });
  assert.deepEqual(node.data.inputPorts.map((port) => typeof port === "string" ? port : port.id), [
    "input-0",
    "input-1"
  ]);
  assert.deepEqual(node.data.outputPorts.map((port) => typeof port === "string" ? port : port.id), [
    "output"
  ]);
  assert.deepEqual(node.data.signalPorts.map((port) => typeof port === "string" ? port : port.id), [
    "post_activate"
  ]);
  assert.equal(node.data.shortDescription, "Combines inputs into a single output.");
});

test("createDroppedFlowNode preserves hybrid node classes and resolved port display classes", () => {
  const node = createDroppedFlowNode({
    nextIndex: 14,
    payload: createFlowNodeDragPayload(RESPONSE_PARSER_DEFINITION),
    position: { x: 320, y: 180 }
  });

  assert.equal(node.className, "flow-studio-node flow-studio-node--hybrid");
  assert.equal(node.data.nodeClass, "hybrid");
  assert.deepEqual(
    node.data.outputPorts.map((port) => typeof port === "string" ? null : port.displayClass),
    ["source", "inline"]
  );
});

test("resolveDroppedFlowNodePosition preserves the drag grab offset", () => {
  assert.deepEqual(
    resolveDroppedFlowNodePosition({
      cursorPosition: { x: 320, y: 240 },
      payload: createFlowNodeDragPayload(USER_PROMPT_DEFINITION, {
        x: 28,
        y: 16
      })
    }),
    { x: 292, y: 224 }
  );
});
