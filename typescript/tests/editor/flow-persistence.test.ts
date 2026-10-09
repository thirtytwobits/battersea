/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises flow persistence behaviour in the editor's dataflow workspace.
 */
import { createFlowExecutionPolicy } from "@battersea/flow";
import assert from "node:assert/strict";
import test from "node:test";
import type { FlowDocument as WireFlowDocument, FlowNodeDefinition as WireFlowNodeDefinition } from "@battersea/flow";
import {
  areFlowDocumentsEqual,
  buildDefaultFlowWorkspace,
  buildFlowDocumentFromWorkspace,
  buildFlowSaveDocument,
  buildFlowValidationDocument,
  buildFlowWorkspaceFromDocument,
  findNextFlowNodeIndex,
  normalizeFlowWorkspaceState,
  reconcileFlowWorkspaceDefinitions,
  renameNodeInstanceInWorkspace,
  removeNodeFromWorkspace,
  FLOW_EDITOR_LAYOUT_KEY,
  slugifyFlowKey
} from "@battersea/editor/core/flow-persistence";
import { createFlowDefinitionLookup } from "@battersea/editor/core/flow-node-definitions";

function assertRecord(value: unknown): asserts value is Record<string, unknown> {
  assert.equal(typeof value, "object");
  assert.notEqual(value, null);
  assert.equal(Array.isArray(value), false);
}

/**
 * Reaches this editor's canvas state through the client-layout namespace.
 *
 * A flow document's `layout` is a map of writer namespace to opaque blob, so
 * asserting on the raw top level would pass for a layout written by any client.
 */
function assertFlowEditorCanvas(layout: unknown): Record<string, unknown> {
  assertRecord(layout);
  const envelope = layout[FLOW_EDITOR_LAYOUT_KEY];
  assertRecord(envelope);
  const canvas = envelope.canvas;
  assertRecord(canvas);
  return canvas;
}

/** Wraps a canvas blob in this editor's client-layout namespace. */
function flowEditorLayout(canvasLayout: Record<string, unknown>): Record<string, unknown> {
  return { [FLOW_EDITOR_LAYOUT_KEY]: canvasLayout };
}

const DEFINITIONS: WireFlowNodeDefinition[] = [
  {
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
  },
  {
    action_ports: [],
    activation_parameters: [],
    class_name: "AndGate",
    dynamic_action_ports: [
      {
        count_parameter: "input_ports",
        name_template: "input-{index}"
      }
    ],
    dynamic_input_ports: [],
    dynamic_output_ports: [],
    handler_id: "battersea.logic.and",
    input_ports: [],
    interfaces: [],
    kind: "logic",
    long_description: "Signal-only logic gate that emits after every input signal is present.",
    output_ports: [],
    parameters: [
      {
        datatype: { kind: "int" },
        editor: {
          default_value: 2,
          kind: "input_port_count",
          min: 2
        },
        name: "input_ports"
      },
      {
        datatype: { kind: "boolean" },
        editor: {
          default_value: false,
          kind: "boolean"
        },
        name: "not"
      }
    ],
    signal_ports: [{
      name: "output"
    }],
    short_description: "Emit when every input signal is present."
  },
  {
    action_ports: [],
    activation_parameters: [],
    class_name: "RulesQuery",
    dynamic_input_ports: [],
    dynamic_output_ports: [],
    handler_id: "primrose.query",
    input_ports: [],
    interfaces: [],
    kind: "source",
    long_description: "Emits a prompt fragment from story-rule query results.",
    output_ports: [
      {
        mode: "final_value",
        phase: "snapshot",
        kind: "output",
        name: "results",
        token_type: "prompt.fragment"
      }
    ],
    parameters: [
      {
        datatype: { kind: "query" },
        editor: {
          default_value: {
            filters: [],
            output: "compact",
            target: "rules"
          },
          kind: "query",
          source: "{query_builder()}"
        },
        name: "query"
      }
    ],
    signal_ports: [{
      name: "post_activate"
    }],
    short_description: "Runs workspace queries."
  },
  {
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
  },
  {
    action_ports: [],
    activation_parameters: [],
    class_name: "ChatAPI",
    dynamic_input_ports: [],
    dynamic_output_ports: [],
    handler_id: "primrose.chat-api",
    input_ports: [
      {
        mode: "final_value",
        phase: "execution",
        kind: "input",
        name: "input",
        token_type: "prompt.fragment"
      }
    ],
    interfaces: [],
    kind: "inline",
    long_description: "Sends a prompt fragment to an AI backend and emits streaming and final response tokens.",
    output_ports: [
      {
        mode: "final_value",
        phase: "execution",
        kind: "output",
        name: "response_stream",
        token_type: "chat.response_stream"
      },
      {
        mode: "final_value",
        phase: "execution",
        kind: "output",
        name: "response",
        token_type: "chat.response"
      }
    ],
    parameters: [
      {
        datatype: {
          item_type: { kind: "string" },
          kind: "list"
        },
        editor: {
          default_value: [],
          kind: "list",
          max: 1,
          source: "{model_picker()}"
        },
        name: "model"
      },
      {
        datatype: {
          item_type: { kind: "string" },
          kind: "list"
        },
        editor: {
          default_value: [],
          kind: "list",
          source: "{tool_bundle_picker()}"
        },
        name: "tools"
      },
      {
        datatype: {
          item_type: { kind: "string" },
          kind: "list"
        },
        editor: {
          default_value: [],
          kind: "list",
          max: 1,
          source: "{tool_execution_picker()}"
        },
        name: "tool_execution"
      },
      {
        datatype: {
          kind: "int"
        },
        editor: {
          default_value: 8,
          kind: "unsigned",
          max: 32,
          min: 1
        },
        name: "max_tool_rounds"
      }
    ],
    signal_ports: [{
      name: "post_activate"
    }],
    short_description: "Prompt an AI and get a response."
  },
  {
    action_ports: [],
    activation_parameters: [],
    class_name: "SessionOutput",
    dynamic_input_ports: [],
    dynamic_output_ports: [],
    handler_id: "primrose.session-output",
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
    kind: "sink",
    long_description: "Consumes streamed and final chat response tokens and records the final activation outputs.",
    output_ports: [],
    parameters: [],
    signal_ports: [{
      name: "post_activate"
    }],
    short_description: "Finalises a session activation."
  }
];

test("buildDefaultFlowWorkspace starts from a clean untitled draft", () => {
  const workspace = buildDefaultFlowWorkspace();

  assert.equal(workspace.title, "");
  assert.equal(workspace.nodes.length, 0);
  assert.equal(workspace.edges.length, 0);
  assert.equal(workspace.baselineFlow?.title, "");
});

test("buildFlowDocumentFromWorkspace preserves an empty title for dirty-state comparison", () => {
  const document = buildFlowDocumentFromWorkspace({
    description: "",
    draftFlowKey: "",
    edges: [],
    nodes: [],
    title: ""
  });

  assert.equal(document.title, "");
  const canvas = assertFlowEditorCanvas(document.layout);
  const nodes = canvas.nodes;
  assertRecord(nodes);
  assert.equal(Object.keys(nodes).length, 0);
});

