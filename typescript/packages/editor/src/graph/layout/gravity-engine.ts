/**
 * Copyright (c) Scott A Dixon
 *
 * A force-directed (`gravity`) layout engine built on d3-force. Unlike the ELK
 * hierarchical engines, this treats the graph as a physical system: nodes repel,
 * edges act as springs, and collisions are resolved from the real node sizes.
 * Crucially it springs on BOTH containment and link edges, so cross-referenced
 * places cluster — the thing the old radial layout never did.
 *
 * The simulation is run to convergence synchronously (no animation), so output
 * is deterministic for a given input and the engine is unit-testable in Node.
 * It is loaded lazily by the registry like the ELK engines.
 */
import {
  forceCollide,
  forceLink,
  forceManyBody,
  forceSimulation,
  forceX,
  forceY,
  type SimulationLinkDatum,
  type SimulationNodeDatum,
} from "d3-force";

import type {
  LayoutEdgeKind,
  LayoutEngine,
  LayoutGraph,
  LayoutPositions,
  LayoutRunOptions,
} from "./types.js";

interface GravityNode extends SimulationNodeDatum {
  id: string;
  width: number;
  height: number;
}

interface GravityLink extends SimulationLinkDatum<GravityNode> {
  kind: LayoutEdgeKind;
}

const SIMULATION_TICKS = 320;
const CHARGE_STRENGTH = -1400;
const GRAVITY_STRENGTH = 0.045;
const COLLIDE_PADDING_PX = 16;

/** Edges spring differently by kind: containment forms the tight skeleton, links pull gently. */
const LINK_DISTANCE: Record<LayoutEdgeKind, number> = {
  containment: 190,
  flow: 210,
  link: 260,
};
const LINK_STRENGTH: Record<LayoutEdgeKind, number> = {
  containment: 0.85,
  flow: 0.7,
  link: 0.3,
};

export function createGravityEngine(id: string): LayoutEngine {
  return {
    id,
    run(
      graph: LayoutGraph,
      options?: LayoutRunOptions,
    ): Promise<LayoutPositions> {
      return Promise.resolve(runGravityLayout(graph, options));
    },
  };
}

/** Pure simulation core — no async, no DOM — kept separate so it can be unit-tested directly. */
export function runGravityLayout(
  graph: LayoutGraph,
  options?: LayoutRunOptions,
): LayoutPositions {
  if (graph.nodes.length === 0) {
    return {};
  }

  const nodes: GravityNode[] = graph.nodes.map((node) => ({
    height: node.height,
    id: node.id,
    width: node.width,
  }));
  const nodeIds = new Set(nodes.map((node) => node.id));
  const links: GravityLink[] = graph.edges
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

  const simulation = forceSimulation<GravityNode>(nodes)
    .force("charge", forceManyBody<GravityNode>().strength(CHARGE_STRENGTH))
    .force(
      "link",
      forceLink<GravityNode, GravityLink>(links)
        .id((node) => node.id)
        .distance((link) => LINK_DISTANCE[link.kind])
        .strength((link) => LINK_STRENGTH[link.kind]),
    )
    .force(
      "collide",
      forceCollide<GravityNode>()
        .radius((node) => collisionRadius(node))
        .iterations(2),
    )
    .force("x", forceX<GravityNode>(0).strength(GRAVITY_STRENGTH))
    .force("y", forceY<GravityNode>(0).strength(GRAVITY_STRENGTH))
    .stop();

  simulation.tick(SIMULATION_TICKS);

  const positions: LayoutPositions = {};
  for (const node of nodes) {
    if (options?.signal?.aborted) {
      throw new DOMException("Layout aborted", "AbortError");
    }
    // d3-force tracks node CENTRES; the engine contract is top-left coordinates.
    positions[node.id] = {
      x: Math.round((node.x ?? 0) - node.width / 2),
      y: Math.round((node.y ?? 0) - node.height / 2),
    };
  }
  return positions;
}

function collisionRadius(node: GravityNode): number {
  return Math.max(node.width, node.height) / 2 + COLLIDE_PADDING_PX;
}
