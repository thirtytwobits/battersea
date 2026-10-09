/** Copyright (c) Scott A Dixon */
import type { LayoutGraph, LayoutPositions } from "../graph/layout/types.js";
import type { FlowStudioWorkspaceState } from "./flow-persistence.js";
import { resolveFlowStudioCanvasNodeViewModel } from "./flow-canvas-node.js";
/** Preserves authored edge direction and every node, including disconnected nodes. */
export function buildDataflowLayoutGraph(
  workspace: FlowStudioWorkspaceState,
): LayoutGraph {
  const incoming = new Set(workspace.edges.map((edge) => edge.target));
  return {
    nodes: workspace.nodes.map((node) => {
      const { layout } = resolveFlowStudioCanvasNodeViewModel(node.data);
      return {
        id: node.id,
        width: node.measured?.width ?? layout.nodeWidthPx,
        height: node.measured?.height ?? layout.nodeHeightPx,
      };
    }),
    edges: workspace.edges.map((edge) => ({
      id: edge.id,
      source: edge.source,
      target: edge.target,
      kind: "flow",
    })),
    roots: workspace.nodes
      .filter((node) => !incoming.has(node.id))
      .map((node) => node.id),
  };
}
/** Layout changes only positions; parameter values, port order and authored edge paths survive. */
export function applyDataflowLayout(
  workspace: FlowStudioWorkspaceState,
  positions: LayoutPositions,
): FlowStudioWorkspaceState {
  return {
    ...workspace,
    nodes: workspace.nodes.map((node) =>
      positions[node.id] ? { ...node, position: positions[node.id] } : node,
    ),
  };
}
