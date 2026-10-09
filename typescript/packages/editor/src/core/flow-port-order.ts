/**
 * Copyright (c) Scott A Dixon
 *
 * Implements saved flow node port ordering and live edge-handle remapping.
 */
import type { FlowNode as WireFlowNode } from "@battersea/flow";

import type { FlowStudioEdge } from "./dataflow-editor-state.js";
import {
  createFlowPortHandleId,
  type FlowPortSide,
  type FlowStudioResolvedPort,
} from "./flow-node-ports.js";

export type FlowStudioPortOrder = NonNullable<WireFlowNode["port_order"]>;

const FLOW_PORT_ORDER_SIDES = [
  "action",
  "automation",
  "input",
  "output",
  "signal",
] as const satisfies readonly FlowPortSide[];

export function cloneFlowStudioPortOrder(
  portOrder: FlowStudioPortOrder | null | undefined,
): FlowStudioPortOrder | undefined {
  if (!portOrder) {
    return undefined;
  }

  const clone = {
    action: [...(portOrder.action ?? [])],
    automation: [...(portOrder.automation ?? [])],
    input: [...(portOrder.input ?? [])],
    output: [...(portOrder.output ?? [])],
    signal: [...(portOrder.signal ?? [])],
  } satisfies FlowStudioPortOrder;

  return hasFlowStudioPortOrder(clone) ? clone : undefined;
}

export function hasFlowStudioPortOrder(
  portOrder: FlowStudioPortOrder | null | undefined,
): boolean {
  return Boolean(
    portOrder &&
      FLOW_PORT_ORDER_SIDES.some((side) => (portOrder[side] ?? []).length > 0),
  );
}

export function applyFlowPortOrder(
  ports: readonly FlowStudioResolvedPort[],
  side: FlowPortSide,
  portOrder: FlowStudioPortOrder | null | undefined,
): FlowStudioResolvedPort[] {
  if (ports.length <= 1) {
    return [...ports];
  }

  const preferredIds = dedupePortIds(
    (portOrder?.[side] ?? []).filter((portId) =>
      ports.some((port) => port.id === portId),
    ),
  );
  if (preferredIds.length === 0) {
    return [...ports];
  }

  const preferredIdSet = new Set(preferredIds);
  const portsById = new Map(ports.map((port) => [port.id, port] as const));

  return [
    ...preferredIds.flatMap((portId) => {
      const port = portsById.get(portId);
      return port ? [port] : [];
    }),
    ...ports.filter((port) => !preferredIdSet.has(port.id)),
  ];
}

export function normalizeFlowStudioPortOrder(options: {
  actionPorts: readonly FlowStudioResolvedPort[];
  automationPorts: readonly FlowStudioResolvedPort[];
  inputPorts: readonly FlowStudioResolvedPort[];
  outputPorts: readonly FlowStudioResolvedPort[];
  portOrder: FlowStudioPortOrder | null | undefined;
  signalPorts: readonly FlowStudioResolvedPort[];
}): FlowStudioPortOrder | undefined {
  if (!options.portOrder) {
    return undefined;
  }

  const normalized: FlowStudioPortOrder = {};
  for (const side of FLOW_PORT_ORDER_SIDES) {
    const sideOrder = options.portOrder[side];
    if (!Array.isArray(sideOrder) || sideOrder.length === 0) {
      continue;
    }

    const ports =
      side === "action"
        ? options.actionPorts
        : side === "automation"
          ? options.automationPorts
          : side === "input"
            ? options.inputPorts
            : side === "output"
              ? options.outputPorts
              : options.signalPorts;
    normalized[side] = applyFlowPortOrder(ports, side, options.portOrder).map(
      (port) => port.id,
    );
  }

  return hasFlowStudioPortOrder(normalized) ? normalized : undefined;
}

export function resolveReorderedFlowStudioPortOrder(options: {
  currentPorts: readonly FlowStudioResolvedPort[];
  portOrder: FlowStudioPortOrder | null | undefined;
  side: FlowPortSide;
  activePortId: string;
  overPortId: string;
}): FlowStudioPortOrder | undefined {
  const currentIds = applyFlowPortOrder(
    options.currentPorts,
    options.side,
    options.portOrder,
  ).map((port) => port.id);
  const activeIndex = currentIds.indexOf(options.activePortId);
  const overIndex = currentIds.indexOf(options.overPortId);

  if (activeIndex < 0 || overIndex < 0 || activeIndex === overIndex) {
    return cloneFlowStudioPortOrder(options.portOrder);
  }

  return resolveReorderedFlowStudioPortOrderByIndex({
    activePortId: options.activePortId,
    currentPorts: options.currentPorts,
    portOrder: options.portOrder,
    side: options.side,
    targetIndex: overIndex,
  });
}

