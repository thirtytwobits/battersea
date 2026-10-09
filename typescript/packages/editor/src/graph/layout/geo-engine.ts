/**
 * Copyright (c) Scott A Dixon
 *
 * A geographic layout engine. Geolocated nodes (those carrying `geo`) are
 * projected through Web Mercator and held near their projected positions —
 * geography is dominant, not a suggestion. The simulation only de-overlaps
 * locally (a Dorling-cartogram-style relaxation), so real-world arrangement is
 * preserved: north stays up, west stays left, far-apart places stay far apart.
 * Abstract (non-geo) nodes are then settled near their connected geo neighbours.
 * When nothing is geolocated there is no map to draw, so it falls back to gravity.
 *
 * Crucially, charge and links do NOT act on geo nodes — those forces are what
 * scramble a projection (pulling connected places together, shoving everything
 * apart). Only abstract nodes feel them. The map's scale adapts to how tightly
 * the places cluster, so a city-dense graph spreads out enough to read while a
 * globe-spanning one still fits its continents.
 *
 * The simulation runs to convergence synchronously, so output is deterministic
 * and the engine is unit-testable in Node. Loaded lazily by the registry.
 */
import {
  forceCollide,
  forceLink,
  forceManyBody,
  forceSimulation,
  type SimulationLinkDatum,
  type SimulationNodeDatum,
} from "d3-force";

import { runGravityLayout } from "./gravity-engine.js";
import type {
  LayoutEdgeKind,
  LayoutEngine,
  LayoutGraph,
  LayoutPositions,
  LayoutRunOptions,
} from "./types.js";

interface GeoNode extends SimulationNodeDatum {
  id: string;
  width: number;
  height: number;
  /** Projected anchor in canvas pixels; undefined for abstract (non-geo) nodes. */
  anchorX?: number;
  anchorY?: number;
}

interface GeoLink extends SimulationLinkDatum<GeoNode> {
  kind: LayoutEdgeKind;
}

/** Web Mercator is undefined at the poles; clamp to the standard cutoff. */
const WEB_MERCATOR_LAT_LIMIT = 85.05112878;
const SIMULATION_TICKS = 300;
const ABSTRACT_CHARGE_STRENGTH = -500;
const COLLIDE_PADDING_PX = 14;
/** Sweeps of order-preserving overlap removal over the geolocated nodes. */
const OVERLAP_REMOVAL_ITERATIONS = 200;
/** A geo node is spaced from its nearest distinct neighbour by ~this multiple of its size. */
const NEIGHBOUR_GAP_FACTOR = 1.35;
/**
 * Final absolute safety clamp on the laid-out map's radius from centre. The
 * density scale is dominant — it spaces the local cluster (where the authoring
 * happens) to a readable gap, faithfully, even when the coherent local world
 * (e.g. the whole UK at London-readable scale) spans well over a hundred
 * thousand pixels. Far *disconnected* outliers are compressed inward by
 * `compressGeographicOutliers`; this clamp only trips for genuinely pathological
 * geometry, keeping coordinates out of absurd numeric territory. The World
 * canvas frames the dense cluster after applying (not the whole extent), so a
 * large faithful map is fine — you land zoomed into the cluster and pan out.
 */
const MAX_GEO_EXTENT_PX = 400000;
const FALLBACK_GEO_SPAN_PX = 1600;
/**
 * A sorted-radius gap this large a fraction of the map's radius marks a clean
 * split between a coherent local cluster and far-flung outliers (a globe-
 * spanning world). Below it the world is one cluster — left fully faithful.
 */
const OUTLIER_GAP_FRACTION = 0.5;
/**
 * Outliers past the local cluster are compressed to at most this multiple of the
 * cluster's radius — far enough to read as "off in that direction", close enough
 * to remain on a pannable map instead of millions of pixels away.
 */
