/**
 * Copyright (c) Scott A Dixon
 *
 * Implements flow node port reorder drag payloads and workspace mutations.
 *
 * For sides that carry one or more dynamic-port groups (e.g. Concatenate's
 * `input-{index}`), reordering renames the underlying port ids so the visual
 * order equals the saved logical order. The Concatenate runtime — and any
 * other handler that sorts dynamic ports by numeric suffix — then honours
 * the drag without us needing a parallel `port_order` field. Fixed-name
 * sides (or sides with no dynamic groups) fall back to the visual-only
 * `port_order` mechanism since their ids are owned by the definition.
 */
import type { DragEndEvent, DragStartEvent } from "@dnd-kit/core";
import type { FlowNodeDefinition as WireFlowNodeDefinition } from "@battersea/flow";

import type {
  FlowStudioEdge,
  FlowStudioNode,
} from "./dataflow-editor-state.js";
import {
  buildDynamicPortRenameMap,
  dynamicPortGroupsForSide,
} from "./flow-dynamic-port-template.js";
import {
  normalizeFlowControllerPortPlacement,
  toggleFlowControllerPortPlacement,
  type FlowControllerPortPlacement,
} from "./flow-controller-port-placement.js";
import {
  buildResolvedNodeData,
  cloneFlowPortParameterValues,
  type FlowPortParameterValues,
} from "./flow-node-definitions.js";
import {
  applyFlowPortOrder,
  cloneFlowStudioPortOrder,
  hasFlowStudioPortOrder,
  remapNodeSideEdgeHandles,
  resolveReorderedFlowStudioPortOrderByIndex,
  type FlowStudioPortOrder,
} from "./flow-port-order.js";
import {
  cloneFlowStudioPortNames,
  createFlowPortHandleId,
  resolveFlowStudioResolvedPorts,
  type FlowPortSide,
  type FlowStudioResolvedPort,
} from "./flow-node-ports.js";
import type { FlowStudioWorkspaceState } from "./flow-persistence.js";

export interface FlowPortReorderDragData {
  kind: "flow-port-reorder";
  nodeId: string;
  portId: string;
  side: FlowPortSide;
}

export interface FlowPortReorderDropData {
  kind: "flow-port-reorder-target";
  nodeId: string;
  portId: string;
  side: FlowPortSide;
}

export function buildFlowPortReorderDragId(
  nodeId: string,
  side: FlowPortSide,
  portId: string,
): string {
  return `flow-port:${nodeId}:${side}:${portId}`;
}

export function buildFlowPortReorderDropId(
  nodeId: string,
  side: FlowPortSide,
  portId: string,
): string {
  return `flow-port-drop:${nodeId}:${side}:${portId}`;
}

export function readFlowPortReorderDragData(
  value: unknown,
): FlowPortReorderDragData | null {
  if (!value || typeof value !== "object") {
    return null;
  }

  const candidate = value as Partial<FlowPortReorderDragData>;
  return candidate.kind === "flow-port-reorder" &&
    typeof candidate.nodeId === "string" &&
    typeof candidate.portId === "string" &&
    isFlowPortSide(candidate.side)
    ? {
        kind: "flow-port-reorder",
        nodeId: candidate.nodeId,
        portId: candidate.portId,
        side: candidate.side,
      }
    : null;
}

export function readFlowPortReorderDropData(
  value: unknown,
): FlowPortReorderDropData | null {
  if (!value || typeof value !== "object") {
    return null;
  }

  const candidate = value as Partial<FlowPortReorderDropData>;
  return candidate.kind === "flow-port-reorder-target" &&
    typeof candidate.nodeId === "string" &&
    typeof candidate.portId === "string" &&
    isFlowPortSide(candidate.side)
    ? {
        kind: "flow-port-reorder-target",
        nodeId: candidate.nodeId,
        portId: candidate.portId,
        side: candidate.side,
      }
    : null;
}

export function isFlowPortReorderDragStartEvent(
  event: DragStartEvent,
): boolean {
  return readFlowPortReorderDragData(event.active.data.current) !== null;
}

export function isFlowPortReorderDragEndEvent(event: DragEndEvent): boolean {
  return readFlowPortReorderDragData(event.active.data.current) !== null;
}

