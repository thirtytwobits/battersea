/**
 * Copyright (c) Scott A Dixon
 *
 * Verifies the shared gesture driver adapter used by authoring edge interactions.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { createAuthoringInteractiveGestureDriver } from "@battersea/editor/graph";

test("shared gesture driver captures once on begin and commits the buffered snapshot", () => {
  const snapshot = { edges: [], nodes: [] };
  const commits: Array<{ label: string; snapshot: unknown; }> = [];
  let captureCount = 0;
  const driver = createAuthoringInteractiveGestureDriver({
    capture: () => {
      captureCount += 1;
      return snapshot;
    },
    commit: (label, previousSnapshot) => {
      commits.push({ label, snapshot: previousSnapshot });
    }
  });

  driver.beginInteractiveGesture("Move bridge");
  driver.commitInteractiveGesture();

  assert.equal(captureCount, 1);
  assert.deepEqual(commits, [{
    label: "Move bridge",
    snapshot
  }]);
});

test("shared gesture driver cancel clears the buffered gesture without committing", () => {
  const driver = createAuthoringInteractiveGestureDriver({
    capture: () => ({ edges: [], nodes: [] }),
    commit: () => {
      assert.fail("cancelled gestures must not commit undo entries");
    }
  });

  driver.beginInteractiveGesture("Move waypoint");
  driver.cancelInteractiveGesture();
  driver.commitInteractiveGesture();
});

test("shared gesture driver replaces the buffered gesture when a new begin arrives", () => {
  const snapshots = [
    { id: "first" },
    { id: "second" }
  ];
  const commits: Array<{ label: string; snapshot: unknown; }> = [];
  const driver = createAuthoringInteractiveGestureDriver({
    capture: () => {
      const nextSnapshot = snapshots.shift();
      assert.ok(nextSnapshot);
      return nextSnapshot;
    },
    commit: (label, snapshot) => {
      commits.push({ label, snapshot });
    }
  });

  driver.beginInteractiveGesture("Move bridge");
  driver.beginInteractiveGesture("Resize bridge");
  driver.commitInteractiveGesture();

  assert.deepEqual(commits, [{
    label: "Resize bridge",
    snapshot: { id: "second" }
  }]);
});
