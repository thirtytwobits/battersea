/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises flow node drop feedback behaviour in the editor's dataflow workspace.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { buildFlowNodeDropFailureNotification } from "@battersea/editor/core/flow-node-drop-feedback";

test("buildFlowNodeDropFailureNotification handles invalid drag payloads", () => {
  assert.deepEqual(
    buildFlowNodeDropFailureNotification({
      nodeTitle: "Multiplexer",
      reason: "invalid-payload"
    }),
    {
      title: "Couldn't add Multiplexer",
      message: "The dragged node data was invalid. Try dragging it in again."
    }
  );
});
