/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises flow node ports behaviour in the editor's dataflow workspace.
 */
import { tokenConnectionCompatible } from "@battersea/flow";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  buildFlowPortSlots,
  canAttachConnectionToPorts,
  buildResolvedFlowPort,
  connectionUsesCompatibleTokenTypes,
  createFlowPortHandleId,
  getFlowPortLabel,
  pruneEdgesForNodeCardinality,
  resolveConnectionFromHandlePair
} from "@battersea/editor/core/flow-node-ports";

test("buildFlowPortSlots applies stable spacing and shared node metrics", () => {
  assert.deepEqual(
    buildFlowPortSlots("input", {
      actionCount: 0,
      automationCount: 0,
      inputCount: 0,
      outputCount: 0,
      signalCount: 0
    }, "inline"),
    {
      handleGapPx: 14,
      nodeHeightPx: 116,
      nodeWidthPx: 228,
      pillMaxWidthPx: 156,
      slots: []
    }
  );

  assert.deepEqual(
    buildFlowPortSlots("input", {
      actionCount: 0,
      automationCount: 0,
      inputCount: 1,
      outputCount: 0,
      signalCount: 0
    }, "inline"),
    {
      handleGapPx: 14,
      nodeHeightPx: 116,
      nodeWidthPx: 228,
      pillMaxWidthPx: 156,
      slots: [
        { handleId: "input-0", index: 0, offsetPixels: 58, offsetPercent: 50, sideCount: 1 }
      ]
    }
  );

  assert.deepEqual(
    buildFlowPortSlots("output", {
      actionCount: 0,
      automationCount: 0,
      inputCount: 3,
      outputCount: 3,
      signalCount: 0
    }, "inline"),
    {
      handleGapPx: 14,
      nodeHeightPx: 116,
      nodeWidthPx: 228,
      pillMaxWidthPx: 156,
      slots: [
        { handleId: "output-0", index: 0, offsetPixels: 22, offsetPercent: 18.966, sideCount: 3 },
        { handleId: "output-1", index: 1, offsetPixels: 58, offsetPercent: 50, sideCount: 3 },
        { handleId: "output-2", index: 2, offsetPixels: 94, offsetPercent: 81.034, sideCount: 3 }
      ]
    }
  );
});

test("buildFlowPortSlots grows the node axis only when density requires more room", () => {
  const verticalLayout = buildFlowPortSlots("output", {
    actionCount: 0,
    automationCount: 0,
    inputCount: 6,
    outputCount: 6,
    signalCount: 0
  }, "inline");
  const horizontalLayout = buildFlowPortSlots("signal", {
    actionCount: 8,
    automationCount: 0,
    inputCount: 0,
    outputCount: 0,
    signalCount: 8
  }, "inline");

  assert.equal(verticalLayout.nodeHeightPx, 184);
  assert.deepEqual(
    verticalLayout.slots.map((slot) => slot.offsetPixels),
    [22, 50, 78, 106, 134, 162]
  );

  assert.equal(horizontalLayout.nodeWidthPx, 240);
  assert.deepEqual(
    horizontalLayout.slots.map((slot) => slot.offsetPixels),
    [22, 50, 78, 106, 134, 162, 190, 218]
  );
});

test("buildFlowPortSlots rotates instrument controller ports onto the vertical axis and uses compact metrics", () => {
  const instrumentLayout = buildFlowPortSlots("signal", {
    actionCount: 3,
    automationCount: 0,
    inputCount: 0,
    outputCount: 0,
    signalCount: 3
  }, "instrument");

  assert.equal(instrumentLayout.nodeHeightPx, 100);
  assert.equal(instrumentLayout.nodeWidthPx, 184);
  assert.equal(instrumentLayout.pillMaxWidthPx, 120);
  assert.deepEqual(
    instrumentLayout.slots.map((slot) => slot.offsetPixels),
    [22, 50, 78]
  );
});

test("createFlowPortHandleId produces stable handle ids for every port family", () => {
  assert.equal(createFlowPortHandleId("action", 1), "action-1");
  assert.equal(createFlowPortHandleId("input", 2), "input-2");
  assert.equal(createFlowPortHandleId("output", 4), "output-4");
  assert.equal(createFlowPortHandleId("signal", 0), "signal-0");
});

