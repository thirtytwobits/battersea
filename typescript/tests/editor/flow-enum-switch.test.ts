/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises flow enum switch behaviour in the editor's dataflow workspace.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { flowEnumUsesSwitch } from "@battersea/editor/core/flow-enum-switch";

const ORDERING_OPTIONS = [
  { label: "Descending", value: "descending" },
  { label: "Ascending", value: "ascending" }
];

test("flowEnumUsesSwitch only enables the switch pattern for binary two-option enums", () => {
  assert.equal(flowEnumUsesSwitch([
    { label: "Disabled", value: "disabled" },
    { label: "Enabled", value: "enabled" }
  ]), true);
  assert.equal(flowEnumUsesSwitch([
    { label: "Off", value: "0" },
    { label: "On", value: "1" }
  ]), true);
  assert.equal(flowEnumUsesSwitch(ORDERING_OPTIONS), false);
  assert.equal(flowEnumUsesSwitch([{ label: "Only", value: "only" }]), false);
  assert.equal(flowEnumUsesSwitch([
    { label: "One", value: "one" },
    { label: "Two", value: "two" },
    { label: "Three", value: "three" }
  ]), false);
});
