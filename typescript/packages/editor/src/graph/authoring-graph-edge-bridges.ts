/**
 * Copyright (c) Scott A Dixon
 *
 * Builds and mutates manual edge bridge geometry for authoring-graph edges.
 */
import type {
  AuthoringGraphBridge,
  AuthoringGraphEdgeLayout,
  AuthoringGraphPoint,
} from "./authoring-graph-edge-types.js";
import type { AuthoringGraphWaypointSegment } from "./authoring-graph-edge-waypoints.js";
import { evaluateAuthoringGraphCubicBezierPoint } from "./authoring-graph-edge-waypoints.js";

const BRIDGE_CAP_HALF_LENGTH_PX = 5;
const BRIDGE_GAP_HANDLE_OFFSET_PX = 12;
const DEFAULT_BRIDGE_GAP_PX = 14;
const MAX_BRIDGE_GAP_PX = 64;
const MIN_BRIDGE_GAP_PX = 6;
const PATH_GAP_SAMPLE_COUNT = 160;
const NEAREST_BRIDGE_SAMPLE_COUNT = 96;

export interface AuthoringGraphBridgeLocation {
  segmentIndex: number;
  t: number;
}

export interface AuthoringGraphInsertionInterval {
  endT: number;
  segmentIndex: number;
  startT: number;
}

export interface AuthoringGraphBridgeGapInterval {
  endT: number;
  segmentIndex: number;
  startT: number;
}

export interface ResolvedAuthoringGraphBridgeGeometry {
  center: AuthoringGraphPoint;
  endCapEnd: AuthoringGraphPoint;
  endCapStart: AuthoringGraphPoint;
  gapEnd: AuthoringGraphPoint;
  gapStart: AuthoringGraphPoint;
  normal: AuthoringGraphPoint;
  sizeHandle: AuthoringGraphPoint;
  startCapEnd: AuthoringGraphPoint;
  startCapStart: AuthoringGraphPoint;
  tangent: AuthoringGraphPoint;
}

export interface ResolvedAuthoringGraphDirectionMarkerGeometry {
  position: AuthoringGraphPoint;
  rotationDegrees: number;
  tangent: AuthoringGraphPoint;
}

export interface AuthoringGraphDirectionMarkerBoundary {
  borderRadiusPx?: number;
  center: AuthoringGraphPoint;
  height: number;
  width: number;
}

export function createBridgeForInterval(
  interval: AuthoringGraphInsertionInterval,
): AuthoringGraphBridge {
  return {
    gap: DEFAULT_BRIDGE_GAP_PX,
    segmentIndex: interval.segmentIndex,
    t: resolveIntervalMidT(interval),
  };
}

export function createBridgeForSegment(
  segmentIndex: number,
): AuthoringGraphBridge {
  return createBridgeForInterval({
    endT: 1,
    segmentIndex,
    startT: 0,
  });
}

export function insertBridgeIntoLayout(
  layout: AuthoringGraphEdgeLayout | undefined,
  bridge: AuthoringGraphBridge,
): AuthoringGraphEdgeLayout {
  return {
    bridges: [...(layout?.bridges ?? []), normalizeBridge(bridge)],
    waypoints: layout?.waypoints,
  };
}

export function moveBridgeInLayout(
  layout: AuthoringGraphEdgeLayout | undefined,
  bridgeIndex: number,
  location: AuthoringGraphBridgeLocation,
): AuthoringGraphEdgeLayout {
  const bridges = layout?.bridges ?? [];
  if (!bridges[bridgeIndex]) {
    return layout ?? {};
  }

  return patchBridge(layout, bridgeIndex, {
    ...bridges[bridgeIndex],
    segmentIndex: location.segmentIndex,
    t: roundUnitInterval(location.t),
  });
}

export function resizeBridgeInLayout(
  layout: AuthoringGraphEdgeLayout | undefined,
  bridgeIndex: number,
  gap: number,
): AuthoringGraphEdgeLayout {
  const bridges = layout?.bridges ?? [];
  const bridge = bridges[bridgeIndex];
  if (!bridge) {
    return layout ?? {};
  }

  return patchBridge(layout, bridgeIndex, {
    ...bridge,
    gap: normalizeBridgeGap(gap),
  });
}