test("buildFlowSaveDocument applies a confirmed draft name without mutating the workspace title", () => {
  const workspace = buildFlowWorkspaceFromDocument({
    definitions: DEFINITIONS,
    document: {
      execution: createFlowExecutionPolicy(),
      description: "  Draft description  ",
      edges: [],
      flow_key: "",
      layout: undefined,
      metadata: null,
      nodes: [{
        definition_name: "UserPrompt",
        id: "user-prompt-1",
        instance_name: "Prompt",
        parameter_values: {
          prompt: "Hello"
        }
      }],
      title: "",
      version: 2,
      output_encoding: "xml",
      plain_fragment_delimiter: "blank_line",
      whitespace_mode: "trim"
    }
  });

  const document = buildFlowSaveDocument({
    titleOverride: "Named draft",
    workspace
  });

  assert.ok(document);
  assert.equal(document.title, "Named draft");
  assert.equal(document.flow_key, "named-draft");
  assert.equal(document.description, "Draft description");
  assert.equal(workspace.title, "");
});

test("buildFlowValidationDocument injects internal identity placeholders for untitled drafts", () => {
  const workspace = buildDefaultFlowWorkspace();
  workspace.description = " Draft description ";

  const document = buildFlowValidationDocument({
    workspace
  });

  assert.equal(document.flow_key, "__draft_validation__");
  assert.equal(document.title, "Untitled flow");
  assert.equal(document.description, "Draft description");
});

test("buildFlowWorkspaceFromDocument maps definition ports to canvas handles", () => {
  const flow: WireFlowDocument = {
    execution: createFlowExecutionPolicy(),
    description: "Test flow",
    edges: [
      {
        id: "edge-1",
        kind: "token",
        order: 1,
        source_node_id: "user-prompt-1",
        source_port: "output",
        target_node_id: "concatenate-1",
        target_port: "input-0"
      }
    ],
    flow_key: "default-session-activation",
    layout: flowEditorLayout({
      canvas: {
        nodes: {
          "user-prompt-1": {
            position: { x: 60, y: 180 }
          },
          "concatenate-1": {
            position: { x: 430, y: 330 }
          }
        }
      }
    }),
    metadata: null,
    nodes: [
      {
        definition_name: "UserPrompt",
        id: "user-prompt-1",
        instance_name: "User Prompt",
        parameter_values: {}
      },
      {
        definition_name: "Concatenate",
        id: "concatenate-1",
        instance_name: "Concatenate",
        parameter_values: { input_ports: 2, ordering: "descending" }
      }
    ],
    title: "Default Session Activation",
    version: 2,
    output_encoding: "xml",
    plain_fragment_delimiter: "blank_line",
    whitespace_mode: "trim"
  };

  const workspace = buildFlowWorkspaceFromDocument({
    definitions: DEFINITIONS,
    document: flow
  });

  assert.equal(workspace.draftFlowKey, "default-session-activation");
  assert.deepEqual(
    workspace.nodes[1].data.inputPorts.map((port) => typeof port === "string" ? port : port.id),
    ["input-0", "input-1"]
  );
  assert.equal(workspace.edges[0].sourceHandle, "output-0");
  assert.equal(workspace.edges[0].targetHandle, "input-0");
  assert.deepEqual(workspace.nodes[0].position, { x: 60, y: 180 });
});

test("query parameter values survive a workspace round trip unchanged", () => {
  const flow: WireFlowDocument = {
    execution: createFlowExecutionPolicy(),
    description: "",
    edges: [],
    flow_key: "query-flow",
    layout: flowEditorLayout({
      canvas: {
        nodes: {
          "query-1": {
            position: { x: 220, y: 140 }
          }
        }
      }
    }),
    metadata: null,
    nodes: [
      {
        definition_name: "RulesQuery",
        id: "query-1",
        instance_name: "Query-1",
        parameter_values: {
          query: {
            filters: [
              {
                field: "rule_type",
                operator: "in",
                value: ["Narrative", "Prompt"]
              }
            ],
            output: "full",
            target: "rules"
          }
        }
      }
    ],
    title: "Query flow",
    version: 2,
    output_encoding: "xml",
    plain_fragment_delimiter: "blank_line",
    whitespace_mode: "trim"
  };

  const workspace = buildFlowWorkspaceFromDocument({
    definitions: DEFINITIONS,
    document: flow
  });
  const rebuilt = buildFlowDocumentFromWorkspace({
    description: workspace.description,
    draftFlowKey: workspace.draftFlowKey,
    edges: workspace.edges,
    nodes: workspace.nodes,
    title: workspace.title
  });

  assert.deepEqual(rebuilt.nodes[0]?.parameter_values.query, flow.nodes[0]?.parameter_values.query);
});

test("port parameter values survive a workspace round trip unchanged", () => {
  const formatterParameter = {
    datatype: { kind: "string" as const },
    editor: {
      default_value: "Default rule",
      kind: "text" as const
    },
    name: "formatter"
  };
  const definitions = [{
    action_ports: [],
    activation_parameters: [],
    class_name: "ChatResponseParser",
    dynamic_input_ports: [],
    dynamic_output_ports: [],
    handler_id: "primrose.chat-response-parser",
    input_ports: [],
    interfaces: [],
    kind: "hybrid" as const,
    long_description: "Parses structured responses.",
    output_ports: [{
      kind: "output" as const,
      name: "format_rule",
      parameters: [formatterParameter],
      token_type: "prompt.fragment"
    }],
    parameters: [],
    signal_ports: [],
    short_description: "Parse responses"
  }];
  const workspace = buildFlowWorkspaceFromDocument({
    definitions,
    document: {
      execution: createFlowExecutionPolicy(),
      description: "",
      edges: [],
      flow_key: "port-parameters",
      nodes: [{
        definition_name: "ChatResponseParser",
        id: "parser-1",
        instance_name: "Parser-1",
        parameter_values: {},
        port_parameter_values: {
          output: {
            format_rule: {
              formatter: "Custom rule"
            }
          }
        }
      }],
      title: "Port parameters",
      version: 2,
      output_encoding: "xml",
      plain_fragment_delimiter: "blank_line",
      whitespace_mode: "trim"
    }
  });
  const rebuilt = buildFlowDocumentFromWorkspace({
    description: workspace.description,
    draftFlowKey: workspace.draftFlowKey,
    edges: workspace.edges,
    nodes: workspace.nodes,
    title: workspace.title
  });

  assert.equal(
    rebuilt.nodes[0]?.port_parameter_values?.output?.format_rule?.formatter,
    "Custom rule"
  );
});

