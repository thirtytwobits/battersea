/**
 * Copyright (c) Scott A Dixon
 *
 * Tracks editor-local placement of controller ports on a node's controller axis.
 */
import type { FlowNodeClass as WireFlowNodeClass } from "@battersea/flow";

export type FlowControllerPortPlacement = "default" | "swapped";
export type FlowControllerPortVisualSide = "bottom" | "left" | "right" | "top";

export function normalizeFlowControllerPortPlacement(
  value: unknown,
): FlowControllerPortPlacement {
  return value === "swapped" ? "swapped" : "default";
}

export function toggleFlowControllerPortPlacement(
  placement: FlowControllerPortPlacement | null | undefined,
): FlowControllerPortPlacement {
  return normalizeFlowControllerPortPlacement(placement) === "swapped"
    ? "default"
    : "swapped";
}

export function hasRotatedControllerPortAxis(
  nodeClass: WireFlowNodeClass | null | undefined,
): boolean {
  return (
    nodeClass === "instrument" ||
    nodeClass === "control" ||
    nodeClass === "logic"
  );
}

export function resolveFlowControllerPortVisualSide(options: {
  nodeClass?: WireFlowNodeClass | null;
  placement: FlowControllerPortPlacement | null | undefined;
  side: "action" | "automation" | "signal";
}): FlowControllerPortVisualSide {
  const placement = normalizeFlowControllerPortPlacement(options.placement);
  const rotatedAxis = hasRotatedControllerPortAxis(options.nodeClass);

  // Automation ports share the action side: both are sinks fed by upstream
  // emissions (token edges), but their delivery is post-activation.
  const isInputControllerSide =
    options.side === "action" || options.side === "automation";

  if (rotatedAxis) {
    if (isInputControllerSide) {
      return placement === "swapped" ? "right" : "left";
    }

    return placement === "swapped" ? "left" : "right";
  }

  if (isInputControllerSide) {
    return placement === "swapped" ? "bottom" : "top";
  }

  return placement === "swapped" ? "top" : "bottom";
}