export function removeBridgeFromLayout(
  layout: AuthoringGraphEdgeLayout | undefined,
  bridgeIndex: number,
): AuthoringGraphEdgeLayout {
  const bridges = layout?.bridges ?? [];
  if (!bridges[bridgeIndex]) {
    return layout ?? {};
  }

  return {
    bridges: [
      ...bridges.slice(0, bridgeIndex),
      ...bridges.slice(bridgeIndex + 1),
    ],
    waypoints: layout?.waypoints,
  };
}

export function resolveNearestBridgeLocation(
  segments: readonly AuthoringGraphWaypointSegment[],
  point: AuthoringGraphPoint,
): AuthoringGraphBridgeLocation | null {
  if (segments.length === 0) {
    return null;
  }

  let best: AuthoringGraphBridgeLocation | null = null;
  let bestDistance = Number.POSITIVE_INFINITY;

  segments.forEach((segment, segmentIndex) => {
    for (
      let sampleIndex = 0;
      sampleIndex <= NEAREST_BRIDGE_SAMPLE_COUNT;
      sampleIndex += 1
    ) {
      const t = sampleIndex / NEAREST_BRIDGE_SAMPLE_COUNT;
      const sample = evaluateAuthoringGraphCubicBezierPoint(segment, t);
      const distance = squaredDistance(sample, point);
      if (distance < bestDistance) {
        bestDistance = distance;
        best = {
          segmentIndex,
          t: roundUnitInterval(t),
        };
      }
    }
  });

  return best;
}

export function resolveBridgeGeometry(
  segments: readonly AuthoringGraphWaypointSegment[],
  bridge: AuthoringGraphBridge,
): ResolvedAuthoringGraphBridgeGeometry | null {
  const segment = segments[bridge.segmentIndex];
  if (!segment) {
    return null;
  }

  const center = evaluateAuthoringGraphCubicBezierPoint(segment, bridge.t);
  const tangent = normalisePoint(
    evaluateCubicBezierDerivative(segment, bridge.t),
  );
  const rawNormal = { x: -tangent.y, y: tangent.x };
  const normal =
    rawNormal.y > 0 ? { x: -rawNormal.x, y: -rawNormal.y } : rawNormal;
  const gap = normalizeBridgeGap(bridge.gap);
  const capOffset = {
    x: roundNumber(normal.x * BRIDGE_CAP_HALF_LENGTH_PX),
    y: roundNumber(normal.y * BRIDGE_CAP_HALF_LENGTH_PX),
  };
  const gapStart = {
    x: roundNumber(center.x - tangent.x * gap),
    y: roundNumber(center.y - tangent.y * gap),
  };
  const gapEnd = {
    x: roundNumber(center.x + tangent.x * gap),
    y: roundNumber(center.y + tangent.y * gap),
  };

  return {
    center,
    endCapEnd: {
      x: roundNumber(gapEnd.x - capOffset.x),
      y: roundNumber(gapEnd.y - capOffset.y),
    },
    endCapStart: {
      x: roundNumber(gapEnd.x + capOffset.x),
      y: roundNumber(gapEnd.y + capOffset.y),
    },
    gapEnd,
    gapStart,
    normal,
    sizeHandle: {
      x: roundNumber(center.x + normal.x * (gap + BRIDGE_GAP_HANDLE_OFFSET_PX)),
      y: roundNumber(center.y + normal.y * (gap + BRIDGE_GAP_HANDLE_OFFSET_PX)),
    },
    startCapEnd: {
      x: roundNumber(gapStart.x - capOffset.x),
      y: roundNumber(gapStart.y - capOffset.y),
    },
    startCapStart: {
      x: roundNumber(gapStart.x + capOffset.x),
      y: roundNumber(gapStart.y + capOffset.y),
    },
    tangent,
  };
}