test("buildFlowDocumentFromWorkspace maps canvas handles back to manifest ports and persists layout", () => {
  const workspace = buildFlowWorkspaceFromDocument({
    definitions: DEFINITIONS,
    document: {
      execution: createFlowExecutionPolicy(),
      description: "",
      edges: [
        {
          id: "edge-1",
          kind: "token",
          order: 1,
          source_node_id: "chat-api-1",
          source_port: "response",
          target_node_id: "session-output-1",
          target_port: "response"
        }
      ],
      flow_key: "test-flow",
      layout: flowEditorLayout({
        canvas: {
          nodes: {
            "chat-api-1": { position: { x: 0, y: 0 } },
            "session-output-1": { position: { x: 200, y: 0 } }
          }
        }
      }),
      metadata: null,
      nodes: [
        {
          definition_name: "ChatAPI",
          id: "chat-api-1",
          instance_name: "Chat API",
          parameter_values: { default_model: [] }
        },
        {
          definition_name: "SessionOutput",
          id: "session-output-1",
          instance_name: "Session Output",
          parameter_values: {}
        }
      ],
      title: "Test flow",
      version: 2,
      output_encoding: "xml",
      plain_fragment_delimiter: "blank_line",
      whitespace_mode: "trim"
    }
  });

  const document = buildFlowDocumentFromWorkspace({
    description: workspace.description,
    draftFlowKey: workspace.draftFlowKey,
    edges: workspace.edges,
    nodes: workspace.nodes,
    title: workspace.title
  });

  assert.equal(document.edges[0].source_port, "response");
  assert.equal(document.edges[0].target_port, "response");
  const modelParameter = document.nodes[0].parameter_values.default_model;
  assert.ok(Array.isArray(modelParameter));
  assert.equal(modelParameter.length, 0);
  const canvas = assertFlowEditorCanvas(document.layout);
  const nodes = canvas.nodes;
  assertRecord(nodes);
  const chatApiNode = nodes["chat-api-1"];
  const sessionOutputNode = nodes["session-output-1"];
  assertRecord(chatApiNode);
  assertRecord(sessionOutputNode);
  const chatApiPosition = chatApiNode.position;
  const sessionOutputPosition = sessionOutputNode.position;
  assertRecord(chatApiPosition);
  assertRecord(sessionOutputPosition);
  assert.equal(chatApiPosition.x, 0);
  assert.equal(sessionOutputPosition.y, 0);
});

test("buildFlowDocumentFromWorkspace persists swapped controller port placement inside the canvas node layout blob", () => {
  const workspace = buildFlowWorkspaceFromDocument({
    definitions: DEFINITIONS,
    document: {
      execution: createFlowExecutionPolicy(),
      description: "",
      edges: [],
      flow_key: "test-flow",
      layout: flowEditorLayout({
        canvas: {
          nodes: {
            "user-prompt-1": {
              controllerPortPlacement: "swapped",
              position: { x: 40, y: 80 }
            }
          }
        }
      }),
      metadata: null,
      nodes: [
        {
          definition_name: "UserPrompt",
          id: "user-prompt-1",
          instance_name: "User Prompt",
          parameter_values: { prompt: "" }
        }
      ],
      title: "Test flow",
      version: 2,
      output_encoding: "xml",
      plain_fragment_delimiter: "blank_line",
      whitespace_mode: "trim"
    }
  });

  const document = buildFlowDocumentFromWorkspace({
    description: workspace.description,
    draftFlowKey: workspace.draftFlowKey,
    edges: workspace.edges,
    nodes: workspace.nodes,
    title: workspace.title
  });

  const canvas = assertFlowEditorCanvas(document.layout);
  const nodes = canvas.nodes;
  assertRecord(nodes);
  const promptNode = nodes["user-prompt-1"];
  assertRecord(promptNode);
  assert.equal(promptNode.controllerPortPlacement, "swapped");
});

test("buildFlowDocumentFromWorkspace round-trips reversed logic direction through layout only", () => {
  const workspace = buildFlowWorkspaceFromDocument({
    definitions: DEFINITIONS,
    document: {
      execution: createFlowExecutionPolicy(),
      description: "",
      edges: [],
      flow_key: "test-flow",
      layout: flowEditorLayout({
        canvas: {
          nodes: {
            "and-1": {
              controllerPortPlacement: "swapped",
              position: { x: 120, y: 160 }
            }
          }
        }
      }),
      metadata: null,
      nodes: [
        {
          definition_name: "AndGate",
          id: "and-1",
          instance_name: "AND-1",
          parameter_values: {
            input_ports: 2,
            not: true
          }
        }
      ],
      title: "Test flow",
      version: 2,
      output_encoding: "xml",
      plain_fragment_delimiter: "blank_line",
      whitespace_mode: "trim"
    }
  });

  const document = buildFlowDocumentFromWorkspace({
    description: workspace.description,
    draftFlowKey: workspace.draftFlowKey,
    edges: workspace.edges,
    nodes: workspace.nodes,
    title: workspace.title
  });

  assert.deepEqual(document.nodes[0]?.parameter_values, {
    input_ports: 2,
    not: true
  });
  assert.equal(
    Object.prototype.hasOwnProperty.call(document.nodes[0]?.parameter_values ?? {}, "visual_reverse"),
    false
  );
  const canvas = assertFlowEditorCanvas(document.layout);
  const nodes = canvas.nodes;
  assertRecord(nodes);
  const logicNode = nodes["and-1"];
  assertRecord(logicNode);
  assert.equal(logicNode.controllerPortPlacement, "swapped");
});

test("buildFlowDocumentFromWorkspace persists edge waypoints inside the canvas layout blob", () => {
  const workspace = buildFlowWorkspaceFromDocument({
    definitions: DEFINITIONS,
    document: {
      execution: createFlowExecutionPolicy(),
      description: "",
      edges: [
        {
          id: "edge-1",
          kind: "token",
          order: 1,
          source_node_id: "chat-api-1",
          source_port: "response",
          target_node_id: "session-output-1",
          target_port: "response"
        }
      ],
      flow_key: "test-flow",
      layout: flowEditorLayout({
        canvas: {
          edges: {
            "edge-1": {
              waypoints: [{
                inHandle: { x: 120, y: 20 },
                outHandle: { x: 180, y: 20 },
                position: { x: 150, y: 20 }
              }]
            }
          },
          nodes: {
            "chat-api-1": { position: { x: 0, y: 0 } },
            "session-output-1": { position: { x: 200, y: 0 } }
          }
        }
      }),
      metadata: null,
      nodes: [
        {
          definition_name: "ChatAPI",
          id: "chat-api-1",
          instance_name: "Chat API",
          parameter_values: { default_model: [] }
        },
        {
          definition_name: "SessionOutput",
          id: "session-output-1",
          instance_name: "Session Output",
          parameter_values: {}
        }
      ],
      title: "Test flow",
      version: 2,
      output_encoding: "xml",
      plain_fragment_delimiter: "blank_line",
      whitespace_mode: "trim"
    }
  });

  const document = buildFlowDocumentFromWorkspace({
    description: workspace.description,
    draftFlowKey: workspace.draftFlowKey,
    edges: workspace.edges,
    nodes: workspace.nodes,
    title: workspace.title
  });

  const canvas = assertFlowEditorCanvas(document.layout);
  const edges = canvas.edges;
  assertRecord(edges);
  const edgeLayout = edges["edge-1"];
  assertRecord(edgeLayout);
  const waypoints = edgeLayout.waypoints;
  assert.ok(Array.isArray(waypoints));
  assert.deepEqual(waypoints[0], {
    inHandle: { x: 120, y: 20 },
    outHandle: { x: 180, y: 20 },
    position: { x: 150, y: 20 }
  });
  const bridges = edgeLayout.bridges;
  assert.equal(bridges, undefined);
});

