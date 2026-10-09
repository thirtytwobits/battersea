/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises the shared authoring-graph handle descriptor and connection helpers.
 */
import assert from "node:assert/strict";
import test from "node:test";
import { Position } from "@xyflow/react";

import {
  getAuthoringGraphHandlePosition,
  indexAuthoringGraphHandles,
  inspectAuthoringGraphConnection
} from "@battersea/editor/graph";

const DESCRIPTORS = [
  {
    direction: "target",
    family: "containment",
    handleId: "containment-target",
    side: "top"
  },
  {
    direction: "source",
    family: "containment",
    handleId: "containment-source",
    side: "bottom"
  },
  {
    direction: "target",
    family: "link",
    handleId: "link-target",
    side: "left"
  },
  {
    direction: "source",
    family: "link",
    handleId: "link-source",
    side: "right"
  }
] as const;

test("indexAuthoringGraphHandles resolves descriptors by id, side, family, and direction", () => {
  const index = indexAuthoringGraphHandles(DESCRIPTORS);

  assert.equal(index.byId.get("containment-source")?.family, "containment");
  assert.deepEqual(
    index.bySide.get("left")?.map((descriptor) => descriptor.handleId),
    ["link-target"]
  );
  assert.deepEqual(
    index.byFamily.get("link")?.map((descriptor) => descriptor.handleId),
    ["link-target", "link-source"]
  );
  assert.deepEqual(
    index.byDirection.get("source")?.map((descriptor) => descriptor.handleId),
    ["containment-source", "link-source"]
  );
});

test("inspectAuthoringGraphConnection rejects missing endpoints, family mismatches, and direction mismatches", () => {
  assert.deepEqual(
    inspectAuthoringGraphConnection({
      connection: {
        source: null,
        sourceHandle: "containment-source",
        target: "place-2",
        targetHandle: "containment-target"
      },
      sourceHandles: DESCRIPTORS,
      targetHandles: DESCRIPTORS
    }),
    {
      reason: "missing_endpoints",
      valid: false
    }
  );

  assert.deepEqual(
    inspectAuthoringGraphConnection({
      connection: {
        source: "place-1",
        sourceHandle: "containment-source",
        target: "place-2",
        targetHandle: "link-target"
      },
      sourceHandles: DESCRIPTORS,
      targetHandles: DESCRIPTORS
    }),
    {
      reason: "family_mismatch",
      valid: false
    }
  );

  assert.deepEqual(
    inspectAuthoringGraphConnection({
      connection: {
        source: "place-1",
        sourceHandle: "containment-target",
        target: "place-2",
        targetHandle: "containment-source"
      },
      sourceHandles: DESCRIPTORS,
      targetHandles: DESCRIPTORS
    }),
    {
      reason: "source_direction_mismatch",
      valid: false
    }
  );
});

test("shared handle helpers support non-left-to-right connections", () => {
  const result = inspectAuthoringGraphConnection({
    allowSelfLoops: false,
    connection: {
      source: "north",
      sourceHandle: "containment-source",
      target: "centre",
      targetHandle: "containment-target"
    },
    sourceHandles: DESCRIPTORS,
    targetHandles: DESCRIPTORS
  });

  assert.equal(result.valid, true);
  if (!result.valid) {
    return;
  }

  assert.equal(result.connection.family, "containment");
  assert.equal(result.connection.sourceHandle.side, "bottom");
  assert.equal(result.connection.targetHandle.side, "top");
  assert.equal(getAuthoringGraphHandlePosition(result.connection.sourceHandle.side), Position.Bottom);
  assert.equal(getAuthoringGraphHandlePosition(result.connection.targetHandle.side), Position.Top);
});