export function buildVisibleEdgePathWithBridges(
  segments: readonly AuthoringGraphWaypointSegment[],
  bridges: readonly AuthoringGraphBridge[],
): string {
  if (segments.length === 0) {
    return "";
  }

  if (bridges.length === 0) {
    return buildSegmentPath(segments);
  }

  const gapIntervals = resolveBridgeGapIntervals(segments, bridges);

  return segments
    .flatMap((segment, segmentIndex) => {
      const hiddenIntervals = gapIntervals
        .filter((interval) => interval.segmentIndex === segmentIndex)
        .sort((left, right) => left.startT - right.startT);

      if (hiddenIntervals.length === 0) {
        return [segment];
      }

      const visibleSegments: AuthoringGraphWaypointSegment[] = [];
      let cursor = 0;

      hiddenIntervals.forEach((interval) => {
        if (interval.startT > cursor + 0.0001) {
          visibleSegments.push(
            extractBezierSubsegment(segment, cursor, interval.startT),
          );
        }

        cursor = Math.max(cursor, interval.endT);
      });

      if (cursor < 1 - 0.0001) {
        visibleSegments.push(extractBezierSubsegment(segment, cursor, 1));
      }

      return visibleSegments;
    })
    .map(
      (segment) =>
        `M ${formatPoint(segment.start)} C ${formatPoint(segment.controlA)}, ${formatPoint(segment.controlB)}, ${formatPoint(segment.end)}`,
    )
    .join(" ");
}

export function resolveBridgeGapIntervals(
  segments: readonly AuthoringGraphWaypointSegment[],
  bridges: readonly AuthoringGraphBridge[],
): AuthoringGraphBridgeGapInterval[] {
  return bridges
    .map((bridge) => {
      const geometry = resolveBridgeGeometry(segments, bridge);
      if (!geometry) {
        return null;
      }

      const segment = segments[bridge.segmentIndex];
      if (!segment) {
        return null;
      }

      const startT = findNearestTAlongSegment(
        segment,
        geometry.gapStart,
        bridge.t,
        -1,
      );
      const endT = findNearestTAlongSegment(
        segment,
        geometry.gapEnd,
        bridge.t,
        1,
      );

      return {
        endT,
        segmentIndex: bridge.segmentIndex,
        startT,
      };
    })
    .filter(
      (interval): interval is AuthoringGraphBridgeGapInterval =>
        interval !== null,
    );
}

export function buildEdgeInsertionIntervals(
  segments: readonly AuthoringGraphWaypointSegment[],
  bridges: readonly AuthoringGraphBridge[],
): AuthoringGraphInsertionInterval[] {
  return segments.flatMap((_, segmentIndex) => {
    const bridgeTs = bridges
      .filter((bridge) => bridge.segmentIndex === segmentIndex)
      .map((bridge) => roundUnitInterval(bridge.t))
      .sort((left, right) => left - right);
    const bounds = [0, ...bridgeTs, 1];

    return bounds.slice(0, -1).flatMap((startT, index) => {
      const endT = bounds[index + 1]!;
      return endT - startT <= 0.0001
        ? []
        : [
            {
              endT,
              segmentIndex,
              startT,
            },
          ];
    });
  });
}

export function resolveInsertionIntervalMidpoint(
  segments: readonly AuthoringGraphWaypointSegment[],
  interval: AuthoringGraphInsertionInterval,
): AuthoringGraphPoint | null {
  const segment = segments[interval.segmentIndex];
  if (!segment) {
    return null;
  }

  return evaluateAuthoringGraphCubicBezierPoint(
    segment,
    resolveIntervalMidT(interval),
  );
}

export function resolveDirectionMarkerGeometry(
  segments: readonly AuthoringGraphWaypointSegment[],
  bridges: readonly AuthoringGraphBridge[],
): ResolvedAuthoringGraphDirectionMarkerGeometry | null {
  const interval = resolveLongestVisibleInterval(segments, bridges);
  if (!interval) {
    return null;
  }

  const segment = segments[interval.segmentIndex];
  if (!segment) {
    return null;
  }

  const t = resolveIntervalMidT(interval);
  const position = evaluateAuthoringGraphCubicBezierPoint(segment, t);
  const tangent = resolveMarkerTangent(segment, t);
  if (!tangent) {
    return null;
  }

  return {
    position,
    rotationDegrees: roundNumber(
      (Math.atan2(tangent.y, tangent.x) * 180) / Math.PI,
    ),
    tangent,
  };
}

