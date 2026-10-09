/**
 * Copyright (c) Scott A Dixon
 *
 * Pure translation between the neutral `LayoutGraph` contract and ELK's graph
 * JSON. Kept free of any `elkjs` import so it can be unit-tested in Node without
 * loading the layout engine.
 */
import type { LayoutGraph, LayoutPositions } from "./types.js";

export type ElkAlgorithm = "layered" | "mrtree";

/** Minimal shape of the ELK graph we build; mirrors elkjs `ElkNode`. */
export interface ElkGraphNode {
  id: string;
  width?: number;
  height?: number;
  x?: number;
  y?: number;
  children?: ElkGraphNode[];
}

export interface ElkGraph {
  id: string;
  layoutOptions: Record<string, string>;
  children: ElkGraphNode[];
  edges: Array<{
    id: string;
    sources: [string];
    targets: [string];
  }>;
}

export interface BuildElkGraphOptions {
  algorithm: ElkAlgorithm;
  direction?: "right" | "down";
}

const ELK_DIRECTION: Record<"right" | "down", string> = {
  down: "DOWN",
  right: "RIGHT",
};

const NODE_SPACING_PX = 64;
const LAYER_SPACING_PX = 96;
const DEFAULT_NODE_WIDTH_PX = 200;
const DEFAULT_NODE_HEIGHT_PX = 72;

/**
 * Builds an ELK graph for the supplied neutral graph. `mrtree` lays the
 * containment hierarchy out as a tidy tree; `layered` produces a directional,
 * crossing-minimised flow. Only `containment` and `flow` edges steer the
 * hierarchy; `link` cross-references are still passed to ELK so layered routing
 * accounts for them, but they never define tree parentage.
 */
export function buildElkGraph(
  graph: LayoutGraph,
  options: BuildElkGraphOptions,
): ElkGraph {
  const direction =
    options.direction ?? (options.algorithm === "layered" ? "right" : "down");
  const layoutOptions: Record<string, string> = {
    "elk.algorithm": options.algorithm,
    "elk.direction": ELK_DIRECTION[direction],
    "elk.spacing.nodeNode": String(NODE_SPACING_PX),
    "elk.layered.spacing.nodeNodeBetweenLayers": String(LAYER_SPACING_PX),
  };
  if (options.algorithm === "mrtree") {
    layoutOptions["elk.spacing.nodeNode"] = String(NODE_SPACING_PX);
  }

  const children = graph.nodes.map((node) => ({
    id: node.id,
    width: Math.max(1, Math.round(node.width) || DEFAULT_NODE_WIDTH_PX),
    height: Math.max(1, Math.round(node.height) || DEFAULT_NODE_HEIGHT_PX),
  }));

  const nodeIds = new Set(graph.nodes.map((node) => node.id));
  const edges = graph.edges
    .filter(
      (edge) =>
        nodeIds.has(edge.source) &&
        nodeIds.has(edge.target) &&
        edge.source !== edge.target,
    )
    .map((edge) => ({
      id: edge.id,
      sources: [edge.source] as [string],
      targets: [edge.target] as [string],
    }));

  return {
    children,
    edges,
    id: "root",
    layoutOptions,
  };
}

/**
 * Reads absolute top-left positions back out of a laid-out ELK graph. ELK
 * returns coordinates relative to the parent; our graph is flat (single level),
 * so child coordinates are already absolute.
 */
export function readElkPositions(result: {
  children?: ElkGraphNode[];
}): LayoutPositions {
  const positions: LayoutPositions = {};
  for (const child of result.children ?? []) {
    if (typeof child.x === "number" && typeof child.y === "number") {
      positions[child.id] = {
        x: Math.round(child.x),
        y: Math.round(child.y),
      };
    }
  }
  return positions;
}