const OUTLIER_REACH_FACTOR = 1.5;
/**
 * Compact map only: a gap along an axis larger than this many readable
 * neighbour-gaps is a gulf between clusters (not intra-cluster spacing), so it
 * gets collapsed. Comfortably above normal within-cluster spread.
 */
const GULF_THRESHOLD_FACTOR = 4;
/**
 * Compact map only. The `compaction` parameter scales the collapsed-gulf gap
 * GEOMETRICALLY from `COMPACTION_LOOSE_FACTOR` readable neighbour-gaps at 0,
 * through `COMPACTION_TIGHT_FACTOR` at 1 (100%), and on toward nearly touching at
 * `COMPACTION_MAX` (200%). Geometric so it keeps shrinking past 100% instead of
 * inverting into a negative gap the way a linear ramp would. Keep
 * `COMPACTION_DEFAULT`/`COMPACTION_MAX` in sync with the `compaction` parameter
 * declared on the registry descriptor.
 */
const COMPACTION_DEFAULT = 0.5;
const COMPACTION_MAX = 2;
const COMPACTION_LOOSE_FACTOR = 10;
const COMPACTION_TIGHT_FACTOR = 0.75;
const LINK_DISTANCE: Record<LayoutEdgeKind, number> = {
  containment: 150,
  flow: 170,
  link: 210,
};
const ABSTRACT_LINK_STRENGTH = 0.4;

/**
 * Reduces the density-scaled anchor extent in place. This is the one step that
 * differs between the two geographic engines: the faithful map compresses far
 * outliers to the rim; the compact map collapses the gulfs between clusters.
 */
type GeoExtentReducer = (
  points: Array<{ x: number; y: number }>,
  desiredGapPx: number,
) => void;

export function createGeographicEngine(id: string): LayoutEngine {
  return {
    id,
    run(
      graph: LayoutGraph,
      options?: LayoutRunOptions,
    ): Promise<LayoutPositions> {
      return Promise.resolve(runGeographicLayout(graph, options));
    },
  };
}

/**
 * The compact geographic engine. Same projection and the same faithful scale
 * *within* each cluster — local distances still read true — but the vast empty
 * gulfs *between* clusters are collapsed so every distinct cluster fits a single
 * viewport. For a story where the real scale of the places is interesting yet
 * the oceans between regions are just dead space.
 */
export function createCompactGeographicEngine(id: string): LayoutEngine {
  return {
    id,
    run(
      graph: LayoutGraph,
      options?: LayoutRunOptions,
    ): Promise<LayoutPositions> {
      return Promise.resolve(runCompactGeographicLayout(graph, options));
    },
  };
}

/** Faithful map: real distances; far outliers compressed to the rim. */
export function runGeographicLayout(
  graph: LayoutGraph,
  options?: LayoutRunOptions,
): LayoutPositions {
  return runGeoLayout(graph, options, compressGeographicOutliers);
}

/** Compact map: faithful within each cluster; the gulfs between clusters collapsed. */
export function runCompactGeographicLayout(
  graph: LayoutGraph,
  options?: LayoutRunOptions,
): LayoutPositions {
  // `compaction` (0 → clusters far apart, 1 → packed nearly touching) sets how
  // small each collapsed gulf becomes. Mirrors the parameter on the descriptor.
  const compaction = clampCompaction(
    options?.parameters?.compaction ?? COMPACTION_DEFAULT,
  );
  const collapsedFactor =
    COMPACTION_LOOSE_FACTOR *
    Math.pow(COMPACTION_TIGHT_FACTOR / COMPACTION_LOOSE_FACTOR, compaction);
  return runGeoLayout(graph, options, (points, desiredGapPx) =>
    collapseGeographicGulfs(points, desiredGapPx, collapsedFactor),
  );
}

function clampCompaction(value: number): number {
  return Number.isFinite(value)
    ? Math.min(COMPACTION_MAX, Math.max(0, value))
    : COMPACTION_DEFAULT;
}

