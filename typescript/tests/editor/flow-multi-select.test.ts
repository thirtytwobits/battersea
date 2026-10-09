/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises flow multi select behaviour in the editor's dataflow workspace.
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  flowMultiSelectHasAllValues,
  selectAllFlowMultiSelectValues,
  selectNoFlowMultiSelectValues
} from "@battersea/editor/core/flow-multi-select";

test("selectAllFlowMultiSelectValues returns every option value once in order", () => {
  assert.deepEqual(
    selectAllFlowMultiSelectValues([
      { label: "Alpha", value: "a" },
      { label: "Bravo", value: "b" },
      { label: "Alpha duplicate", value: "a" }
    ]),
    ["a", "b"]
  );
});

test("selectNoFlowMultiSelectValues clears all selections", () => {
  assert.deepEqual(selectNoFlowMultiSelectValues(), []);
});

test("flowMultiSelectHasAllValues only reports true when every option is selected", () => {
  const options = [
    { label: "Alpha", value: "a" },
    { label: "Bravo", value: "b" }
  ];

  assert.equal(flowMultiSelectHasAllValues(options, ["a"]), false);
  assert.equal(flowMultiSelectHasAllValues(options, ["a", "b"]), true);
  assert.equal(flowMultiSelectHasAllValues([], []), false);
});