export function resolveTargetBoundaryDirectionMarkerGeometry(
  segments: readonly AuthoringGraphWaypointSegment[],
  boundary: AuthoringGraphDirectionMarkerBoundary,
): ResolvedAuthoringGraphDirectionMarkerGeometry | null {
  if (
    segments.length === 0 ||
    !Number.isFinite(boundary.width) ||
    !Number.isFinite(boundary.height) ||
    boundary.width <= 0 ||
    boundary.height <= 0
  ) {
    return null;
  }

  const normalisedBoundary = {
    ...boundary,
    borderRadiusPx: normalizeBoundaryRadius(boundary),
  };
  const sampleCount = 80;

  for (
    let segmentIndex = segments.length - 1;
    segmentIndex >= 0;
    segmentIndex -= 1
  ) {
    const segment = segments[segmentIndex]!;
    let insideT: number | null = isPointInsideRoundedBoundary(
      segment.end,
      normalisedBoundary,
    )
      ? 1
      : null;

    for (
      let sampleIndex = sampleCount - 1;
      sampleIndex >= 0;
      sampleIndex -= 1
    ) {
      const t = sampleIndex / sampleCount;
      const point = evaluateAuthoringGraphCubicBezierPoint(segment, t);

      if (isPointInsideRoundedBoundary(point, normalisedBoundary)) {
        insideT = t;
        continue;
      }

      if (insideT === null) {
        continue;
      }

      return resolveBoundaryIntersectionGeometry(
        segment,
        t,
        insideT,
        normalisedBoundary,
      );
    }
  }

  return null;
}

export function resolveNearestInsertionInterval(
  segments: readonly AuthoringGraphWaypointSegment[],
  bridges: readonly AuthoringGraphBridge[],
  point: AuthoringGraphPoint,
): AuthoringGraphInsertionInterval | null {
  const intervals = buildEdgeInsertionIntervals(segments, bridges);
  if (intervals.length === 0) {
    return null;
  }

  let bestInterval: AuthoringGraphInsertionInterval | null = null;
  let bestDistance = Number.POSITIVE_INFINITY;

  intervals.forEach((interval) => {
    const segment = segments[interval.segmentIndex];
    if (!segment) {
      return;
    }

    for (
      let sampleIndex = 0;
      sampleIndex <= NEAREST_BRIDGE_SAMPLE_COUNT;
      sampleIndex += 1
    ) {
      const interpolation = sampleIndex / NEAREST_BRIDGE_SAMPLE_COUNT;
      const t =
        interval.startT + (interval.endT - interval.startT) * interpolation;
      const sample = evaluateAuthoringGraphCubicBezierPoint(segment, t);
      const distance = squaredDistance(sample, point);
      if (distance < bestDistance) {
        bestDistance = distance;
        bestInterval = interval;
      }
    }
  });

  return bestInterval;
}

export function updateBridgeIndicesForWaypointInsertion(
  bridges: readonly AuthoringGraphBridge[] | undefined,
  insertedSegmentIndex: number,
  insertedT: number,
): AuthoringGraphBridge[] | undefined {
  if (!bridges || bridges.length === 0) {
    return bridges ? [] : undefined;
  }

  return bridges.map((bridge) =>
    bridge.segmentIndex === insertedSegmentIndex
      ? bridge.t <= insertedT
        ? {
            ...bridge,
            t: roundUnitInterval(insertedT <= 0 ? 0 : bridge.t / insertedT),
          }
        : {
            ...bridge,
            segmentIndex: bridge.segmentIndex + 1,
            t: roundUnitInterval(
              (bridge.t - insertedT) / Math.max(1 - insertedT, 0.0001),
            ),
          }
      : bridge.segmentIndex > insertedSegmentIndex
        ? {
            ...bridge,
            segmentIndex: bridge.segmentIndex + 1,
          }
        : bridge,
  );
}

export function updateBridgeIndicesForWaypointRemoval(
  bridges: readonly AuthoringGraphBridge[] | undefined,
  removedWaypointIndex: number,
): AuthoringGraphBridge[] | undefined {
  if (!bridges || bridges.length === 0) {
    return bridges ? [] : undefined;
  }

  return bridges.map((bridge) => {
    if (bridge.segmentIndex > removedWaypointIndex) {
      return {
        ...bridge,
        segmentIndex: bridge.segmentIndex - 1,
      };
    }

    if (bridge.segmentIndex === removedWaypointIndex) {
      return {
        ...bridge,
        t: roundUnitInterval(bridge.t * 0.5),
      };
    }

    return bridge;
  });
}

