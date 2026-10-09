/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises visible direction marker placement for shared authoring graph edges.
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  createBridgeForSegment,
  resolveDirectionMarkerGeometry,
  resolveTargetBoundaryDirectionMarkerGeometry
} from "@battersea/editor/graph";
import type { AuthoringGraphWaypointSegment } from "@battersea/editor/graph";

test("direction marker resolves the midpoint and tangent of a straight edge", () => {
  const marker = resolveDirectionMarkerGeometry([createLineSegment(0, 0, 100, 0)], []);

  assert.deepEqual(marker?.position, { x: 50, y: 0 });
  assert.deepEqual(marker?.tangent, { x: 1, y: 0 });
  assert.equal(marker?.rotationDegrees, 0);
});

test("direction marker chooses the longest waypointed visible segment", () => {
  const marker = resolveDirectionMarkerGeometry([
    createLineSegment(0, 0, 40, 0),
    createLineSegment(40, 0, 200, 0)
  ], []);

  assert.deepEqual(marker?.position, { x: 120, y: 0 });
  assert.equal(marker?.rotationDegrees, 0);
});

test("direction marker avoids bridge gaps", () => {
  const marker = resolveDirectionMarkerGeometry([
    createLineSegment(0, 0, 300, 0)
  ], [createBridgeForSegment(0)]);

  assert.ok(marker);
  assert.notEqual(marker.position.x, 150);
  assert.ok(marker.position.x < 140 || marker.position.x > 160);
});

test("direction marker follows curved edge tangents", () => {
  const marker = resolveDirectionMarkerGeometry([{
    controlA: { x: 0, y: 90 },
    controlB: { x: 100, y: 90 },
    end: { x: 100, y: 0 },
    start: { x: 0, y: 0 }
  }], []);

  assert.deepEqual(marker?.position, { x: 50, y: 67.5 });
  assert.deepEqual(marker?.tangent, { x: 1, y: 0 });
  assert.equal(marker?.rotationDegrees, 0);
});

test("direction marker omits zero-length edges", () => {
  const marker = resolveDirectionMarkerGeometry([{
    controlA: { x: 12, y: 12 },
    controlB: { x: 12, y: 12 },
    end: { x: 12, y: 12 },
    start: { x: 12, y: 12 }
  }], []);

  assert.equal(marker, null);
});

test("target-boundary direction marker sits where the edge enters the target bounds", () => {
  const marker = resolveTargetBoundaryDirectionMarkerGeometry([createLineSegment(0, 0, 100, 0)], {
    center: { x: 100, y: 0 },
    height: 40,
    width: 40
  });

  assert.deepEqual(marker?.position, { x: 80, y: 0 });
  assert.deepEqual(marker?.tangent, { x: 1, y: 0 });
  assert.equal(marker?.rotationDegrees, 0);
});

test("target-boundary direction marker follows vertical target entries", () => {
  const marker = resolveTargetBoundaryDirectionMarkerGeometry([createLineSegment(100, -120, 100, 0)], {
    center: { x: 100, y: 0 },
    height: 40,
    width: 40
  });

  assert.deepEqual(marker?.position, { x: 100, y: -20 });
  assert.deepEqual(marker?.tangent, { x: 0, y: 1 });
  assert.equal(marker?.rotationDegrees, 90);
});

test("target-boundary direction marker honours rounded Place corners", () => {
  const marker = resolveTargetBoundaryDirectionMarkerGeometry([createLineSegment(0, 0, 100, 100)], {
    borderRadiusPx: 10,
    center: { x: 100, y: 100 },
    height: 40,
    width: 40
  });

  assert.ok(marker);
  assert.ok(marker.position.x > 82 && marker.position.x < 84);
  assert.ok(marker.position.y > 82 && marker.position.y < 84);
  assert.deepEqual(marker.tangent, { x: 0.707, y: 0.707 });
  assert.equal(marker.rotationDegrees, 45);
});

test("target-boundary direction marker omits edges that never reach the target bounds", () => {
  const marker = resolveTargetBoundaryDirectionMarkerGeometry([createLineSegment(0, 0, 40, 0)], {
    center: { x: 100, y: 0 },
    height: 40,
    width: 40
  });

  assert.equal(marker, null);
});

function createLineSegment(
  startX: number,
  startY: number,
  endX: number,
  endY: number
): AuthoringGraphWaypointSegment {
  return {
    controlA: {
      x: startX + ((endX - startX) / 3),
      y: startY + ((endY - startY) / 3)
    },
    controlB: {
      x: startX + (((endX - startX) * 2) / 3),
      y: startY + (((endY - startY) * 2) / 3)
    },
    end: { x: endX, y: endY },
    start: { x: startX, y: startY }
  };
}