test("buildFlowDocumentFromWorkspace persists edge bridges inside the canvas layout blob", () => {
  const workspace = buildFlowWorkspaceFromDocument({
    definitions: DEFINITIONS,
    document: {
      execution: createFlowExecutionPolicy(),
      description: "",
      edges: [
        {
          id: "edge-1",
          kind: "token",
          order: 1,
          source_node_id: "chat-api-1",
          source_port: "response",
          target_node_id: "session-output-1",
          target_port: "response"
        }
      ],
      flow_key: "test-flow",
      layout: flowEditorLayout({
        canvas: {
          edges: {
            "edge-1": {
              bridges: [{
                gap: 18,
                segmentIndex: 0,
                t: 0.5
              }]
            }
          },
          nodes: {
            "chat-api-1": { position: { x: 0, y: 0 } },
            "session-output-1": { position: { x: 200, y: 0 } }
          }
        }
      }),
      metadata: null,
      nodes: [
        {
          definition_name: "ChatAPI",
          id: "chat-api-1",
          instance_name: "Chat API",
          parameter_values: { default_model: [] }
        },
        {
          definition_name: "SessionOutput",
          id: "session-output-1",
          instance_name: "Session Output",
          parameter_values: {}
        }
      ],
      title: "Test flow",
      version: 2,
      output_encoding: "xml",
      plain_fragment_delimiter: "blank_line",
      whitespace_mode: "trim"
    }
  });

  const document = buildFlowDocumentFromWorkspace({
    description: workspace.description,
    draftFlowKey: workspace.draftFlowKey,
    edges: workspace.edges,
    nodes: workspace.nodes,
    title: workspace.title
  });

  const canvas = assertFlowEditorCanvas(document.layout);
  const edges = canvas.edges;
  assertRecord(edges);
  const edgeLayout = edges["edge-1"];
  assertRecord(edgeLayout);
  const bridges = edgeLayout.bridges;
  assert.ok(Array.isArray(bridges));
  assert.deepEqual(bridges[0], {
    gap: 18,
    segmentIndex: 0,
    t: 0.5
  });
});

test("buildFlowWorkspaceFromDocument restores edge waypoints from layout", () => {
  const workspace = buildFlowWorkspaceFromDocument({
    definitions: DEFINITIONS,
    document: {
      execution: createFlowExecutionPolicy(),
      description: "",
      edges: [
        {
          id: "edge-1",
          kind: "token",
          order: 1,
          source_node_id: "chat-api-1",
          source_port: "response",
          target_node_id: "session-output-1",
          target_port: "response"
        }
      ],
      flow_key: "test-flow",
      layout: flowEditorLayout({
        canvas: {
          edges: {
            "edge-1": {
              waypoints: [{
                inHandle: { x: 120, y: 20 },
                outHandle: { x: 180, y: 20 },
                position: { x: 150, y: 20 }
              }]
            }
          },
          nodes: {
            "chat-api-1": { position: { x: 0, y: 0 } },
            "session-output-1": { position: { x: 200, y: 0 } }
          }
        }
      }),
      metadata: null,
      nodes: [
        {
          definition_name: "ChatAPI",
          id: "chat-api-1",
          instance_name: "Chat API",
          parameter_values: { default_model: [] }
        },
        {
          definition_name: "SessionOutput",
          id: "session-output-1",
          instance_name: "Session Output",
          parameter_values: {}
        }
      ],
      title: "Test flow",
      version: 2,
      output_encoding: "xml",
      plain_fragment_delimiter: "blank_line",
      whitespace_mode: "trim"
    }
  });

  assert.deepEqual(workspace.edges[0]?.data?.waypoints, [{
    inHandle: { x: 120, y: 20 },
    outHandle: { x: 180, y: 20 },
    position: { x: 150, y: 20 }
  }]);
});

test("buildFlowWorkspaceFromDocument restores swapped controller port placement from layout", () => {
  const workspace = buildFlowWorkspaceFromDocument({
    definitions: DEFINITIONS,
    document: {
      execution: createFlowExecutionPolicy(),
      description: "",
      edges: [],
      flow_key: "test-flow",
      layout: flowEditorLayout({
        canvas: {
          nodes: {
            "user-prompt-1": {
              controllerPortPlacement: "swapped",
              position: { x: 40, y: 80 }
            }
          }
        }
      }),
      metadata: null,
      nodes: [
        {
          definition_name: "UserPrompt",
          id: "user-prompt-1",
          instance_name: "User Prompt",
          parameter_values: { prompt: "" }
        }
      ],
      title: "Test flow",
      version: 2,
      output_encoding: "xml",
      plain_fragment_delimiter: "blank_line",
      whitespace_mode: "trim"
    }
  });

  assert.equal(workspace.nodes[0]?.data.controllerPortPlacement, "swapped");
});

test("buildFlowWorkspaceFromDocument restores edge bridges from layout", () => {
  const workspace = buildFlowWorkspaceFromDocument({
    definitions: DEFINITIONS,
    document: {
      execution: createFlowExecutionPolicy(),
      description: "",
      edges: [
        {
          id: "edge-1",
          kind: "token",
          order: 1,
          source_node_id: "chat-api-1",
          source_port: "response",
          target_node_id: "session-output-1",
          target_port: "response"
        }
      ],
      flow_key: "test-flow",
      layout: flowEditorLayout({
        canvas: {
          edges: {
            "edge-1": {
              bridges: [{
                gap: 18,
                segmentIndex: 0,
                t: 0.5
              }]
            }
          },
          nodes: {
            "chat-api-1": { position: { x: 0, y: 0 } },
            "session-output-1": { position: { x: 200, y: 0 } }
          }
        }
      }),
      metadata: null,
      nodes: [
        {
          definition_name: "ChatAPI",
          id: "chat-api-1",
          instance_name: "Chat API",
          parameter_values: { default_model: [] }
        },
        {
          definition_name: "SessionOutput",
          id: "session-output-1",
          instance_name: "Session Output",
          parameter_values: {}
        }
      ],
      title: "Test flow",
      version: 2,
      output_encoding: "xml",
      plain_fragment_delimiter: "blank_line",
      whitespace_mode: "trim"
    }
  });

  assert.deepEqual(workspace.edges[0]?.data?.bridges, [{
    gap: 18,
    segmentIndex: 0,
    t: 0.5
  }]);
});