export function resolveBridgeGapFromHandle(
  geometry: ResolvedAuthoringGraphBridgeGeometry,
  handlePosition: AuthoringGraphPoint,
): number {
  const gap =
    Math.abs(
      (handlePosition.x - geometry.center.x) * geometry.normal.x +
        (handlePosition.y - geometry.center.y) * geometry.normal.y,
    ) - BRIDGE_GAP_HANDLE_OFFSET_PX;

  return normalizeBridgeGap(gap);
}

function patchBridge(
  layout: AuthoringGraphEdgeLayout | undefined,
  bridgeIndex: number,
  bridge: AuthoringGraphBridge,
): AuthoringGraphEdgeLayout {
  const bridges = layout?.bridges ?? [];

  return {
    bridges: bridges.map((entry, index) =>
      index === bridgeIndex ? normalizeBridge(bridge) : entry,
    ),
    waypoints: layout?.waypoints,
  };
}

function buildSegmentPath(
  segments: readonly AuthoringGraphWaypointSegment[],
): string {
  return segments
    .map((segment, index) =>
      index === 0
        ? `M ${formatPoint(segment.start)} C ${formatPoint(segment.controlA)}, ${formatPoint(segment.controlB)}, ${formatPoint(segment.end)}`
        : `C ${formatPoint(segment.controlA)}, ${formatPoint(segment.controlB)}, ${formatPoint(segment.end)}`,
    )
    .join(" ");
}

function resolveLongestVisibleInterval(
  segments: readonly AuthoringGraphWaypointSegment[],
  bridges: readonly AuthoringGraphBridge[],
): AuthoringGraphInsertionInterval | null {
  const intervals =
    bridges.length === 0
      ? segments.map((_, segmentIndex) => ({
          endT: 1,
          segmentIndex,
          startT: 0,
        }))
      : resolveVisibleIntervals(segments, bridges);

  let bestInterval: AuthoringGraphInsertionInterval | null = null;
  let bestLength = 0;

  intervals.forEach((interval) => {
    const segment = segments[interval.segmentIndex];
    if (!segment) {
      return;
    }

    const length = resolveIntervalArcLength(segment, interval);
    if (length > bestLength) {
      bestInterval = interval;
      bestLength = length;
    }
  });

  return bestLength <= 0.0001 ? null : bestInterval;
}

function resolveVisibleIntervals(
  segments: readonly AuthoringGraphWaypointSegment[],
  bridges: readonly AuthoringGraphBridge[],
): AuthoringGraphInsertionInterval[] {
  const gapIntervals = resolveBridgeGapIntervals(segments, bridges);

  return segments.flatMap((_, segmentIndex) => {
    const hiddenIntervals = gapIntervals
      .filter((interval) => interval.segmentIndex === segmentIndex)
      .sort((left, right) => left.startT - right.startT);
    const visibleIntervals: AuthoringGraphInsertionInterval[] = [];
    let cursor = 0;

    hiddenIntervals.forEach((interval) => {
      if (interval.startT > cursor + 0.0001) {
        visibleIntervals.push({
          endT: interval.startT,
          segmentIndex,
          startT: cursor,
        });
      }

      cursor = Math.max(cursor, interval.endT);
    });

    if (cursor < 1 - 0.0001) {
      visibleIntervals.push({
        endT: 1,
        segmentIndex,
        startT: cursor,
      });
    }

    return visibleIntervals;
  });
}

function resolveIntervalArcLength(
  segment: AuthoringGraphWaypointSegment,
  interval: AuthoringGraphInsertionInterval,
): number {
  const sampleCount = 24;
  let length = 0;
  let previous = evaluateAuthoringGraphCubicBezierPoint(
    segment,
    interval.startT,
  );

  for (let sampleIndex = 1; sampleIndex <= sampleCount; sampleIndex += 1) {
    const t =
      interval.startT +
      (interval.endT - interval.startT) * (sampleIndex / sampleCount);
    const next = evaluateAuthoringGraphCubicBezierPoint(segment, t);
    length += Math.hypot(next.x - previous.x, next.y - previous.y);
    previous = next;
  }

  return length;
}