test("pruneEdgesForNodeCardinality removes edges that no longer have a backing port", () => {
  const edges = [
    {
      id: "kept-input",
      source: "upstream",
      sourceHandle: "output-0",
      target: "node-1",
      targetHandle: "input-0"
    },
    {
      id: "removed-input",
      source: "upstream",
      sourceHandle: "output-1",
      target: "node-1",
      targetHandle: "input-2"
    },
    {
      id: "removed-output",
      source: "node-1",
      sourceHandle: "output-3",
      target: "downstream",
      targetHandle: "input-0"
    },
    {
      id: "untouched",
      source: "other-node",
      sourceHandle: "output-0",
      target: "downstream",
      targetHandle: "input-0"
    }
  ];

  assert.deepEqual(
    pruneEdgesForNodeCardinality(edges, "node-1", "input", 2).map((edge) => edge.id),
    ["kept-input", "removed-output", "untouched"]
  );
  assert.deepEqual(
    pruneEdgesForNodeCardinality(edges, "node-1", "output", 2).map((edge) => edge.id),
    ["kept-input", "removed-input", "untouched"]
  );
  assert.deepEqual(
    pruneEdgesForNodeCardinality(edges, "node-1", "input", 0).map((edge) => edge.id),
    ["removed-output", "untouched"]
  );
});

test("pruneEdgesForNodeCardinality preserves non-matching handle families on the same node", () => {
  const edges = [
    {
      id: "signal-kept",
      source: "upstream",
      sourceHandle: "signal-0",
      target: "node-1",
      targetHandle: "action-0"
    },
    {
      id: "token-removed",
      source: "upstream",
      sourceHandle: "output-0",
      target: "node-1",
      targetHandle: "input-0"
    }
  ];

  assert.deepEqual(
    pruneEdgesForNodeCardinality(edges, "node-1", "input", 0).map((edge) => edge.id),
    ["signal-kept"]
  );
});

test("canAttachConnectionToPorts permits token fan-out to different inputs", () => {
  assert.equal(
    canAttachConnectionToPorts([
      {
        source: "mux-1",
        sourceHandle: "output-0",
        target: "story-1",
        targetHandle: "input-0"
      }
    ], {
      source: "mux-1",
      sourceHandle: "output-0",
      target: "story-2",
      targetHandle: "input-0"
    }),
    true
  );
});

test("canAttachConnectionToPorts rejects a second edge on the same input handle", () => {
  assert.equal(
    canAttachConnectionToPorts([
      {
        source: "prompt-1",
        sourceHandle: "output-0",
        target: "model-1",
        targetHandle: "input-0"
      }
    ], {
      source: "rules-1",
      sourceHandle: "output-0",
      target: "model-1",
      targetHandle: "input-0"
    }),
    false
  );
});

test("canAttachConnectionToPorts allows connections on different handles", () => {
  assert.equal(
    canAttachConnectionToPorts([
      {
        source: "mux-1",
        sourceHandle: "output-0",
        target: "augment-1",
        targetHandle: "input-0"
      }
    ], {
      source: "mux-1",
      sourceHandle: "output-1",
      target: "augment-1",
      targetHandle: "input-1"
    }),
    true
  );
});

test("canAttachConnectionToPorts rejects incompatible token types when node port metadata is available", () => {
  assert.equal(
    canAttachConnectionToPorts([], {
      source: "query-1",
      sourceHandle: "output-0",
      target: "chat-api-1",
      targetHandle: "input-0"
    }, [
      {
        id: "query-1",
        data: {
          outputPorts: [buildResolvedFlowPort({
            id: "results",
            side: "output",
            tokenType: "prompt.fragmentArray"
          })]
        }
      },
      {
        id: "chat-api-1",
        data: {
          inputPorts: [buildResolvedFlowPort({
            acceptedTokenTypes: ["prompt.fragment"],
            id: "input",
            side: "input",
            tokenType: "prompt.fragment"
          })]
        }
      }
    ]),
    false
  );
});

test("connectionUsesCompatibleTokenTypes rejects an auto Multiplexer output into the strict response_metadata input", () => {
  // Reproduces the editor letting you wire a Multiplexer (auto output,
  // inputs only ever prompt.fragment|chat.raw|chat.response) straight into
  // ChronicleOutput's response_metadata (chat.response_metadata only).
  // The auto output can never carry chat.response_metadata, so this
  // connection must be refused. Expected: false.
  assert.equal(
    connectionUsesCompatibleTokenTypes({
      connection: {
        source: "mux-1",
        sourceHandle: "output-0",
        target: "chronicle-1",
        targetHandle: "input-2"
      },
      nodes: [
        {
          id: "mux-1",
          data: {
            inputPorts: [buildResolvedFlowPort({
              acceptedTokenTypes: ["prompt.fragment", "chat.raw", "chat.response"],
              id: "input",
              side: "input",
              tokenType: "oneof"
            })],
            outputPorts: [buildResolvedFlowPort({
              id: "output-0",
              side: "output",
              tokenType: "auto"
            })]
          }
        },
        {
          id: "chronicle-1",
          data: {
            // Faithful to the real ChronicleOutput: prompt[0],
            // response[1], response_metadata[2] — so handle "input-2"
            // resolves to the strict metadata port.
            inputPorts: [
              buildResolvedFlowPort({
                id: "prompt",
                side: "input",
                tokenType: "chat.prompt"
              }),
              buildResolvedFlowPort({
                id: "response",
                side: "input",
                tokenType: "chat.response"
              }),
              buildResolvedFlowPort({
                id: "response_metadata",
                side: "input",
                tokenType: "chat.response_metadata"
              })
            ]
          }
        }
      ]
    }),
    false
  );
});