test("buildFlowDocumentFromWorkspace preserves numbered dynamic ports", () => {
  const workspace = buildFlowWorkspaceFromDocument({
    definitions: DEFINITIONS,
    document: {
      execution: createFlowExecutionPolicy(),
      description: "",
      edges: [
        {
          id: "edge-1",
          kind: "token",
          order: 1,
          source_node_id: "user-prompt-1",
          source_port: "output",
          target_node_id: "concatenate-1",
          target_port: "input-2"
        }
      ],
      flow_key: "dynamic-flow",
      layout: undefined,
      metadata: null,
      nodes: [
        {
          definition_name: "UserPrompt",
          id: "user-prompt-1",
          instance_name: "User Prompt",
          parameter_values: {}
        },
        {
          definition_name: "Concatenate",
          id: "concatenate-1",
          instance_name: "Concatenate",
          parameter_values: { input_ports: 3, ordering: "descending" }
        }
      ],
      title: "Dynamic flow",
      version: 2,
      output_encoding: "xml",
      plain_fragment_delimiter: "blank_line",
      whitespace_mode: "trim"
    }
  });

  assert.deepEqual(
    workspace.nodes[1].data.inputPorts.map((port) => typeof port === "string" ? port : port.id),
    ["input-0", "input-1", "input-2"]
  );
  assert.equal(workspace.edges[0].targetHandle, "input-2");

  const document = buildFlowDocumentFromWorkspace({
    description: workspace.description,
    draftFlowKey: workspace.draftFlowKey,
    edges: workspace.edges,
    nodes: workspace.nodes,
    title: workspace.title
  });

  assert.equal(document.edges[0].target_port, "input-2");
});

test("buildFlowDocumentFromWorkspace canonicalises flow ordering for stable round trips", () => {
  const flow: WireFlowDocument = {
    execution: createFlowExecutionPolicy(),
    description: "",
    edges: [
      {
        id: "edge-b",
        kind: "token",
        order: 2,
        source_node_id: "user-prompt-1",
        source_port: "output",
        target_node_id: "concatenate-1",
        target_port: "input-1"
      },
      {
        id: "edge-a",
        kind: "token",
        order: 1,
        source_node_id: "user-prompt-1",
        source_port: "output",
        target_node_id: "concatenate-1",
        target_port: "input-0"
      }
    ],
    flow_key: "stable-round-trip",
    layout: flowEditorLayout({
      canvas: {
        edges: {
          "edge-b": {
            waypoints: [{
              inHandle: { x: 120, y: 40 },
              outHandle: { x: 180, y: 40 },
              position: { x: 150, y: 40 }
            }]
          },
          "edge-a": {
            bridges: [{
              gap: 18,
              segmentIndex: 0,
              t: 0.5
            }]
          }
        },
        nodes: {
          "user-prompt-1": {
            position: { x: 80, y: 120 }
          },
          "concatenate-1": {
            position: { x: 400, y: 120 }
          }
        }
      }
    }),
    metadata: null,
    nodes: [
      {
        definition_name: "UserPrompt",
        id: "user-prompt-1",
        instance_name: "User Prompt",
        parameter_values: {},
        port_names: {
          output: {
            output: "Story Prompt"
          }
        }
      },
      {
        definition_name: "Concatenate",
        id: "concatenate-1",
        instance_name: "Concatenate",
        parameter_values: {
          whitespace_mode: "trim",
          ordering: "descending",
          input_ports: 2
        },
        port_names: {
          input: {
            "input-1": "Context",
            "input-0": "Prompt"
          }
        }
      }
    ],
    title: "Stable round trip",
    version: 2,
    output_encoding: "xml",
    plain_fragment_delimiter: "blank_line",
    whitespace_mode: "trim"
  };

  const firstWorkspace = buildFlowWorkspaceFromDocument({
    definitions: DEFINITIONS,
    document: flow
  });
  const firstRoundTrip = buildFlowDocumentFromWorkspace({
    description: firstWorkspace.description,
    draftFlowKey: firstWorkspace.draftFlowKey,
    edges: firstWorkspace.edges,
    nodes: firstWorkspace.nodes,
    title: firstWorkspace.title
  });

  assert.deepEqual(
    firstRoundTrip.nodes.map((node) => node.id),
    ["concatenate-1", "user-prompt-1"]
  );
  assert.deepEqual(
    firstRoundTrip.edges.map((edge) => edge.id),
    ["edge-a", "edge-b"]
  );
  assert.deepEqual(
    Object.keys(firstRoundTrip.nodes[0]?.parameter_values ?? {}),
    ["input_ports", "ordering", "whitespace_mode"]
  );
  assert.deepEqual(
    Object.keys(firstRoundTrip.nodes[0]?.port_names?.input ?? {}),
    ["input-0", "input-1"]
  );
  assert.deepEqual(
    Object.keys(assertFlowEditorCanvas(firstRoundTrip.layout).edges ?? {}),
    ["edge-a", "edge-b"]
  );

  const secondWorkspace = buildFlowWorkspaceFromDocument({
    definitions: DEFINITIONS,
    document: firstRoundTrip
  });
  const secondRoundTrip = buildFlowDocumentFromWorkspace({
    description: secondWorkspace.description,
    draftFlowKey: secondWorkspace.draftFlowKey,
    edges: secondWorkspace.edges,
    nodes: secondWorkspace.nodes,
    title: secondWorkspace.title
  });

  assert.equal(
    JSON.stringify(firstRoundTrip, null, 2),
    JSON.stringify(secondRoundTrip, null, 2)
  );
});

test("buildFlowWorkspaceFromDocument uses deterministic fallback positions when layout is missing", () => {
  const workspace = buildFlowWorkspaceFromDocument({
    definitions: DEFINITIONS,
    document: {
      execution: createFlowExecutionPolicy(),
      description: "",
      edges: [],
      flow_key: "no-layout",
      layout: undefined,
      metadata: null,
      nodes: [
        {
          definition_name: "UserPrompt",
          id: "user-prompt-1",
          instance_name: "User Prompt",
          parameter_values: {}
        },
        {
          definition_name: "Concatenate",
          id: "concatenate-1",
          instance_name: "Concatenate",
          parameter_values: { input_ports: 2, ordering: "descending" }
        }
      ],
      title: "No layout",
      version: 2,
      output_encoding: "xml",
      plain_fragment_delimiter: "blank_line",
      whitespace_mode: "trim"
    }
  });

  assert.deepEqual(workspace.nodes[0].position, { x: 80, y: 120 });
  assert.deepEqual(workspace.nodes[1].position, { x: 400, y: 120 });
});

test("areFlowDocumentsEqual treats node ordering and JSON key ordering as irrelevant", () => {
  const left: WireFlowDocument = {
    execution: createFlowExecutionPolicy(),
    version: 2,
    output_encoding: "xml",
    plain_fragment_delimiter: "blank_line",
    whitespace_mode: "trim",
    flow_key: "same-flow",
    title: "Same flow",
    description: "",
    nodes: [
      {
        definition_name: "UserPrompt",
        id: "b-node",
        instance_name: "User Prompt",
        parameter_values: {}
      },
      {
        definition_name: "Concatenate",
        id: "a-node",
        instance_name: "Concatenate",
        parameter_values: {
          ordering: "descending",
          input_ports: 2
        }
      }
    ],
    edges: [],
    layout: flowEditorLayout({
      canvas: {
        nodes: {
          "a-node": { position: { y: 120, x: 80 } }
        }
      }
    }),
    metadata: null
  };

  const right: WireFlowDocument = {
    ...left,
    nodes: [...left.nodes].reverse(),
    layout: flowEditorLayout({
      canvas: {
        nodes: {
          "a-node": { position: { x: 80, y: 120 } }
        }
      }
    })
  };

  assert.equal(areFlowDocumentsEqual(left, right), true);
});