export function reorderNodePortsInWorkspace(options: {
  activePortId: string;
  definition: WireFlowNodeDefinition;
  nodeId: string;
  previewIndex: number;
  side: FlowPortSide;
  workspace: FlowStudioWorkspaceState;
}): FlowStudioWorkspaceState {
  const targetNode = options.workspace.nodes.find(
    (node) => node.id === options.nodeId,
  );
  if (!targetNode) {
    return options.workspace;
  }

  const currentPorts = resolveNodePortsForSide(targetNode, options.side);
  const nextPortOrder = resolveReorderedFlowStudioPortOrderByIndex({
    activePortId: options.activePortId,
    currentPorts,
    portOrder: targetNode.data.portOrder,
    side: options.side,
    targetIndex: options.previewIndex,
  });

  const visualOrderOldIds = applyFlowPortOrder(
    currentPorts,
    options.side,
    nextPortOrder,
  ).map((port) => port.id);

  // Renumber the dynamic-port groups on this side so each surviving port's
  // id reflects its new visual position (Concatenate's runtime sorts by
  // numeric suffix, so visual order has to live in the id itself). Fixed
  // ports keep their definition-owned names via identity entries.
  const hasDynamicGroupOnSide =
    dynamicPortGroupsForSide(options.definition, options.side).length > 0;
  const oldToNewId = buildDynamicPortRenameMap({
    definition: options.definition,
    side: options.side,
    visualOrderOldIds,
  });
  const renameChangedAnyId = visualOrderOldIds.some(
    (id) => oldToNewId.get(id) !== id,
  );

  // If nothing changed (drag landed in the same slot) and we couldn't rename
  // anything, leave the workspace untouched.
  const visualOrderUnchanged =
    visualOrderOldIds.length === currentPorts.length &&
    visualOrderOldIds.every((id, index) => id === currentPorts[index].id);
  if (visualOrderUnchanged && !renameChangedAnyId) {
    return options.workspace;
  }

  const nextPortNames = rekeySidePortNames(
    targetNode,
    options.side,
    oldToNewId,
  );
  const nextPortParameterValues = rekeySidePortParameters(
    targetNode,
    options.side,
    oldToNewId,
  );

  // Compute the final port_order for this side. For pure-dynamic sides the
  // renumber makes natural order match visual order, so we can drop the
  // side from port_order (the saved flow is leaner and the runtime needs no
  // help). For mixed-port sides we still need port_order to record the
  // interleaving of fixed and dynamic ports the user chose.
  const visualOrderNewIds = visualOrderOldIds.map(
    (id) => oldToNewId.get(id) ?? id,
  );
  const sideOrderToPersist = computeSideOrderToPersist({
    definition: options.definition,
    hasDynamicGroupOnSide,
    side: options.side,
    visualOrderNewIds,
  });

  const previousOverall =
    cloneFlowStudioPortOrder(targetNode.data.portOrder) ?? {};
  const nextOverall: FlowStudioPortOrder = { ...previousOverall };
  if (sideOrderToPersist === undefined) {
    delete nextOverall[options.side];
  } else {
    nextOverall[options.side] = sideOrderToPersist;
  }
  const nextOverallOrder = hasFlowStudioPortOrder(nextOverall)
    ? nextOverall
    : undefined;

  const nextData = buildResolvedNodeData({
    controllerPortPlacement: targetNode.data.controllerPortPlacement,
    definition: options.definition,
    instanceName: targetNode.data.instanceName,
    parameterValues: targetNode.data.parameterValues,
    portOrder: nextOverallOrder,
    portNames: nextPortNames,
    portParameterValues: nextPortParameterValues,
  });
  const nextPorts = resolveNodePortsForSide(
    { ...targetNode, data: nextData },
    options.side,
  );

  const nextIndexByPortId = new Map(
    nextPorts.map((port, index) => [port.id, index] as const),
  );

  const nextEdges = renameChangedAnyId
    ? remapEdgesAfterPortRename({
        edges: options.workspace.edges,
        nextIndexByPortId,
        nodeId: options.nodeId,
        oldToNewId,
        previousPorts: currentPorts,
        side: options.side,
      })
    : remapNodeSideEdgeHandles({
        edges: options.workspace.edges,
        nextPorts,
        nodeId: options.nodeId,
        previousPorts: currentPorts,
        side: options.side,
      });

  return {
    ...options.workspace,
    edges: nextEdges,
    nodes: options.workspace.nodes.map((node) =>
      node.id === options.nodeId
        ? {
            ...node,
            className: `flow-studio-node flow-studio-node--${nextData.nodeClass}`,
            data: nextData,
          }
        : node,
    ),
  };
}

export function swapNodeControllerPortPlacementInWorkspace(options: {
  definition: WireFlowNodeDefinition;
  nodeId: string;
  workspace: FlowStudioWorkspaceState;
}): FlowStudioWorkspaceState {
  const targetNode = options.workspace.nodes.find(
    (node) => node.id === options.nodeId,
  );
  return setNodeControllerPortPlacementInWorkspace({
    definition: options.definition,
    nodeId: options.nodeId,
    placement: toggleFlowControllerPortPlacement(
      targetNode?.data.controllerPortPlacement,
    ),
    workspace: options.workspace,
  });
}