test("tokenConnectionCompatible matches the shared cross-language fixture", () => {
  const fixtureUrl = new URL(import.meta.resolve("@battersea/flow/fixtures/token-type-compatibility.json"));
  const fixture = JSON.parse(readFileSync(fixtureUrl, "utf8")) as {
    cases: ReadonlyArray<{
      name: string;
      source_token_type: string;
      source_node_input_accepted: string[];
      target_accepted: string[];
      expected: boolean;
    }>;
  };
  assert.ok(fixture.cases.length > 0, "fixture must contain at least one case");
  for (const testCase of fixture.cases) {
    assert.equal(
      tokenConnectionCompatible(
        testCase.source_token_type,
        testCase.source_node_input_accepted,
        testCase.target_accepted
      ),
      testCase.expected,
      `shared fixture case "${testCase.name}" disagrees with the editor rule`
    );
  }
});

test("canAttachConnectionToPorts rejects a second edge on the same signal handle", () => {
  assert.equal(
    canAttachConnectionToPorts([
      {
        data: { kind: "signal", order: 1 },
        source: "prompt-1",
        sourceHandle: "signal-0",
        target: "prompt-1",
        targetHandle: "action-0"
      }
    ], {
      source: "prompt-1",
      sourceHandle: "signal-0",
      target: "prompt-2",
      targetHandle: "action-0"
    }),
    false
  );
});

test("canAttachConnectionToPorts rejects mixed token and signal handle families", () => {
  assert.equal(
    canAttachConnectionToPorts([], {
      source: "prompt-1",
      sourceHandle: "signal-0",
      target: "concat-1",
      targetHandle: "input-0"
    }),
    false
  );
});

test("resolveConnectionFromHandlePair maps handle ids to a connectable edge shape", () => {
  assert.deepEqual(
    resolveConnectionFromHandlePair({
      fromHandle: {
        id: "output-1",
        nodeId: "mux-1"
      },
      toHandle: {
        id: "input-0",
        nodeId: "story-1"
      }
    }),
    {
      source: "mux-1",
      sourceHandle: "output-1",
      target: "story-1",
      targetHandle: "input-0"
    }
  );
});

test("resolveConnectionFromHandlePair supports signal-to-action wiring", () => {
  assert.deepEqual(
    resolveConnectionFromHandlePair({
      fromHandle: {
        id: "signal-0",
        nodeId: "prompt-1"
      },
      toHandle: {
        id: "action-0",
        nodeId: "prompt-1"
      }
    }),
    {
      source: "prompt-1",
      sourceHandle: "signal-0",
      target: "prompt-1",
      targetHandle: "action-0"
    }
  );
});

test("resolveConnectionFromHandlePair returns null when the drag does not end on a handle", () => {
  assert.equal(
    resolveConnectionFromHandlePair({
      fromHandle: {
        id: "output-0",
        nodeId: "prompt-1"
      },
      toHandle: null
    }),
    null
  );
});

test("getFlowPortLabel names the model API outputs", () => {
  assert.equal(getFlowPortLabel("response"), "Response");
  assert.equal(getFlowPortLabel("response_stream"), "Response Stream");
});

test("getFlowPortLabel names augment inputs and mux outputs", () => {
  assert.equal(getFlowPortLabel("input-0"), "Input 0");
  assert.equal(getFlowPortLabel("input-1"), "Input 1");
  assert.equal(getFlowPortLabel("output-0"), "Output 0");
  assert.equal(getFlowPortLabel("output-2"), "Output 2");
});


test("connection compatibility checks consumption mode and snapshot ownership independently of nominal types", () => {
  const source = buildResolvedFlowPort({id: "out", side: "output", tokenType: "text", mode: "stream", phase: "execution"});
  const target = buildResolvedFlowPort({id: "in", side: "input", tokenType: "text", mode: "stream", phase: "execution"});
  const nodes = [{id: "source", data: {outputPorts: [source]}}, {id: "sink", data: {inputPorts: [target]}}];
  const connection = {source: "source", sourceHandle: "output-0", target: "sink", targetHandle: "input-0"};
  assert.equal(connectionUsesCompatibleTokenTypes({nodes, connection}), true);
  target.mode = "final_value";
  assert.equal(connectionUsesCompatibleTokenTypes({nodes, connection}), false);
  source.mode = "final_value";
  target.phase = "snapshot";
  assert.equal(connectionUsesCompatibleTokenTypes({nodes, connection}), false);
  source.phase = "snapshot";
  assert.equal(connectionUsesCompatibleTokenTypes({nodes, connection}), true);
});