test("areFlowDocumentsEqual treats an empty canvas layout and a missing layout as equivalent", () => {
  const left: WireFlowDocument = {
    execution: createFlowExecutionPolicy(),
    description: "",
    edges: [],
    flow_key: "empty-flow",
    layout: undefined,
    metadata: null,
    nodes: [],
    title: "Empty flow",
    version: 2,
    output_encoding: "xml",
    plain_fragment_delimiter: "blank_line",
    whitespace_mode: "trim"
  };

  const right: WireFlowDocument = {
    ...left,
    layout: flowEditorLayout({
      canvas: {
        nodes: {}
      }
    })
  };

  assert.equal(areFlowDocumentsEqual(left, right), true);
});

test("areFlowDocumentsEqual ignores insignificant floating-point drift in layout positions", () => {
  const left: WireFlowDocument = {
    execution: createFlowExecutionPolicy(),
    description: "",
    edges: [],
    flow_key: "position-drift",
    layout: flowEditorLayout({
      canvas: {
        nodes: {
          "query-1": {
            position: {
              x: 1434.9939198254483,
              y: -18.536712071702222
            }
          }
        }
      }
    }),
    metadata: null,
    nodes: [
      {
        definition_name: "RulesQuery",
        id: "query-1",
        instance_name: "Query-1",
        parameter_values: {}
      }
    ],
    title: "Position drift",
    version: 2,
    output_encoding: "xml",
    plain_fragment_delimiter: "blank_line",
    whitespace_mode: "trim"
  };

  const right: WireFlowDocument = {
    ...left,
    layout: flowEditorLayout({
      canvas: {
        nodes: {
          "query-1": {
            position: {
              x: 1434.9939198254485,
              y: -18.536712071702226
            }
          }
        }
      }
    })
  };

  assert.equal(areFlowDocumentsEqual(left, right), true);
});

test("areFlowDocumentsEqual ignores insignificant floating-point drift in edge waypoints", () => {
  const left: WireFlowDocument = {
    execution: createFlowExecutionPolicy(),
    description: "",
    edges: [{
      id: "edge-1",
      kind: "token",
      order: 1,
      source_node_id: "chat-api-1",
      source_port: "response",
      target_node_id: "session-output-1",
      target_port: "response"
    }],
    flow_key: "edge-waypoint-drift",
    layout: flowEditorLayout({
      canvas: {
        edges: {
          "edge-1": {
            waypoints: [{
              inHandle: { x: 120.123456789, y: 20.987654321 },
              outHandle: { x: 180.123456789, y: 20.987654321 },
              position: { x: 150.123456789, y: 20.987654321 }
            }]
          }
        },
        nodes: {
          "chat-api-1": { position: { x: 0, y: 0 } },
          "session-output-1": { position: { x: 200, y: 0 } }
        }
      }
    }),
    metadata: null,
    nodes: [
      {
        definition_name: "ChatAPI",
        id: "chat-api-1",
        instance_name: "Chat API",
        parameter_values: {}
      },
      {
        definition_name: "SessionOutput",
        id: "session-output-1",
        instance_name: "Session Output",
        parameter_values: {}
      }
    ],
    title: "Waypoint drift",
    version: 2,
    output_encoding: "xml",
    plain_fragment_delimiter: "blank_line",
    whitespace_mode: "trim"
  };

  const right: WireFlowDocument = {
    ...left,
    layout: flowEditorLayout({
      canvas: {
        edges: {
          "edge-1": {
            waypoints: [{
              inHandle: { x: 120.1234567801, y: 20.9876543299 },
              outHandle: { x: 180.1234567801, y: 20.9876543299 },
              position: { x: 150.1234567801, y: 20.9876543299 }
            }]
          }
        },
        nodes: {
          "chat-api-1": { position: { x: 0, y: 0 } },
          "session-output-1": { position: { x: 200, y: 0 } }
        }
      }
    })
  };

  assert.equal(areFlowDocumentsEqual(left, right), true);
});

test("areFlowDocumentsEqual ignores insignificant floating-point drift in edge bridges", () => {
  const left: WireFlowDocument = {
    execution: createFlowExecutionPolicy(),
    description: "",
    edges: [{
      id: "edge-1",
      kind: "token",
      order: 1,
      source_node_id: "chat-api-1",
      source_port: "response",
      target_node_id: "session-output-1",
      target_port: "response"
    }],
    flow_key: "edge-bridge-drift",
    layout: flowEditorLayout({
      canvas: {
        edges: {
          "edge-1": {
            bridges: [{
              gap: 18.123456789,
              segmentIndex: 0,
              t: 0.5000000001
            }]
          }
        },
        nodes: {
          "chat-api-1": { position: { x: 0, y: 0 } },
          "session-output-1": { position: { x: 200, y: 0 } }
        }
      }
    }),
    metadata: null,
    nodes: [
      {
        definition_name: "ChatAPI",
        id: "chat-api-1",
        instance_name: "Chat API",
        parameter_values: {}
      },
      {
        definition_name: "SessionOutput",
        id: "session-output-1",
        instance_name: "Session Output",
        parameter_values: {}
      }
    ],
    title: "Bridge drift",
    version: 2,
    output_encoding: "xml",
    plain_fragment_delimiter: "blank_line",
    whitespace_mode: "trim"
  };

  const right: WireFlowDocument = {
    ...left,
    layout: flowEditorLayout({
      canvas: {
        edges: {
          "edge-1": {
            bridges: [{
              gap: 18.1234567801,
              segmentIndex: 0,
              t: 0.5000000009
            }]
          }
        },
        nodes: {
          "chat-api-1": { position: { x: 0, y: 0 } },
          "session-output-1": { position: { x: 200, y: 0 } }
        }
      }
    })
  };

  assert.equal(areFlowDocumentsEqual(left, right), true);
});

test("normalizeFlowWorkspaceState repairs invalid persisted data", () => {
  const normalized = normalizeFlowWorkspaceState({
    baselineFlow: null,
    description: 42 as never,
    draftFlowKey: undefined as never,
    edges: null as never,
    nodes: [
      {
        data: {
          actionPorts: [],
          definitionName: "UserPrompt",
          inputPorts: null,
          instanceName: "User Prompt",
          longDescription: null,
          nodeClass: "source",
          outputPorts: ["output"],
          parameterValues: null,
          signalPorts: null,
          shortDescription: null
        },
        id: "user-prompt-1",
        position: { x: Number.NaN, y: 40 },
        type: undefined
      } as never
    ],
    selectedFlowKey: null as never,
    selectedTarget: undefined as never,
    selectedNodeId: null as never,
    title: undefined as never
  });

  assert.equal(normalized.description, "");
  assert.equal(normalized.nodes[0].data.definitionName, "UserPrompt");
  assert.deepEqual(normalized.nodes[0].data.inputPorts, []);
  assert.deepEqual(
    normalized.nodes[0].data.outputPorts.map((port) => typeof port === "string" ? null : port.displayClass),
    ["source"]
  );
  assert.equal(normalized.nodes[0].className, "flow-studio-node flow-studio-node--source");
  assert.deepEqual(normalized.nodes[0].data.parameterValues, {});
  assert.deepEqual(normalized.nodes[0].position, { x: 0, y: 40 });
  assert.equal(normalized.nodes[0].type, "flowStudio");
});

