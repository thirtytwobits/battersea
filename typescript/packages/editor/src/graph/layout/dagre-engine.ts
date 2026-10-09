/** Copyright (c) Scott A Dixon */
import dagre from "@dagrejs/dagre";
import type { LayoutEngine, LayoutGraph, LayoutRunOptions } from "./types.js";
export function createDagreEngine(): LayoutEngine {
  return {
    id: "dagre",
    async run(graph: LayoutGraph, options?: LayoutRunOptions) {
      options?.signal?.throwIfAborted();
      const layout = new dagre.graphlib.Graph({ multigraph: true });
      layout.setGraph({ rankdir: options?.direction === "down" ? "TB" : "LR" });
      layout.setDefaultEdgeLabel(() => ({}));
      for (const node of graph.nodes)
        layout.setNode(node.id, { width: node.width, height: node.height });
      for (const edge of graph.edges) {
        if (!layout.hasNode(edge.source) || !layout.hasNode(edge.target))
          throw new Error(`Unknown endpoint for edge ${edge.id}`);
        layout.setEdge(edge.source, edge.target, {}, edge.id);
      }
      dagre.layout(layout);
      options?.signal?.throwIfAborted();
      return Object.fromEntries(
        graph.nodes.map(({ id, width, height }) => {
          const node = layout.node(id);
          return [id, { x: node.x - width / 2, y: node.y - height / 2 }];
        }),
      );
    },
  };
}
