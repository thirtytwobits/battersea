/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises flow control layout behaviour in the editor's dataflow workspace.
 */
import assert from "node:assert/strict";
import test from "node:test";
import type { FlowParameterDefinition as WireFlowParameterDefinition } from "@battersea/flow";
import { groupControlsForDetailPane } from "@battersea/editor/core/flow-control-layout";

function makeParameter(
  name: string,
  kind: WireFlowParameterDefinition["editor"]["kind"]
): WireFlowParameterDefinition {
  const datatype
    = kind === "list"
      ? {
        item_type: { kind: "string" as const },
        kind: "list" as const
      }
      : kind === "string" || kind === "enum"
        ? { kind: "string" as const }
        : { kind: "int" as const };

  return {
    datatype,
    editor: {
      kind
    },
    name
  };
}

test("groupControlsForDetailPane keeps input and output port-count parameters side-by-side", () => {
  const groups = groupControlsForDetailPane([
    makeParameter("input_ports", "input_port_count"),
    makeParameter("output_ports", "output_port_count"),
    makeParameter("ordering", "enum")
  ]);

  assert.deepEqual(groups, [
    {
      layout: "row",
      parameters: [makeParameter("input_ports", "input_port_count"), makeParameter("output_ports", "output_port_count")]
    },
    {
      layout: "stack",
      parameters: [makeParameter("ordering", "enum")]
    }
  ]);
});

test("groupControlsForDetailPane leaves unmatched port-count parameters stacked", () => {
  const groups = groupControlsForDetailPane([
    makeParameter("input_ports", "input_port_count"),
    makeParameter("ordering", "enum")
  ]);

  assert.deepEqual(groups, [
    {
      layout: "stack",
      parameters: [makeParameter("input_ports", "input_port_count")]
    },
    {
      layout: "stack",
      parameters: [makeParameter("ordering", "enum")]
    }
  ]);
});

test("groupControlsForDetailPane groups shared formatting parameters into one stack", () => {
  const groups = groupControlsForDetailPane([
    makeParameter("ordering", "enum"),
    makeParameter("whitespace_mode", "enum"),
    makeParameter("output_encoding", "enum"),
    makeParameter("plain_fragment_delimiter", "enum"),
    makeParameter("empty_input_rule", "enum")
  ]);

  assert.deepEqual(groups, [
    {
      layout: "stack",
      parameters: [makeParameter("ordering", "enum")]
    },
    {
      layout: "stack",
      parameters: [
        makeParameter("output_encoding", "enum"),
        makeParameter("whitespace_mode", "enum"),
        makeParameter("plain_fragment_delimiter", "enum")
      ]
    },
    {
      layout: "stack",
      parameters: [makeParameter("empty_input_rule", "enum")]
    }
  ]);
});