test("reconcileFlowWorkspaceDefinitions refreshes persisted ports from current definitions", () => {
  const workspace = normalizeFlowWorkspaceState({
    ...buildDefaultFlowWorkspace(),
    nodes: [
      {
        className: "flow-studio-node flow-studio-node--hybrid",
        data: {
          actionPorts: [],
          definitionName: "ChatResponseParser",
          inputPorts: ["response_stream", "response"],
          instanceName: "Response Parser-14",
          longDescription: "Parses the structured response envelope.",
          nodeClass: "hybrid",
          outputPorts: [
            "format_rule",
            "user_response_stream"
          ],
          parameterValues: {},
          portNames: undefined,
          signalPorts: [],
          shortDescription: "Parses a structured response."
        },
        id: "response-parser-14",
        position: { x: 120, y: 80 },
        type: "flowStudio"
      }
    ]
  });

  const reconciled = reconcileFlowWorkspaceDefinitions(workspace, createFlowDefinitionLookup([
    {
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
      long_description: "Parses the structured response envelope.",
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
          name: "user_response_stream",
          token_type: "chat.response_stream"
        }
      ],
      parameters: [],
      signal_ports: [],
      short_description: "Parses a structured response."
    }
  ]));

  assert.equal(reconciled.nodes[0].data.nodeClass, "hybrid");
  assert.deepEqual(
    reconciled.nodes[0].data.outputPorts.map((port) => typeof port === "string" ? null : port.displayClass),
    ["source", "inline"]
  );
});

test("removeNodeFromWorkspace drops the node and all attached edges", () => {
  const workspace = buildFlowWorkspaceFromDocument({
    definitions: DEFINITIONS,
    document: {
      execution: createFlowExecutionPolicy(),
      description: "",
      edges: [
        {
          id: "edge-1",
          kind: "token",
          order: 1,
          source_node_id: "user-prompt-1",
          source_port: "output",
          target_node_id: "concatenate-1",
          target_port: "input-0"
        }
      ],
      flow_key: "remove-node",
      layout: undefined,
      metadata: null,
      nodes: [
        {
          definition_name: "UserPrompt",
          id: "user-prompt-1",
          instance_name: "User Prompt",
          parameter_values: {}
        },
        {
          definition_name: "Concatenate",
          id: "concatenate-1",
          instance_name: "Concatenate",
          parameter_values: { input_ports: 2, ordering: "descending" }
        }
      ],
      title: "Remove node",
      version: 2,
      output_encoding: "xml",
      plain_fragment_delimiter: "blank_line",
      whitespace_mode: "trim"
    }
  });

  const next = removeNodeFromWorkspace({
    ...workspace,
    selectedTarget: { kind: "node", nodeId: "user-prompt-1" }
  }, "user-prompt-1");

  assert.equal(next.nodes.length, 1);
  assert.equal(next.edges.length, 0);
  assert.equal(next.selectedTarget.kind, "none");
});

test("renameNodeInstanceInWorkspace updates the node instance name without disturbing position", () => {
  const workspace = buildFlowWorkspaceFromDocument({
    definitions: DEFINITIONS,
    document: {
      execution: createFlowExecutionPolicy(),
      description: "",
      edges: [],
      flow_key: "rename-node",
      layout: flowEditorLayout({
        canvas: {
          nodes: {
            "chat-api-1": {
              position: { x: 140, y: 260 }
            }
          }
        }
      }),
      metadata: null,
      nodes: [
        {
          definition_name: "ChatAPI",
          id: "chat-api-1",
          instance_name: "Old name",
          parameter_values: {}
        }
      ],
      title: "Rename node",
      version: 2,
      output_encoding: "xml",
      plain_fragment_delimiter: "blank_line",
      whitespace_mode: "trim"
    }
  });

  const renamedWorkspace = renameNodeInstanceInWorkspace(workspace, "chat-api-1", "Renamed node");

  assert.equal(renamedWorkspace.nodes[0].data.instanceName, "Renamed node");
  assert.deepEqual(renamedWorkspace.nodes[0].position, { x: 140, y: 260 });
});

test("findNextFlowNodeIndex returns one greater than the highest numeric node suffix", () => {
  const workspace = buildFlowWorkspaceFromDocument({
    definitions: DEFINITIONS,
    document: {
      execution: createFlowExecutionPolicy(),
      description: "",
      edges: [],
      flow_key: "suffixes",
      layout: undefined,
      metadata: null,
      nodes: [
        {
          definition_name: "UserPrompt",
          id: "user-prompt-3",
          instance_name: "User Prompt",
          parameter_values: {}
        },
        {
          definition_name: "Concatenate",
          id: "concatenate-9",
          instance_name: "Concatenate",
          parameter_values: { input_ports: 2, ordering: "descending" }
        },
        {
          definition_name: "ChatAPI",
          id: "chat-api",
          instance_name: "Chat API",
          parameter_values: { default_model: [] }
        }
      ],
      title: "Suffixes",
      version: 2,
      output_encoding: "xml",
      plain_fragment_delimiter: "blank_line",
      whitespace_mode: "trim"
    }
  });

  assert.equal(findNextFlowNodeIndex(workspace.nodes), 10);
});


test("slugifyFlowKey lowercases and trims repeated punctuation", () => {
  assert.equal(slugifyFlowKey("  My First Flow!  "), "my-first-flow");
  assert.equal(slugifyFlowKey("---"), "untitled-flow");
});

test("multiplexer output count survives a workspace round trip", () => {
  const definitions: WireFlowNodeDefinition[] = [{
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
    signal_ports: [],
    short_description: "Duplicates one input to many outputs."
  }];

  const workspace = buildFlowWorkspaceFromDocument({
    definitions,
    document: {
      execution: createFlowExecutionPolicy(),
      description: "",
      edges: [],
      flow_key: "multiplexer-round-trip",
      layout: undefined,
      metadata: null,
      nodes: [
        {
          definition_name: "Multiplexer",
          id: "multiplexer-1",
          instance_name: "Multiplexer-1",
          parameter_values: {
            output_ports: 3
          }
        }
      ],
      title: "Multiplexer round trip",
      version: 2,
      output_encoding: "xml",
      plain_fragment_delimiter: "blank_line",
      whitespace_mode: "trim"
    }
  });

  assert.deepEqual(
    workspace.nodes[0].data.outputPorts.map((port) => typeof port === "string" ? port : port.id),
    ["output-0", "output-1", "output-2"]
  );

  const roundTripped = buildFlowDocumentFromWorkspace({
    description: workspace.description,
    draftFlowKey: workspace.draftFlowKey,
    edges: workspace.edges,
    nodes: workspace.nodes,
    title: workspace.title
  });

  assert.equal(roundTripped.nodes[0]?.definition_name, "Multiplexer");
  assert.equal(roundTripped.nodes[0]?.parameter_values.output_ports, 3);
});