/** Pure core — no async, no DOM — so it can be unit-tested directly. */
function runGeoLayout(
  graph: LayoutGraph,
  options: LayoutRunOptions | undefined,
  reduceExtent: GeoExtentReducer,
): LayoutPositions {
  if (graph.nodes.length === 0) {
    return {};
  }

  const anchors = projectGeoAnchors(graph, reduceExtent);
  if (anchors.size === 0) {
    // Nothing geolocated — there is no map to draw, so fall back to gravity.
    return runGravityLayout(graph, options);
  }

  const nodes: GeoNode[] = graph.nodes.map((node) => {
    const anchor = anchors.get(node.id);
    return {
      anchorX: anchor?.x,
      anchorY: anchor?.y,
      height: node.height,
      id: node.id,
      width: node.width,
      // Seed geo nodes at their projected position; de-overlap nudges outward
      // from there along anchor order, never reordering them.
      x: anchor?.x,
      y: anchor?.y,
    };
  });

  const isGeo = (node: GeoNode): boolean =>
    node.anchorX !== undefined && node.anchorY !== undefined;
  const geoNodes = nodes.filter(isGeo);
  const abstractNodes = nodes.filter((node) => !isGeo(node));

  // Geo nodes are placed by an order-preserving overlap removal — NOT a force
  // simulation. Force collision pushes nodes apart along their *current*
  // positions, which drift from the projection in a tight cluster and scramble
  // local geography. Separating along the *anchor* order keeps north above
  // south and east right of west even when a city packs many places together.
  removeGeographicOverlaps(geoNodes);

  // Abstract (non-geo) nodes have no projected home, so settle them near their
  // connected geo neighbours with a small force pass, geo nodes pinned in place.
  if (abstractNodes.length > 0) {
    for (const node of geoNodes) {
      node.fx = node.x;
      node.fy = node.y;
    }
    const nodeIds = new Set(nodes.map((node) => node.id));
    const links: GeoLink[] = graph.edges
      .filter(
        (edge) =>
          nodeIds.has(edge.source) &&
          nodeIds.has(edge.target) &&
          edge.source !== edge.target,
      )
      .map((edge) => ({
        kind: edge.kind,
        source: edge.source,
        target: edge.target,
      }));

    forceSimulation<GeoNode>(nodes)
      .force(
        "charge",
        forceManyBody<GeoNode>().strength((node) =>
          isGeo(node) ? 0 : ABSTRACT_CHARGE_STRENGTH,
        ),
      )
      .force(
        "link",
        forceLink<GeoNode, GeoLink>(links)
          .id((node) => node.id)
          .distance((link) => LINK_DISTANCE[link.kind])
          .strength((link) => {
            const source = link.source as GeoNode;
            const target = link.target as GeoNode;
            return isGeo(source) && isGeo(target) ? 0 : ABSTRACT_LINK_STRENGTH;
          }),
      )
      .force(
        "collide",
        forceCollide<GeoNode>()
          .radius(
            (node) =>
              Math.max(node.width, node.height) / 2 + COLLIDE_PADDING_PX,
          )
          .iterations(2),
      )
      .stop()
      .tick(SIMULATION_TICKS);
  }

  const positions: LayoutPositions = {};
  for (const node of nodes) {
    if (options?.signal?.aborted) {
      throw new DOMException("Layout aborted", "AbortError");
    }
    // Positions track node CENTRES; the engine contract is top-left coordinates.
    positions[node.id] = {
      x: Math.round((node.x ?? 0) - node.width / 2),
      y: Math.round((node.y ?? 0) - node.height / 2),
    };
  }
  return positions;
}

/**
 * Removes overlaps between geo nodes while preserving their geographic ordering.
 * Each overlapping pair is separated along whichever axis they overlap *less* on,
 * always in the direction of their anchor order — the node with the smaller
 * projected x stays left, the smaller projected y stays up. Sweeping repeats
 * until overlaps clear (or the budget runs out), so the map stays compact and no
 * two places ever swap places.
 */
