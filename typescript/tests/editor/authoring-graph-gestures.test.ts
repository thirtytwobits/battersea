/**
 * Copyright (c) Scott A Dixon
 *
 * Covers shared gesture lifecycle helpers for authoring-graph interactions.
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  createAuthoringGraphInteractiveGestureSession,
  resolveAuthoringGraphEdgeGestureLabel
} from "@battersea/editor/graph";

test("gesture session ignores duplicate begin calls and commits the active gesture once", () => {
  const calls: string[] = [];
  const session = createAuthoringGraphInteractiveGestureSession();
  const driver = {
    beginInteractiveGesture: (label: string) => {
      calls.push(`begin:${label}`);
    },
    cancelInteractiveGesture: () => {
      calls.push("cancel");
    },
    commitInteractiveGesture: () => {
      calls.push("commit");
    }
  };

  session.beginGesture(driver, "Move bridge");
  session.beginGesture(driver, "Move bridge");
  session.commitGesture(driver);
  session.commitGesture(driver);

  assert.equal(session.getActiveLabel(), null);
  assert.deepEqual(calls, [
    "begin:Move bridge",
    "commit"
  ]);
});

test("gesture session cancels the active gesture before replacing it deterministically", () => {
  const calls: string[] = [];
  const session = createAuthoringGraphInteractiveGestureSession();
  const driver = {
    beginInteractiveGesture: (label: string) => {
      calls.push(`begin:${label}`);
    },
    cancelInteractiveGesture: () => {
      calls.push("cancel");
    },
    commitInteractiveGesture: () => {
      calls.push("commit");
    }
  };

  session.beginGesture(driver, "Move bridge");
  session.beginGesture(driver, "Resize bridge");
  session.cancelGesture(driver);
  session.cancelGesture(driver);

  assert.equal(session.getActiveLabel(), null);
  assert.deepEqual(calls, [
    "begin:Move bridge",
    "cancel",
    "begin:Resize bridge",
    "cancel"
  ]);
});

test("edge gesture labels resolve to the expected shared undo labels", () => {
  assert.equal(resolveAuthoringGraphEdgeGestureLabel("anchor"), "Move waypoint");
  assert.equal(resolveAuthoringGraphEdgeGestureLabel("bridge"), "Move bridge");
  assert.equal(resolveAuthoringGraphEdgeGestureLabel("bridgeGap"), "Resize bridge");
  assert.equal(resolveAuthoringGraphEdgeGestureLabel("inHandle"), "Move waypoint handle");
  assert.equal(resolveAuthoringGraphEdgeGestureLabel("outHandle"), "Move waypoint handle");
});
