/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises the editor's edge marking against the runtime's and the runtime view's records.
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import type { Runtime } from "@battersea/flow";
import {
  applyFlowEdgeMarking,
  resolveFlowEdgeMarkingClassNames,
  type FlowActivityRecord,
  type FlowEdgeMarkingState
} from "@battersea/editor/core/flow-edge-marking";

function fixture<T>(path: string): T {
  return JSON.parse(readFileSync(new URL(`../../../crates/${path}`, import.meta.url), "utf8")) as T;
}

const run = fixture<{ records: FlowActivityRecord[] }>("battersea-runtime/fixtures/edge-marking.json");
const view = fixture<Runtime.RuntimeRecord[]>("battersea-telemetry/fixtures/edge-marking-view.json");

const ENDED: FlowEdgeMarkingState = {
  a: { queued: 0, close: { delivered: true, failed: false } },
  b: { queued: 0, close: { delivered: true, failed: false } }
};

function fold(records: readonly FlowActivityRecord[]): FlowEdgeMarkingState[] {
  const states: FlowEdgeMarkingState[] = [];
  let state: FlowEdgeMarkingState = {};
  for (const record of records) {
    state = applyFlowEdgeMarking(state, record);
    states.push(state);
  }
  return states;
}

test("the runtime's marking records and the view's marking record describe the same edges", () => {
  const fromRun = fold(run.records);
  const fromView = fold(
    view.map((record) => ({ phase: `runtime.${record.state.kind}`, detail: record.state }))
  );
  assert.deepEqual(fromRun.at(-1), ENDED);
  assert.deepEqual(fromView.at(-1), ENDED);
  assert.deepEqual(
    [...new Set(fromRun.map((state) => JSON.stringify(state)))],
    [...new Set(fromView.map((state) => JSON.stringify(state)))]
  );
});

test("each edge is drawn occupied, closing, then closed as the run moves through it", () => {
  const classes = fold(run.records).map((state) => ({
    a: resolveFlowEdgeMarkingClassNames(state.a).join(" "),
    go: resolveFlowEdgeMarkingClassNames(state.go).join(" ")
  }));
  const distinct = (edge: "a" | "go") =>
    classes.map((entry) => entry[edge]).filter((value, index, all) => value !== all[index - 1]);
  assert.deepEqual(distinct("a"), [
    "",
    "flow-studio-edge--occupied",
    "flow-studio-edge--occupied flow-studio-edge--closing",
    "flow-studio-edge--closing",
    "flow-studio-edge--closed"
  ]);
  assert.deepEqual(distinct("go"), ["", "flow-studio-edge--occupied", ""]);
});

test("a view resync replaces the marking and an execution record replaces the edges it names", () => {
  const before: FlowEdgeMarkingState = { a: { queued: 2 }, b: { queued: 1 } };
  assert.deepEqual(
    applyFlowEdgeMarking(before, {
      phase: "runtime.marking",
      detail: { kind: "marking", edges: { c: { queued: 1 } } }
    }),
    { c: { queued: 1 } }
  );
  assert.deepEqual(
    applyFlowEdgeMarking(before, {
      phase: "flow.edge.marking",
      detail: { edges: { a: { queued: 0 }, b: { queued: 0, close: { delivered: false, failed: true } } } }
    }),
    { b: { queued: 0, close: { delivered: false, failed: true } } }
  );
  assert.deepEqual(
    resolveFlowEdgeMarkingClassNames({ queued: 0, close: { delivered: false, failed: true } }),
    ["flow-studio-edge--closing", "flow-studio-edge--failed"]
  );
});

test("records that hold no marking leave it unchanged", () => {
  const state: FlowEdgeMarkingState = { a: { queued: 1 } };
  for (const record of [
    { phase: "flow.token.emit", detail: { targets: [{ edgeId: "a" }] } },
    { phase: "flow.edge.marking", detail: { edges: { a: { queued: -1 } } } },
    { phase: "flow.edge.marking", detail: "edges" },
    { phase: "runtime.port", detail: { kind: "port", port: "out" } }
  ]) {
    assert.equal(applyFlowEdgeMarking(state, record), state);
  }
});