export function setNodeControllerPortPlacementInWorkspace(options: {
  definition: WireFlowNodeDefinition;
  nodeId: string;
  placement: FlowControllerPortPlacement;
  workspace: FlowStudioWorkspaceState;
}): FlowStudioWorkspaceState {
  const targetNode = options.workspace.nodes.find(
    (node) => node.id === options.nodeId,
  );
  if (!targetNode) {
    return options.workspace;
  }

  const placement = normalizeFlowControllerPortPlacement(options.placement);
  const nextData = buildResolvedNodeData({
    controllerPortPlacement: placement,
    definition: options.definition,
    instanceName: targetNode.data.instanceName,
    parameterValues: targetNode.data.parameterValues,
    portParameterValues: targetNode.data.portParameterValues,
    portOrder: targetNode.data.portOrder,
    portNames: targetNode.data.portNames,
  });
  if (
    nextData.controllerPortPlacement === targetNode.data.controllerPortPlacement
  ) {
    return options.workspace;
  }

  return {
    ...options.workspace,
    nodes: options.workspace.nodes.map((node) =>
      node.id === options.nodeId
        ? {
            ...node,
            className: `flow-studio-node flow-studio-node--${nextData.nodeClass}`,
            data: nextData,
          }
        : node,
    ),
  };
}

function isFlowPortSide(value: unknown): value is FlowPortSide {
  return (
    value === "action" ||
    value === "input" ||
    value === "output" ||
    value === "signal"
  );
}

function resolveNodePortsForSide(node: FlowStudioNode, side: FlowPortSide) {
  return side === "action"
    ? resolveFlowStudioResolvedPorts(
        node.data.actionPorts,
        "action",
        node.data.nodeClass,
      )
    : side === "input"
      ? resolveFlowStudioResolvedPorts(
          node.data.inputPorts,
          "input",
          node.data.nodeClass,
        )
      : side === "output"
        ? resolveFlowStudioResolvedPorts(
            node.data.outputPorts,
            "output",
            node.data.nodeClass,
          )
        : resolveFlowStudioResolvedPorts(
            node.data.signalPorts,
            "signal",
            node.data.nodeClass,
          );
}

function rekeySidePortNames(
  node: FlowStudioNode,
  side: FlowPortSide,
  oldToNewId: ReadonlyMap<string, string>,
): ReturnType<typeof cloneFlowStudioPortNames> {
  const previous = cloneFlowStudioPortNames(node.data.portNames) ?? {};
  const previousSide = previous[side] ?? {};
  const nextSide: Record<string, string> = {};
  for (const [oldId, alias] of Object.entries(previousSide)) {
    const newId = oldToNewId.get(oldId) ?? oldId;
    nextSide[newId] = alias;
  }

  return {
    ...previous,
    [side]: Object.keys(nextSide).length > 0 ? nextSide : undefined,
  };
}

function rekeySidePortParameters(
  node: FlowStudioNode,
  side: FlowPortSide,
  oldToNewId: ReadonlyMap<string, string>,
): FlowPortParameterValues | undefined {
  if (side !== "input" && side !== "output") {
    return cloneFlowPortParameterValues(node.data.portParameterValues);
  }

  const previous =
    cloneFlowPortParameterValues(node.data.portParameterValues) ?? {};
  const previousSide = previous[side] ?? {};
  const nextSide: Record<string, Record<string, unknown>> = {};
  for (const [oldId, values] of Object.entries(previousSide)) {
    const newId = oldToNewId.get(oldId) ?? oldId;
    nextSide[newId] = values;
  }

  return {
    ...previous,
    [side]: Object.keys(nextSide).length > 0 ? nextSide : undefined,
  };
}

function computeSideOrderToPersist(options: {
  definition: WireFlowNodeDefinition;
  hasDynamicGroupOnSide: boolean;
  side: FlowPortSide;
  visualOrderNewIds: readonly string[];
}): string[] | undefined {
  // If the side has no dynamic groups, fall back to the visual-only port_order
  // mechanism: ids are owned by the definition, so the only way to express
  // user-chosen order is the parallel array.
  if (!options.hasDynamicGroupOnSide) {
    return [...options.visualOrderNewIds];
  }

  // After dynamic-port renumber, the natural definition order interleaves
  // fixed ports (in definition order) with each dynamic group's renumbered
  // members. If the visual order matches that natural order, the saved
  // port_order entry is redundant and we drop it.
  const naturalOrder = naturalSideOrderAfterRename({
    definition: options.definition,
    side: options.side,
    visualOrderNewIds: options.visualOrderNewIds,
  });

  if (
    naturalOrder.length === options.visualOrderNewIds.length &&
    naturalOrder.every((id, index) => id === options.visualOrderNewIds[index])
  ) {
    return undefined;
  }

  return [...options.visualOrderNewIds];
}