function removeGeographicOverlaps(nodes: readonly GeoNode[]): void {
  for (
    let iteration = 0;
    iteration < OVERLAP_REMOVAL_ITERATIONS;
    iteration += 1
  ) {
    let moved = false;
    for (let i = 0; i < nodes.length; i += 1) {
      const a = nodes[i]!;
      for (let j = i + 1; j < nodes.length; j += 1) {
        const b = nodes[j]!;
        const overlapX =
          (a.width + b.width) / 2 +
          COLLIDE_PADDING_PX -
          Math.abs((a.x ?? 0) - (b.x ?? 0));
        const overlapY =
          (a.height + b.height) / 2 +
          COLLIDE_PADDING_PX -
          Math.abs((a.y ?? 0) - (b.y ?? 0));
        if (overlapX <= 0 || overlapY <= 0) {
          continue;
        }

        if (overlapX <= overlapY) {
          const shift = overlapX / 2;
          const aIsWest = (a.anchorX ?? 0) <= (b.anchorX ?? 0);
          a.x = (a.x ?? 0) + (aIsWest ? -shift : shift);
          b.x = (b.x ?? 0) + (aIsWest ? shift : -shift);
        } else {
          const shift = overlapY / 2;
          const aIsNorth = (a.anchorY ?? 0) <= (b.anchorY ?? 0);
          a.y = (a.y ?? 0) + (aIsNorth ? -shift : shift);
          b.y = (b.y ?? 0) + (aIsNorth ? shift : -shift);
        }
        moved = true;
      }
    }
    if (!moved) {
      break;
    }
  }
}

/**
 * Projects every geolocated node through Web Mercator and scales the result so
 * the typical gap between distinct places is a readable multiple of node size.
 * Scaling to the data's density (rather than a fixed extent) is what lets a
 * city-tight cluster spread out enough to read while continents stay far apart.
 */
function projectGeoAnchors(
  graph: LayoutGraph,
  reduceExtent: GeoExtentReducer,
): Map<string, { x: number; y: number }> {
  const projected: Array<{ id: string; x: number; y: number }> = [];
  for (const node of graph.nodes) {
    const geo = node.geo;
    if (
      geo &&
      Number.isFinite(geo.latitude) &&
      Number.isFinite(geo.longitude)
    ) {
      projected.push({
        id: node.id,
        ...projectMercator(geo.latitude, geo.longitude),
      });
    }
  }
  if (projected.length === 0) {
    return new Map();
  }

  const desiredGapPx = medianNodeFootprint(graph) * NEIGHBOUR_GAP_FACTOR;
  const scale = resolveGeoDensityScale(projected, desiredGapPx);

  // Density scale spaces every cluster to a readable gap — faithfully, with no
  // global cap that would crush it. The caller's `reduceExtent` then bounds the
  // overall extent: the faithful map compresses far outliers to the rim; the
  // compact map collapses the gulfs between clusters. Either way the dense local
  // geography is left exact.
  const placed = projected.map((point) => ({
    id: point.id,
    x: point.x * scale,
    y: point.y * scale,
  }));
  reduceExtent(placed, desiredGapPx);
  clampToMaxExtent(placed);
  centreGeographicPointsAtOrigin(placed);

  return new Map(placed.map((point) => [point.id, { x: point.x, y: point.y }]));
}

/**
 * D3 seeds unpositioned nodes around the origin. Keep the projected anchors
 * around that same origin before the abstract-node force pass; otherwise a
 * dense real-world cluster can put its Mercator anchors far from the origin,
 * and the linked abstract hierarchy spends the fixed simulation budget
 * flying across that artificial offset instead of settling near its parents.
 * Translation leaves every geographic distance and ordering unchanged.
 */
