/**
 * Copyright (c) Scott A Dixon
 *
 * Resolves edge presentation for the editor's dataflow canvas.
 */
import type { FlowStudioEdge } from "./dataflow-editor-state.js";
import type { FlowStudioEdgeActivationPhase } from "./flow-edge-activation.js";
import { resolveConnectionSourceTokenType } from "./flow-node-ports.js";
import type { FlowStudioNode } from "./dataflow-editor-state.js";

function joinEdgeClassNames(
  ...classNames: Array<string | null | undefined>
): string | undefined {
  const joined = classNames
    .flatMap((value) => value?.split(/\s+/u) ?? [])
    .filter((value) => value.length > 0)
    .join(" ")
    .trim();

  return joined.length > 0 ? joined : undefined;
}

/**
 * Returns true when a token type carries an array/list of values rather than
 * a single value. Array-typed edges are rendered with a doubled stroke so
 * authors can tell at a glance that the wire moves a sequence.
 *
 * Token-type names follow two naming conventions in the manifest: a
 * `camelCaseArray` suffix (e.g. `prompt.fragmentArray`) and a
 * snake_case `_list` / `_ids` suffix (e.g. `media.asset_list`,
 * `story.character_ids`). Both indicate cardinality > 1.
 */
export function isArrayTokenType(
  tokenType: string | null | undefined,
): boolean {
  if (!tokenType) {
    return false;
  }
  return (
    /Array$/.test(tokenType) ||
    /_list$/.test(tokenType) ||
    /_ids$/.test(tokenType) ||
    /_array$/.test(tokenType)
  );
}

export function resolveFlowCanvasEdge(
  edge: FlowStudioEdge,
  nodes: readonly FlowStudioNode[],
  activationPhase: FlowStudioEdgeActivationPhase | null = null,
): FlowStudioEdge {
  const tokenType = resolveConnectionSourceTokenType({
    connection: edge,
    nodes,
  });

  return {
    ...edge,
    animated: activationPhase === "active",
    data: {
      bridges: Array.isArray(edge.data?.bridges)
        ? edge.data.bridges
        : undefined,
      kind: edge.data?.kind === "signal" ? "signal" : "token",
      order: typeof edge.data?.order === "number" ? edge.data.order : 0,
      tokenType: tokenType ?? undefined,
      waypoints: Array.isArray(edge.data?.waypoints)
        ? edge.data.waypoints
        : undefined,
    },
    className: joinEdgeClassNames(
      "flow-studio-edge",
      edge.className,
      edge.data?.kind === "signal" ? "flow-studio-edge--signal" : null,
      isArrayTokenType(tokenType) ? "flow-studio-edge--fragment-array" : null,
      activationPhase === "active" ? "flow-studio-edge--active" : null,
      activationPhase === "fading" ? "flow-studio-edge--fading" : null,
    ),
    type: "flowStudio",
  };
}