function resolveBoundaryIntersectionGeometry(
  segment: AuthoringGraphWaypointSegment,
  outsideT: number,
  insideT: number,
  boundary: Required<AuthoringGraphDirectionMarkerBoundary>,
): ResolvedAuthoringGraphDirectionMarkerGeometry | null {
  let outside = outsideT;
  let inside = insideT;

  for (let iteration = 0; iteration < 20; iteration += 1) {
    const mid = (outside + inside) * 0.5;
    const point = evaluateAuthoringGraphCubicBezierPoint(segment, mid);
    if (isPointInsideRoundedBoundary(point, boundary)) {
      inside = mid;
    } else {
      outside = mid;
    }
  }

  const t = (outside + inside) * 0.5;
  const position = evaluateAuthoringGraphCubicBezierPoint(segment, t);
  const tangent = resolveMarkerTangent(segment, t);
  if (!tangent) {
    return null;
  }

  return {
    position: roundPoint(position),
    rotationDegrees: roundNumber(
      (Math.atan2(tangent.y, tangent.x) * 180) / Math.PI,
    ),
    tangent,
  };
}

function isPointInsideRoundedBoundary(
  point: AuthoringGraphPoint,
  boundary: Required<AuthoringGraphDirectionMarkerBoundary>,
): boolean {
  const halfWidth = boundary.width * 0.5;
  const halfHeight = boundary.height * 0.5;
  const radius = boundary.borderRadiusPx;
  const dx = Math.abs(point.x - boundary.center.x) - (halfWidth - radius);
  const dy = Math.abs(point.y - boundary.center.y) - (halfHeight - radius);
  const outsideX = Math.max(dx, 0);
  const outsideY = Math.max(dy, 0);

  return (
    Math.hypot(outsideX, outsideY) <= radius + 0.0001 &&
    Math.max(dx, dy) <= radius + 0.0001
  );
}

function normalizeBoundaryRadius(
  boundary: AuthoringGraphDirectionMarkerBoundary,
): number {
  if (!Number.isFinite(boundary.borderRadiusPx ?? 0)) {
    return 0;
  }

  return Math.max(
    0,
    Math.min(
      boundary.borderRadiusPx ?? 0,
      boundary.width * 0.5,
      boundary.height * 0.5,
    ),
  );
}

function findNearestTAlongSegment(
  segment: AuthoringGraphWaypointSegment,
  point: AuthoringGraphPoint,
  pivot: number,
  direction: -1 | 1,
): number {
  let bestT = pivot;
  let bestDistance = Number.POSITIVE_INFINITY;

  for (
    let sampleIndex = 0;
    sampleIndex <= PATH_GAP_SAMPLE_COUNT;
    sampleIndex += 1
  ) {
    const t = sampleIndex / PATH_GAP_SAMPLE_COUNT;
    if (direction === -1 && t > pivot) {
      break;
    }
    if (direction === 1 && t < pivot) {
      continue;
    }

    const sample = evaluateAuthoringGraphCubicBezierPoint(segment, t);
    const distance = squaredDistance(sample, point);
    if (distance < bestDistance) {
      bestDistance = distance;
      bestT = t;
    }
  }

  return roundUnitInterval(bestT);
}

function extractBezierSubsegment(
  segment: AuthoringGraphWaypointSegment,
  startT: number,
  endT: number,
): AuthoringGraphWaypointSegment {
  const clampedStart = clampUnitInterval(startT);
  const clampedEnd = clampUnitInterval(endT);
  if (clampedEnd <= clampedStart + 0.0001) {
    return {
      controlA: segment.start,
      controlB: segment.start,
      end: segment.start,
      start: segment.start,
    };
  }

  const splitAtEnd = splitBezier(segment, clampedEnd).left;
  const relativeStart = clampedStart / clampedEnd;
  return splitBezier(splitAtEnd, relativeStart).right;
}