function centreGeographicPointsAtOrigin(
  points: Array<{ x: number; y: number }>,
): void {
  const centre = centroidOf(points);
  for (const point of points) {
    point.x -= centre.x;
    point.y -= centre.y;
  }
}

/**
 * Picks pixels-per-Mercator-unit so the median nearest-distinct-neighbour gap
 * maps to `desiredGapPx`. Scaling to density (rather than a fixed extent) is what
 * lets a city-tight cluster spread out enough to read; the extent is then bounded
 * by compressing far outliers, not by shrinking this scale (which would crush the
 * cluster). Falls back to a fixed scale when every place shares a coordinate.
 */
function resolveGeoDensityScale(
  projected: ReadonlyArray<{ x: number; y: number }>,
  desiredGapPx: number,
): number {
  const nearestGaps: number[] = [];
  for (let i = 0; i < projected.length; i += 1) {
    let nearest = Number.POSITIVE_INFINITY;
    for (let j = 0; j < projected.length; j += 1) {
      if (i === j) {
        continue;
      }
      const distance = Math.hypot(
        projected[i]!.x - projected[j]!.x,
        projected[i]!.y - projected[j]!.y,
      );
      if (distance > 0) {
        nearest = Math.min(nearest, distance);
      }
    }
    if (Number.isFinite(nearest)) {
      nearestGaps.push(nearest);
    }
  }

  const medianGap = medianOf(nearestGaps);
  if (medianGap && medianGap > 0) {
    return desiredGapPx / medianGap;
  }

  // Every place coincides — no distinct neighbours to scale against.
  return FALLBACK_GEO_SPAN_PX;
}

/**
 * Pulls far *disconnected* outliers (a place on a different continent) inward so
 * a globe-spanning world stays on a pannable canvas, while leaving the coherent
 * local cluster exact. Detects the split as the dominant gap in the sorted radii
 * from the centroid: everything inside it (the local world) keeps its true,
 * density-scaled position; everything past it is soft-compressed into a band just
 * beyond the cluster, in its true compass direction. A single-cluster world has
 * no dominant gap and is left entirely untouched.
 */
function compressGeographicOutliers(
  points: Array<{ x: number; y: number }>,
  _desiredGapPx: number,
): void {
  if (points.length < 3) {
    return;
  }
  const centre = centroidOf(points);
  const radii = points
    .map((point) => Math.hypot(point.x - centre.x, point.y - centre.y))
    .filter((radius) => radius > 0)
    .sort((left, right) => left - right);
  if (radii.length < 2) {
    return;
  }

  const maxRadius = radii[radii.length - 1]!;
  let coreRadius = -1;
  let widestGap = 0;
  for (let i = 0; i < radii.length - 1; i += 1) {
    const gap = radii[i + 1]! - radii[i]!;
    if (gap > widestGap) {
      widestGap = gap;
      coreRadius = radii[i]!;
    }
  }

  // Only compress when the world is clearly bimodal — a local cluster and a
  // far-flung remainder. Otherwise it is one cluster: leave it fully faithful.
  if (coreRadius <= 0 || widestGap < maxRadius * OUTLIER_GAP_FRACTION) {
    return;
  }

  const reach = coreRadius * OUTLIER_REACH_FACTOR;
  for (const point of points) {
    const dx = point.x - centre.x;
    const dy = point.y - centre.y;
    const distance = Math.hypot(dx, dy);
    if (distance <= coreRadius) {
      continue;
    }
    const compressed =
      coreRadius +
      (reach - coreRadius) *
        (1 - Math.exp(-(distance - coreRadius) / coreRadius));
    const k = compressed / distance;
    point.x = centre.x + dx * k;
    point.y = centre.y + dy * k;
  }
}