export function resolveReorderedFlowStudioPortOrderByIndex(options: {
  activePortId: string;
  currentPorts: readonly FlowStudioResolvedPort[];
  portOrder: FlowStudioPortOrder | null | undefined;
  side: FlowPortSide;
  targetIndex: number;
}): FlowStudioPortOrder | undefined {
  const currentIds = applyFlowPortOrder(
    options.currentPorts,
    options.side,
    options.portOrder,
  ).map((port) => port.id);
  const activeIndex = currentIds.indexOf(options.activePortId);
  if (activeIndex < 0) {
    return cloneFlowStudioPortOrder(options.portOrder);
  }

  const nextIds = [...currentIds];
  const [moved] = nextIds.splice(activeIndex, 1);
  if (!moved) {
    return cloneFlowStudioPortOrder(options.portOrder);
  }
  const boundedTargetIndex = Math.max(
    0,
    Math.min(options.targetIndex, nextIds.length),
  );
  nextIds.splice(boundedTargetIndex, 0, moved);

  const nextOrder: FlowStudioPortOrder =
    cloneFlowStudioPortOrder(options.portOrder) ?? {};
  nextOrder[options.side] = nextIds;
  return hasFlowStudioPortOrder(nextOrder) ? nextOrder : undefined;
}

export function remapNodeSideEdgeHandles(options: {
  edges: readonly FlowStudioEdge[];
  nextPorts: readonly FlowStudioResolvedPort[];
  nodeId: string;
  previousPorts: readonly FlowStudioResolvedPort[];
  side: FlowPortSide;
}): FlowStudioEdge[] {
  if (options.previousPorts.length === 0 || options.nextPorts.length === 0) {
    return [...options.edges];
  }

  const nextIndexByPortId = new Map(
    options.nextPorts.map((port, index) => [port.id, index] as const),
  );

  return options.edges.map((edge) => {
    if (
      (options.side === "input" || options.side === "action") &&
      edge.target === options.nodeId
    ) {
      const nextHandle = remapHandleIdForSide({
        handleId: edge.targetHandle,
        nextIndexByPortId,
        previousPorts: options.previousPorts,
        side: options.side,
      });
      return nextHandle && nextHandle !== edge.targetHandle
        ? {
            ...edge,
            targetHandle: nextHandle,
          }
        : edge;
    }

    if (
      (options.side === "output" || options.side === "signal") &&
      edge.source === options.nodeId
    ) {
      const nextHandle = remapHandleIdForSide({
        handleId: edge.sourceHandle,
        nextIndexByPortId,
        previousPorts: options.previousPorts,
        side: options.side,
      });
      return nextHandle && nextHandle !== edge.sourceHandle
        ? {
            ...edge,
            sourceHandle: nextHandle,
          }
        : edge;
    }

    return edge;
  });
}

function dedupePortIds(portIds: readonly string[]): string[] {
  const seen = new Set<string>();
  return portIds.filter((portId) => {
    if (seen.has(portId)) {
      return false;
    }

    seen.add(portId);
    return true;
  });
}

function remapHandleIdForSide(options: {
  handleId: string | null | undefined;
  nextIndexByPortId: ReadonlyMap<string, number>;
  previousPorts: readonly FlowStudioResolvedPort[];
  side: FlowPortSide;
}): string | null {
  const previousIndex = extractFlowPortHandleIndex(
    options.side,
    options.handleId,
  );
  if (previousIndex === null) {
    return null;
  }

  const previousPortId = options.previousPorts[previousIndex]?.id;
  if (!previousPortId) {
    return null;
  }

  const nextIndex = options.nextIndexByPortId.get(previousPortId);
  return typeof nextIndex === "number"
    ? createFlowPortHandleId(options.side, nextIndex)
    : null;
}

function extractFlowPortHandleIndex(
  side: FlowPortSide,
  handleId: string | null | undefined,
): number | null {
  if (!handleId?.startsWith(`${side}-`)) {
    return null;
  }

  const index = Number.parseInt(handleId.slice(side.length + 1), 10);
  return Number.isFinite(index) ? index : null;
}