function splitBezier(
  segment: AuthoringGraphWaypointSegment,
  t: number,
): {
  left: AuthoringGraphWaypointSegment;
  right: AuthoringGraphWaypointSegment;
} {
  const p0 = segment.start;
  const p1 = segment.controlA;
  const p2 = segment.controlB;
  const p3 = segment.end;

  const p01 = interpolatePoint(p0, p1, t);
  const p12 = interpolatePoint(p1, p2, t);
  const p23 = interpolatePoint(p2, p3, t);
  const p012 = interpolatePoint(p01, p12, t);
  const p123 = interpolatePoint(p12, p23, t);
  const p0123 = interpolatePoint(p012, p123, t);

  return {
    left: {
      controlA: p01,
      controlB: p012,
      end: p0123,
      start: p0,
    },
    right: {
      controlA: p123,
      controlB: p23,
      end: p3,
      start: p0123,
    },
  };
}

function evaluateCubicBezierDerivative(
  segment: AuthoringGraphWaypointSegment,
  t: number,
): AuthoringGraphPoint {
  const mt = 1 - t;

  return {
    x:
      3 * mt * mt * (segment.controlA.x - segment.start.x) +
      6 * mt * t * (segment.controlB.x - segment.controlA.x) +
      3 * t * t * (segment.end.x - segment.controlB.x),
    y:
      3 * mt * mt * (segment.controlA.y - segment.start.y) +
      6 * mt * t * (segment.controlB.y - segment.controlA.y) +
      3 * t * t * (segment.end.y - segment.controlB.y),
  };
}

function resolveMarkerTangent(
  segment: AuthoringGraphWaypointSegment,
  t: number,
): AuthoringGraphPoint | null {
  const tangent = normalisePointOrNull(
    evaluateCubicBezierDerivative(segment, t),
  );
  if (tangent) {
    return tangent;
  }

  return normalisePointOrNull({
    x: segment.end.x - segment.start.x,
    y: segment.end.y - segment.start.y,
  });
}

function interpolatePoint(
  start: AuthoringGraphPoint,
  end: AuthoringGraphPoint,
  t: number,
): AuthoringGraphPoint {
  return {
    x: roundNumber(start.x + (end.x - start.x) * t),
    y: roundNumber(start.y + (end.y - start.y) * t),
  };
}

function roundPoint(point: AuthoringGraphPoint): AuthoringGraphPoint {
  return {
    x: roundNumber(point.x),
    y: roundNumber(point.y),
  };
}

function normalizeBridge(bridge: AuthoringGraphBridge): AuthoringGraphBridge {
  return {
    gap: normalizeBridgeGap(bridge.gap),
    segmentIndex: bridge.segmentIndex,
    t: roundUnitInterval(bridge.t),
  };
}

function normalizeBridgeGap(value: number): number {
  if (!Number.isFinite(value)) {
    return DEFAULT_BRIDGE_GAP_PX;
  }

  return roundNumber(
    Math.max(MIN_BRIDGE_GAP_PX, Math.min(MAX_BRIDGE_GAP_PX, value)),
  );
}

function roundUnitInterval(value: number): number {
  return roundNumber(clampUnitInterval(value));
}

function clampUnitInterval(value: number): number {
  return Math.max(0, Math.min(1, value));
}

function resolveIntervalMidT(
  interval: AuthoringGraphInsertionInterval,
): number {
  return roundUnitInterval((interval.startT + interval.endT) * 0.5);
}

function normalisePoint(point: AuthoringGraphPoint): AuthoringGraphPoint {
  const magnitude = Math.hypot(point.x, point.y);
  if (magnitude <= 0.0001) {
    return { x: 1, y: 0 };
  }

  return {
    x: roundNumber(point.x / magnitude),
    y: roundNumber(point.y / magnitude),
  };
}

function normalisePointOrNull(
  point: AuthoringGraphPoint,
): AuthoringGraphPoint | null {
  const magnitude = Math.hypot(point.x, point.y);
  if (magnitude <= 0.0001) {
    return null;
  }

  return {
    x: roundNumber(point.x / magnitude),
    y: roundNumber(point.y / magnitude),
  };
}

function squaredDistance(
  left: AuthoringGraphPoint,
  right: AuthoringGraphPoint,
): number {
  const deltaX = left.x - right.x;
  const deltaY = left.y - right.y;
  return deltaX * deltaX + deltaY * deltaY;
}

function roundNumber(value: number): number {
  return Number(value.toFixed(3));
}

function formatPoint(point: AuthoringGraphPoint): string {
  return `${point.x} ${point.y}`;
}