/**
 * Collapses the empty gulfs between clusters, leaving each cluster's internal
 * geography untouched. Works one axis at a time: walk the nodes in sorted order
 * and, wherever the jump to the next node is far larger than normal within-
 * cluster spacing (a gulf — an ocean, a continent), shrink just that jump to a
 * fixed readable gap, sliding everything past it inward. Intra-cluster gaps are
 * below the threshold and pass through unchanged, so real local scale survives
 * while the dead space between regions disappears. Order is preserved on both
 * axes, so a cluster to the west/north stays west/north — just nearer.
 */
function collapseGeographicGulfs(
  points: Array<{ x: number; y: number }>,
  desiredGapPx: number,
  collapsedFactor: number,
): void {
  collapseAxisGulfs(points, "x", desiredGapPx, collapsedFactor);
  collapseAxisGulfs(points, "y", desiredGapPx, collapsedFactor);
}

function collapseAxisGulfs(
  points: Array<{ x: number; y: number }>,
  axis: "x" | "y",
  desiredGapPx: number,
  collapsedFactor: number,
): void {
  if (points.length < 2) {
    return;
  }
  const gulfThreshold = desiredGapPx * GULF_THRESHOLD_FACTOR;
  const collapsedGulf = desiredGapPx * collapsedFactor;
  const sorted = [...points].sort((left, right) => left[axis] - right[axis]);
  // Snapshot the original coordinates so each gap is read before any shifting.
  const original = sorted.map((point) => point[axis]);

  let shift = 0;
  for (let i = 1; i < sorted.length; i += 1) {
    const gap = original[i]! - original[i - 1]!;
    if (gap > gulfThreshold) {
      shift += gap - collapsedGulf;
    }
    sorted[i]![axis] = original[i]! - shift;
  }
}

/** Final radial safety clamp so pathological geometry can't blow the extent. */
function clampToMaxExtent(points: Array<{ x: number; y: number }>): void {
  if (points.length === 0) {
    return;
  }
  const centre = centroidOf(points);
  const maxRadius = MAX_GEO_EXTENT_PX / 2;
  for (const point of points) {
    const dx = point.x - centre.x;
    const dy = point.y - centre.y;
    const distance = Math.hypot(dx, dy);
    if (distance <= maxRadius || distance === 0) {
      continue;
    }
    const k = maxRadius / distance;
    point.x = centre.x + dx * k;
    point.y = centre.y + dy * k;
  }
}

/**
 * Component-wise MEDIAN centre — robust to far outliers, unlike the mean, which a
 * single different-continent place drags right out of the dense cluster. The
 * compression/clamp measure radius from here, so the centre must sit *in* the
 * cluster or the cluster itself gets pulled around.
 */
function centroidOf(points: ReadonlyArray<{ x: number; y: number }>): {
  x: number;
  y: number;
} {
  return {
    x: medianOf(points.map((point) => point.x)) ?? 0,
    y: medianOf(points.map((point) => point.y)) ?? 0,
  };
}

function medianNodeFootprint(graph: LayoutGraph): number {
  const footprints = graph.nodes.map((node) =>
    Math.max(node.width, node.height),
  );
  return medianOf(footprints) ?? 200;
}

function medianOf(values: readonly number[]): number | undefined {
  if (values.length === 0) {
    return undefined;
  }

  const sorted = [...values].sort((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0
    ? (sorted[middle - 1]! + sorted[middle]!) / 2
    : sorted[middle]!;
}

/** Normalised Web Mercator projection ([0,1], y increasing southward). */
function projectMercator(
  latitude: number,
  longitude: number,
): { x: number; y: number } {
  const clampedLatitude = Math.min(
    WEB_MERCATOR_LAT_LIMIT,
    Math.max(-WEB_MERCATOR_LAT_LIMIT, latitude),
  );
  const latitudeRadians = (clampedLatitude * Math.PI) / 180;
  return {
    x: (longitude + 180) / 360,
    y:
      (1 -
        Math.log(Math.tan(latitudeRadians) + 1 / Math.cos(latitudeRadians)) /
          Math.PI) /
      2,
  };
}