test("node port order survives a document to workspace round trip without adding untouched sides", () => {
  const document: WireFlowDocument = {
    execution: createFlowExecutionPolicy(),
    description: "",
    edges: [],
    flow_key: "port-order-round-trip",
    layout: undefined,
    metadata: null,
    nodes: [{
      definition_name: "Concatenate",
      id: "concatenate-1",
      instance_name: "Concatenate-1",
      parameter_values: {
        input_ports: 4
      },
      port_order: {
        input: ["input-2", "input-0", "input-1"]
      }
    }],
    title: "Port order round trip",
    version: 2,
    output_encoding: "xml",
    plain_fragment_delimiter: "blank_line",
    whitespace_mode: "trim"
  };

  const workspace = buildFlowWorkspaceFromDocument({
    definitions: DEFINITIONS,
    document
  });

  assert.deepEqual(
    workspace.nodes[0]?.data.inputPorts.map((port) => typeof port === "string" ? port : port.id),
    ["input-2", "input-0", "input-1", "input-3"]
  );
  assert.deepEqual(workspace.nodes[0]?.data.portOrder?.input, ["input-2", "input-0", "input-1", "input-3"]);
  assert.equal(workspace.nodes[0]?.data.portOrder?.output, undefined);

  const roundTripped = buildFlowDocumentFromWorkspace({
    description: workspace.description,
    draftFlowKey: workspace.draftFlowKey,
    edges: workspace.edges,
    nodes: workspace.nodes,
    title: workspace.title
  });

  assert.deepEqual(roundTripped.nodes[0]?.port_order?.input, ["input-2", "input-0", "input-1", "input-3"]);
  assert.equal(roundTripped.nodes[0]?.port_order?.output, undefined);
  assert.equal(roundTripped.nodes[0]?.port_order?.signal, undefined);
});

test("saving an edited document preserves host metadata and another editor's layout", () => {
  const document = { ...buildFlowDocumentFromWorkspace(buildDefaultFlowWorkspace()), flow_key: "extensions", title: "Extensions",
    metadata: { application: { revision: 7 } }, layout: { external_editor: { viewport: { x: 2, y: 3 } } } };
  const before = structuredClone(document);
  const workspace = buildFlowWorkspaceFromDocument({ definitions: [], document });
  workspace.description = "An authored change";
  const saved = buildFlowSaveDocument({ workspace });
  assert.ok(saved);
  assert.deepEqual(saved.metadata, document.metadata);
  assert.deepEqual((saved.layout as Record<string, unknown>).external_editor, document.layout.external_editor);
  assert.deepEqual(document, before);
});

test("legacy flows cannot enter the editor or be saved without an explicit upgrade", () => {
  const document = buildFlowDocumentFromWorkspace(buildDefaultFlowWorkspace());
  const legacy = { ...document, version: 1 };
  delete (legacy as Partial<WireFlowDocument>).execution;
  const before = structuredClone(legacy);
  assert.throws(() => buildFlowWorkspaceFromDocument({ definitions: [], document: legacy }), /explicit upgrade/);
  assert.throws(() => buildFlowDocumentFromWorkspace({ ...buildDefaultFlowWorkspace(), baselineFlow: legacy }), /explicit upgrade/);
  assert.throws(() => normalizeFlowWorkspaceState({ ...buildDefaultFlowWorkspace(), baselineFlow: legacy }), /explicit upgrade/);
  assert.deepEqual(legacy, before);
  const missingPolicy = { ...document };
  delete (missingPolicy as Partial<WireFlowDocument>).execution;
  assert.throws(() => buildFlowWorkspaceFromDocument({ definitions: [], document: missingPolicy }), /execution policy/);
});

test("saving preserves authored source priority and limits across layout and node-array changes", () => {
  const order = ["z-source", "a-source"];
  const document: WireFlowDocument = {
    ...buildFlowDocumentFromWorkspace(buildDefaultFlowWorkspace()),
    execution: createFlowExecutionPolicy(order),
    nodes: order.map(id => ({ id, definition_name: "UserPrompt", instance_name: id, parameter_values: {} })),
  };
  document.execution.limits.node_retained_bytes /= 2;
  const before = structuredClone(document);
  const workspace = buildFlowWorkspaceFromDocument({ definitions: DEFINITIONS, document });
  workspace.nodes.reverse();
  workspace.nodes[0].position.x += 100;
  workspace.nodes[0].data.instanceName = "A new title";
  const saved = buildFlowDocumentFromWorkspace({ ...workspace, baselineFlow: workspace.baselineFlow });
  assert.deepEqual(saved.execution, before.execution);
  assert.deepEqual(document, before);
  saved.execution.source_order.reverse();
  saved.execution.limits.node_retained_bytes /= 2;
  assert.deepEqual(workspace.baselineFlow?.execution, before.execution);
});

test("stream queue policy survives editor round trips and new edges receive finite limits", () => {
  const source = structuredClone(DEFINITIONS.find(def => def.class_name === "UserPrompt")!);
  source.output_ports[0].mode = "stream";
  source.output_ports[0].phase = "execution";
  const sink: WireFlowNodeDefinition = {
    ...structuredClone(source), class_name: "StreamSink", kind: "sink",
    input_ports: [{ ...source.output_ports[0], kind: "input", name: "input" }], output_ports: [],
  };
  const queue = { ...createFlowExecutionPolicy().limits.provider_queue, items: 3, policy: "drop_oldest" as const };
  const document: WireFlowDocument = {
    ...buildFlowDocumentFromWorkspace(buildDefaultFlowWorkspace()),
    execution: createFlowExecutionPolicy(["producer"]),
    nodes: [
      { id: "producer", definition_name: source.class_name, instance_name: "Producer", parameter_values: {} },
      { id: "consumer", definition_name: sink.class_name, instance_name: "Consumer", parameter_values: {} },
    ],
    edges: [{ id: "stream", kind: "token", order: 0, source_node_id: "producer", source_port: "output", target_node_id: "consumer", target_port: "input", queue }],
  };
  const workspace = buildFlowWorkspaceFromDocument({ definitions: [source, sink], document });
  const saved = buildFlowDocumentFromWorkspace(workspace);
  assert.deepEqual(saved.edges[0].queue, queue);
  saved.edges[0].queue!.items += 1;
  assert.deepEqual(workspace.edges[0].data?.queue, queue);
  delete workspace.edges[0].data!.queue;
  const newEdge = buildFlowDocumentFromWorkspace(workspace);
  assert.deepEqual(newEdge.edges[0].queue, document.execution.limits.provider_queue);
  for (const node of workspace.nodes) {
    for (const port of [...node.data.inputPorts, ...node.data.outputPorts]) {
      if (typeof port !== "string") port.mode = "final_value";
    }
  }
  workspace.edges[0].data!.queue = queue;
  const finalEdge = buildFlowDocumentFromWorkspace(workspace);
  assert.equal(finalEdge.edges[0].queue, undefined);
});
