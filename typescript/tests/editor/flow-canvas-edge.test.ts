/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises hover-retention helpers for the dataflow canvas edge affordances.
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  eventTargetMatchesWaypointHoverSurface,
  stopWaypointSurfacePropagation
} from "@battersea/editor/graph";

test("shared editable-edge hover helpers detect transitions onto the waypoint add affordance", () => {
  const target = {
    closest: (selector: string) => selector === ".flow-studio-edge__add-button-shell"
      ? {}
      : null
  } as unknown as EventTarget;

  assert.equal(
    eventTargetMatchesWaypointHoverSurface(target, ".flow-studio-edge__add-button-shell"),
    true
  );
});

test("shared editable-edge hover helpers ignore unrelated transition targets", () => {
  const unrelatedTarget = {
    closest: () => null
  } as unknown as EventTarget;

  assert.equal(
    eventTargetMatchesWaypointHoverSurface(unrelatedTarget, ".flow-studio-edge__add-button-shell"),
    false
  );
  assert.equal(
    eventTargetMatchesWaypointHoverSurface(null, ".flow-studio-edge__add-button-shell"),
    false
  );
});

test("shared editable-edge hover helpers consume waypoint surface events", () => {
  let prevented = false;
  let stopped = false;
  let stoppedImmediately = false;

  stopWaypointSurfacePropagation({
    nativeEvent: {
      stopImmediatePropagation: () => {
        stoppedImmediately = true;
      }
    } as Event,
    preventDefault: () => {
      prevented = true;
    },
    stopPropagation: () => {
      stopped = true;
    }
  });

  assert.equal(prevented, true);
  assert.equal(stopped, true);
  assert.equal(stoppedImmediately, true);
});