function naturalSideOrderAfterRename(options: {
  definition: WireFlowNodeDefinition;
  side: FlowPortSide;
  visualOrderNewIds: readonly string[];
}): string[] {
  // Reconstruct the order the runtime would resolve from the definition
  // when the saved flow carries no `port_order` for this side: fixed
  // ports first (in definition order), then each dynamic group's
  // renumbered members in numeric-suffix order.
  const fixedPortDefs = fixedPortDefsForSide(options.definition, options.side);
  const visualSet = new Set(options.visualOrderNewIds);
  const fixedOrder = fixedPortDefs
    .map((port) => port.name)
    .filter((name) => visualSet.has(name));

  const dynamicOrders = dynamicPortGroupsForSide(
    options.definition,
    options.side,
  ).map((group) => {
    const matched = options.visualOrderNewIds.filter((id) =>
      matchesGroupTemplate(group.name_template, id),
    );
    matched.sort(
      (left, right) =>
        extractSuffixForSort(group.name_template, left) -
        extractSuffixForSort(group.name_template, right),
    );
    return matched;
  });

  return [...fixedOrder, ...dynamicOrders.flat()];
}

function fixedPortDefsForSide(
  definition: WireFlowNodeDefinition,
  side: FlowPortSide,
): ReadonlyArray<{ name: string }> {
  if (side === "input") {
    return definition.input_ports ?? [];
  }
  if (side === "output") {
    return definition.output_ports ?? [];
  }
  if (side === "action") {
    return definition.action_ports ?? [];
  }
  return definition.signal_ports ?? [];
}

function matchesGroupTemplate(template: string, portId: string): boolean {
  const placeholder = "{index}";
  const at = template.indexOf(placeholder);
  if (at < 0) {
    return portId === template;
  }
  const prefix = template.slice(0, at);
  const suffix = template.slice(at + placeholder.length);
  if (!portId.startsWith(prefix) || !portId.endsWith(suffix)) {
    return false;
  }
  const middle = portId.slice(prefix.length, portId.length - suffix.length);
  return /^\d+$/.test(middle);
}

function extractSuffixForSort(template: string, portId: string): number {
  const placeholder = "{index}";
  const at = template.indexOf(placeholder);
  if (at < 0) {
    return 0;
  }
  const prefix = template.slice(0, at);
  const suffix = template.slice(at + placeholder.length);
  const middle = portId.slice(prefix.length, portId.length - suffix.length);
  const parsed = Number.parseInt(middle, 10);
  return Number.isFinite(parsed) ? parsed : 0;
}

function remapEdgesAfterPortRename(options: {
  edges: readonly FlowStudioEdge[];
  nextIndexByPortId: ReadonlyMap<string, number>;
  nodeId: string;
  oldToNewId: ReadonlyMap<string, string>;
  previousPorts: readonly FlowStudioResolvedPort[];
  side: FlowPortSide;
}): FlowStudioEdge[] {
  const attachesToTarget =
    options.side === "input" || options.side === "action";
  const result: FlowStudioEdge[] = [];

  for (const edge of options.edges) {
    const onThisNode = attachesToTarget
      ? edge.target === options.nodeId
      : edge.source === options.nodeId;
    if (!onThisNode) {
      result.push(edge);
      continue;
    }

    const handle = attachesToTarget ? edge.targetHandle : edge.sourceHandle;
    const previousIndex = extractHandleIndex(options.side, handle);
    if (previousIndex === null) {
      result.push(edge);
      continue;
    }

    const oldId = options.previousPorts[previousIndex]?.id;
    if (!oldId) {
      result.push(edge);
      continue;
    }

    const newId = options.oldToNewId.get(oldId) ?? oldId;
    const nextIndex = options.nextIndexByPortId.get(newId);
    if (nextIndex === undefined) {
      result.push(edge);
      continue;
    }

    const nextHandle = createFlowPortHandleId(options.side, nextIndex);
    if (nextHandle === handle) {
      result.push(edge);
      continue;
    }

    result.push(
      attachesToTarget
        ? { ...edge, targetHandle: nextHandle }
        : { ...edge, sourceHandle: nextHandle },
    );
  }

  return result;
}

function extractHandleIndex(
  side: FlowPortSide,
  handleId: string | null | undefined,
): number | null {
  if (!handleId?.startsWith(`${side}-`)) {
    return null;
  }

  const index = Number.parseInt(handleId.slice(side.length + 1), 10);
  return Number.isFinite(index) ? index : null;
}
