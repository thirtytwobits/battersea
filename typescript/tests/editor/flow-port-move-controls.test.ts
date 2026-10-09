/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises non-drag flow node port move-control helpers.
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveFlowPortMoveControlState,
  resolveFlowPortMoveTargetIndex
} from "@battersea/editor/core/flow-port-move-controls";
import type { FlowStudioResolvedPort } from "@battersea/editor/core/flow-node-ports";

const INPUT_PORTS: readonly FlowStudioResolvedPort[] = [
  { displayClass: "inline", id: "input-2", label: "Input 2", side: "input" },
  { displayClass: "inline", id: "input-0", label: "Input 0", side: "input" },
  { displayClass: "inline", id: "input-1", label: "Input 1", side: "input" }
];

test("resolveFlowPortMoveControlState resolves the selected port index from the current ordered side", () => {
  const moveState = resolveFlowPortMoveControlState({
    portId: "input-0",
    ports: INPUT_PORTS,
    side: "input"
  });

  assert.equal(moveState?.kind, "reorder");
  assert.equal(moveState?.currentIndex, 1);
  if (moveState?.kind !== "reorder") {
    assert.fail("Expected reorder control state");
  }
  assert.equal(moveState.canMoveTowardStart, true);
  assert.equal(moveState.canMoveTowardEnd, true);
});

test("resolveFlowPortMoveControlState disables boundary moves and maps icons by side orientation", () => {
  const firstInput = resolveFlowPortMoveControlState({
    portId: "input-2",
    ports: INPUT_PORTS,
    side: "input"
  });
  const lastSignal = resolveFlowPortMoveControlState({
    portId: "input-1",
    ports: INPUT_PORTS.map((port) => ({
      ...port,
      side: "signal" as const
    })),
    side: "signal"
  });

  assert.equal(firstInput?.kind, "reorder");
  if (firstInput?.kind !== "reorder") {
    assert.fail("Expected input reorder control state");
  }
  assert.equal(firstInput?.canMoveTowardStart, false);
  assert.equal(firstInput.startIcon, "arrow-up");
  assert.equal(firstInput.endIcon, "arrow-down");
  assert.equal(lastSignal?.kind, "swap-side");
  if (lastSignal?.kind !== "swap-side") {
    assert.fail("Expected signal swap control state");
  }
  assert.equal(lastSignal.direction, "toward-end");
  assert.equal(lastSignal.icon, "arrow-down");
});

test("resolveFlowPortMoveControlState maps swapped controller ports to the opposite side affordance", () => {
  const swappedAction = resolveFlowPortMoveControlState({
    controllerPortPlacement: "swapped",
    portId: "input-2",
    ports: INPUT_PORTS.map((port) => ({
      ...port,
      side: "action" as const
    })),
    side: "action"
  });

  assert.equal(swappedAction?.kind, "swap-side");
  if (swappedAction?.kind !== "swap-side") {
    assert.fail("Expected action swap control state");
  }
  assert.equal(swappedAction.direction, "toward-end");
  assert.equal(swappedAction.icon, "arrow-down");
});

test("resolveFlowPortMoveControlState rotates instrument controller ports onto the left and right sides", () => {
  const defaultInstrumentAction = resolveFlowPortMoveControlState({
    nodeClass: "instrument",
    portId: "input-2",
    ports: INPUT_PORTS.map((port) => ({
      ...port,
      side: "action" as const
    })),
    side: "action"
  });
  const swappedInstrumentSignal = resolveFlowPortMoveControlState({
    controllerPortPlacement: "swapped",
    nodeClass: "instrument",
    portId: "input-1",
    ports: INPUT_PORTS.map((port) => ({
      ...port,
      side: "signal" as const
    })),
    side: "signal"
  });

  assert.equal(defaultInstrumentAction?.kind, "swap-side");
  if (defaultInstrumentAction?.kind !== "swap-side") {
    assert.fail("Expected instrument action swap control state");
  }
  assert.equal(defaultInstrumentAction.direction, "toward-start");
  assert.equal(defaultInstrumentAction.icon, "arrow-left");
  assert.equal(defaultInstrumentAction.label, "Swap left and right controller ports");

  assert.equal(swappedInstrumentSignal?.kind, "swap-side");
  if (swappedInstrumentSignal?.kind !== "swap-side") {
    assert.fail("Expected instrument signal swap control state");
  }
  assert.equal(swappedInstrumentSignal.direction, "toward-start");
  assert.equal(swappedInstrumentSignal.icon, "arrow-left");
});

test("resolveFlowPortMoveTargetIndex moves one slot toward start or end and ignores invalid moves", () => {
  assert.equal(resolveFlowPortMoveTargetIndex({
    currentIndex: 1,
    direction: "toward-start",
    portCount: 3
  }), 0);
  assert.equal(resolveFlowPortMoveTargetIndex({
    currentIndex: 1,
    direction: "toward-end",
    portCount: 3
  }), 2);
  assert.equal(resolveFlowPortMoveTargetIndex({
    currentIndex: 0,
    direction: "toward-start",
    portCount: 3
  }), null);
});
