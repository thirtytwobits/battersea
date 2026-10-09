/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises dataflow connection helper behaviour in the editor's dataflow workspace.
 */
import assert from "node:assert/strict";
import test from "node:test";

import type { FlowStudioEdge } from "@battersea/editor/core/dataflow-editor-state";
import { hasStructuralEdgeRemovalChange, resolveConnectionAppendResult } from "@battersea/editor/hooks/use-dataflow-connections";

test("resolveConnectionAppendResult keeps duplicate source and target port attachments out of the graph", () => {
  const edges: FlowStudioEdge[] = [{
    id: "chat-api-1-output-0-render-1-input-0-1",
    source: "chat-api-1",
    sourceHandle: "output-0",
    target: "render-1",
    targetHandle: "input-0"
  }];

  const result = resolveConnectionAppendResult({
    connection: {
      source: "chat-api-1",
      sourceHandle: "output-0",
      target: "other-render-1",
      targetHandle: "input-0"
    },
    edges
  });

  assert.equal(result.status, "duplicate-port");
  assert.deepEqual(result.edges, edges);
});

test("resolveConnectionAppendResult appends valid edges using the current id and ordering scheme", () => {
  const result = resolveConnectionAppendResult({
    connection: {
      source: "chat-api-1",
      sourceHandle: "output-0",
      target: "render-1",
      targetHandle: "input-0"
    },
    edges: []
  });

  assert.equal(result.status, "updated");
  assert.deepEqual(result.edges, [{
    id: "chat-api-1-output-0-render-1-input-0-1",
    source: "chat-api-1",
    sourceHandle: "output-0",
    target: "render-1",
    targetHandle: "input-0",
    data: {
      kind: "token",
      order: 1
    }
  }]);
});

test("resolveConnectionAppendResult marks signal edges with the signal kind", () => {
  const result = resolveConnectionAppendResult({
    connection: {
      source: "prompt-1",
      sourceHandle: "signal-0",
      target: "prompt-1",
      targetHandle: "action-0"
    },
    edges: []
  });

  assert.equal(result.status, "updated");
  assert.deepEqual(result.edges[0]?.data, {
    kind: "signal",
    order: 1
  });
});

test("hasStructuralEdgeRemovalChange only reports actual structural removals", () => {
  assert.equal(hasStructuralEdgeRemovalChange([{
    id: "edge-1",
    selected: true,
    type: "select"
  }]), false);

  assert.equal(hasStructuralEdgeRemovalChange([{
    id: "edge-1",
    type: "remove"
  }]), true);
});
