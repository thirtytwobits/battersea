/**
 * Copyright (c) Scott A Dixon
 *
 * Implements non-drag flow node port move-control semantics.
 */
import type { FlowNodeClass as WireFlowNodeClass } from "@battersea/flow";
import {
  resolveFlowControllerPortVisualSide,
  type FlowControllerPortPlacement,
} from "./flow-controller-port-placement.js";
import type {
  FlowPortSide,
  FlowStudioResolvedPort,
} from "./flow-node-ports.js";

export type FlowPortMoveDirection = "toward-end" | "toward-start";

export interface FlowPortReorderControlState {
  canMoveTowardEnd: boolean;
  canMoveTowardStart: boolean;
  currentIndex: number;
  endIcon: "arrow-down" | "arrow-right";
  endLabel: string;
  kind: "reorder";
  startIcon: "arrow-left" | "arrow-up";
  startLabel: string;
}

export interface FlowPortSwapSideControlState {
  currentIndex: number;
  direction: FlowPortMoveDirection;
  icon: "arrow-down" | "arrow-left" | "arrow-right" | "arrow-up";
  kind: "swap-side";
  label: string;
}

export type FlowPortMoveControlState =
  | FlowPortReorderControlState
  | FlowPortSwapSideControlState;

export function resolveFlowPortMoveControlState(options: {
  controllerPortPlacement?: FlowControllerPortPlacement | null;
  nodeClass?: WireFlowNodeClass | null;
  portId: string;
  ports: readonly FlowStudioResolvedPort[];
  side: FlowPortSide;
}): FlowPortMoveControlState | null {
  const currentIndex = options.ports.findIndex(
    (port) => port.id === options.portId,
  );
  if (currentIndex < 0) {
    return null;
  }

  if (options.side === "action" || options.side === "signal") {
    const visualSide = resolveFlowControllerPortVisualSide({
      nodeClass: options.nodeClass,
      placement: options.controllerPortPlacement,
      side: options.side,
    });
    const direction =
      visualSide === "top" || visualSide === "left"
        ? "toward-start"
        : "toward-end";
    const icon =
      visualSide === "top"
        ? "arrow-up"
        : visualSide === "bottom"
          ? "arrow-down"
          : visualSide === "left"
            ? "arrow-left"
            : "arrow-right";
    const label =
      visualSide === "left" || visualSide === "right"
        ? "Swap left and right controller ports"
        : "Swap top and bottom controller ports";

    return {
      currentIndex,
      direction,
      icon,
      kind: "swap-side",
      label,
    };
  }

  return {
    canMoveTowardEnd: currentIndex < options.ports.length - 1,
    canMoveTowardStart: currentIndex > 0,
    currentIndex,
    endIcon: "arrow-down",
    endLabel: "Move port later",
    kind: "reorder",
    startIcon: "arrow-up",
    startLabel: "Move port earlier",
  };
}

export function resolveFlowPortMoveTargetIndex(options: {
  currentIndex: number;
  direction: FlowPortMoveDirection;
  portCount: number;
}): number | null {
  if (
    options.portCount <= 1 ||
    options.currentIndex < 0 ||
    options.currentIndex >= options.portCount
  ) {
    return null;
  }

  if (options.direction === "toward-start") {
    return options.currentIndex > 0 ? options.currentIndex - 1 : null;
  }

  return options.currentIndex < options.portCount - 1
    ? options.currentIndex + 1
    : null;
}
