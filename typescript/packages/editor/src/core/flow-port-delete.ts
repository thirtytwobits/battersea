/**
 * Copyright (c) Scott A Dixon
 *
 * Deletes a single variadic (dynamic-group) port from a flow node in one step.
 *
 * Dynamic ports are generated positionally from a count parameter
 * (`value_0 … value_{count-1}`), so shrinking the count only ever drops the
 * highest index. Deleting an arbitrary port therefore means: drop the target's
 * alias/parameters/edges, renumber the surviving dynamic ports down into the
 * contiguous id space the new count produces, shift their aliases, per-port
 * parameters, and edges to follow, and decrement the count parameter. This
 * replaces the manual "reorder-to-end then shrink" workaround.
 */
import type {
  FlowDynamicPortGroup as WireFlowDynamicPortGroup,
  FlowNodeDefinition as WireFlowNodeDefinition,
} from "@battersea/flow";

import type {
  FlowStudioEdge,
  FlowStudioNode,
} from "./dataflow-editor-state.js";
import {
  dynamicPortTemplateId,
  dynamicPortTemplateIndex,
} from "./flow-dynamic-port-template.js";
import {
  buildResolvedNodeData,
  cloneFlowPortParameterValues,
  getEffectiveParameterValue,
  type FlowParameterValues,
} from "./flow-node-definitions.js";
import {
  cloneFlowStudioPortNames,
  createFlowPortHandleId,
  resolveFlowStudioResolvedPorts,
  type FlowPortSide,
  type FlowStudioResolvedPort,
} from "./flow-node-ports.js";
import {
  cloneFlowStudioPortOrder,
  type FlowStudioPortOrder,
} from "./flow-port-order.js";
import type { FlowStudioWorkspaceState } from "./flow-persistence.js";

interface DynamicPortGroupMatch {
  group: WireFlowDynamicPortGroup;
  index: number;
}

/**
 * Resolves the dynamic port group (and the port's index within it) that owns
 * `portId` on `side`, or `null` when the port is fixed / not variadic.
 */
export function resolveDynamicPortGroupMatch(
  definition: WireFlowNodeDefinition,
  side: FlowPortSide,
  portId: string,
): DynamicPortGroupMatch | null {
  if (side !== "input" && side !== "output") {
    return null;
  }

  const groups =
    side === "input"
      ? (definition.dynamic_input_ports ?? [])
      : (definition.dynamic_output_ports ?? []);
  for (const group of groups) {
    const index = dynamicPortTemplateIndex(group.name_template, portId);
    if (index !== null) {
      return { group, index };
    }
  }

  return null;
}

/**
 * True when `portId` is a variadic port whose group still has more ports than
 * its minimum, i.e. it can be deleted without violating the count's lower bound.
 */
export function canDeleteVariadicPort(options: {
  definition: WireFlowNodeDefinition;
  parameterValues: FlowParameterValues;
  portId: string;
  side: FlowPortSide;
}): boolean {
  const match = resolveDynamicPortGroupMatch(
    options.definition,
    options.side,
    options.portId,
  );
  if (!match) {
    return false;
  }

  return (
    resolveGroupCount(
      options.definition,
      match.group,
      options.parameterValues,
    ) > resolveGroupMinimum(options.definition, match.group)
  );
}

export function deleteVariadicPortInWorkspace(options: {
  definition: WireFlowNodeDefinition;
  nodeId: string;
  portId: string;
  side: FlowPortSide;
  workspace: FlowStudioWorkspaceState;
}): FlowStudioWorkspaceState {
  const node = options.workspace.nodes.find(
    (candidate) => candidate.id === options.nodeId,
  );
  if (!node) {
    return options.workspace;
  }

  const match = resolveDynamicPortGroupMatch(
    options.definition,
    options.side,
    options.portId,
  );
  if (!match) {
    return options.workspace;
  }

  const parameterValues = node.data.parameterValues;
  const count = resolveGroupCount(
    options.definition,
    match.group,
    parameterValues,
  );
  if (count <= resolveGroupMinimum(options.definition, match.group)) {
    return options.workspace;
  }

  const previousPorts = resolveNodePortsForSide(node, options.side);
  if (!previousPorts.some((port) => port.id === options.portId)) {
    return options.workspace;
  }

  // Renumber the surviving dynamic ports into a contiguous id space, in their
  // current display order. Fixed ports (and ports from other dynamic groups)
  // keep their ids. `oldToNewId` excludes the target, so its alias/params/edges
  // are dropped rather than remapped.
  const template = match.group.name_template;
  const survivors = previousPorts.filter((port) => port.id !== options.portId);
  const oldToNewId = new Map<string, string>();
  let dynamicSeq = 0;
  for (const port of survivors) {
    if (dynamicPortTemplateIndex(template, port.id) !== null) {
      oldToNewId.set(port.id, dynamicPortTemplateId(template, dynamicSeq));
      dynamicSeq += 1;
    } else {
      oldToNewId.set(port.id, port.id);
    }
  }

  const nextPortNames = rekeySidePortNames(node, options.side, oldToNewId);
  const nextPortParameterValues = rekeySidePortParameters(
    node,
    options.side,
    oldToNewId,
  );
  const nextParameterValues: FlowParameterValues = {
    ...parameterValues,
    [match.group.count_parameter]: count - 1,
  };
  const previousOrder = cloneFlowStudioPortOrder(node.data.portOrder) ?? {};
  const nextPortOrder: FlowStudioPortOrder = {
    ...previousOrder,
    [options.side]: survivors
      .map((port) => oldToNewId.get(port.id))
      .filter((portId): portId is string => typeof portId === "string"),
  };

  const nextData = buildResolvedNodeData({
    controllerPortPlacement: node.data.controllerPortPlacement,
    definition: options.definition,
    instanceName: node.data.instanceName,
    parameterValues: nextParameterValues,
    portOrder: nextPortOrder,
    portNames: nextPortNames,
    portParameterValues: nextPortParameterValues,
  });
  const nextPorts = resolveNodePortsForSide(
    { ...node, data: nextData },
    options.side,
  );
  const nextIndexByPortId = new Map(
    nextPorts.map((port, index) => [port.id, index] as const),
  );

  const nextEdges = remapEdgesAfterPortDeletion({
    edges: options.workspace.edges,
    nextIndexByPortId,
    nodeId: options.nodeId,
    oldToNewId,
    previousPorts,
    side: options.side,
    targetPortId: options.portId,
  });

  // Drop a now-dangling port selection so the detail pane falls back to the node.
  const selectedTarget = options.workspace.selectedTarget;
  const nextSelectedTarget =
    selectedTarget.kind === "port" &&
    selectedTarget.nodeId === options.nodeId &&
    selectedTarget.side === options.side &&
    selectedTarget.portId === options.portId
      ? { kind: "node" as const, nodeId: options.nodeId }
      : selectedTarget;

  return {
    ...options.workspace,
    edges: nextEdges,
    nodes: options.workspace.nodes.map((candidate) =>
      candidate.id === options.nodeId
        ? {
            ...candidate,
            className: `flow-studio-node flow-studio-node--${nextData.nodeClass}`,
            data: nextData,
          }
        : candidate,
    ),
    selectedTarget: nextSelectedTarget,
  };
}

function resolveNodePortsForSide(
  node: FlowStudioNode,
  side: FlowPortSide,
): FlowStudioResolvedPort[] {
  switch (side) {
    case "action":
      return resolveFlowStudioResolvedPorts(
        node.data.actionPorts,
        "action",
        node.data.nodeClass,
      );
    case "input":
      return resolveFlowStudioResolvedPorts(
        node.data.inputPorts,
        "input",
        node.data.nodeClass,
      );
    case "output":
      return resolveFlowStudioResolvedPorts(
        node.data.outputPorts,
        "output",
        node.data.nodeClass,
      );
    case "signal":
      return resolveFlowStudioResolvedPorts(
        node.data.signalPorts,
        "signal",
        node.data.nodeClass,
      );
    default:
      return resolveFlowStudioResolvedPorts(
        node.data.automationPorts,
        "automation",
        node.data.nodeClass,
      );
  }
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
    const newId = oldToNewId.get(oldId);
    if (newId) {
      nextSide[newId] = alias;
    }
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
): ReturnType<typeof cloneFlowPortParameterValues> {
  if (side !== "input" && side !== "output") {
    return cloneFlowPortParameterValues(node.data.portParameterValues);
  }

  const previous =
    cloneFlowPortParameterValues(node.data.portParameterValues) ?? {};
  const previousSide = previous[side] ?? {};
  const nextSide: Record<string, Record<string, unknown>> = {};
  for (const [oldId, values] of Object.entries(previousSide)) {
    const newId = oldToNewId.get(oldId);
    if (newId) {
      nextSide[newId] = values;
    }
  }

  return {
    ...previous,
    [side]: Object.keys(nextSide).length > 0 ? nextSide : undefined,
  };
}

function remapEdgesAfterPortDeletion(options: {
  edges: readonly FlowStudioEdge[];
  nextIndexByPortId: ReadonlyMap<string, number>;
  nodeId: string;
  oldToNewId: ReadonlyMap<string, string>;
  previousPorts: readonly FlowStudioResolvedPort[];
  side: FlowPortSide;
  targetPortId: string;
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

    // Edge connected to the deleted port: drop it entirely.
    if (oldId === options.targetPortId) {
      continue;
    }

    const newId = options.oldToNewId.get(oldId);
    const nextIndex =
      newId === undefined ? undefined : options.nextIndexByPortId.get(newId);
    if (nextIndex === undefined) {
      // Survivor we can't place (shouldn't happen) — drop rather than dangle.
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

function resolveGroupCount(
  definition: WireFlowNodeDefinition,
  group: WireFlowDynamicPortGroup,
  parameterValues: FlowParameterValues,
): number {
  const raw = getEffectiveParameterValue(
    definition,
    group.count_parameter,
    parameterValues,
  );
  return typeof raw === "number" && Number.isFinite(raw) ? Math.floor(raw) : 0;
}

function resolveGroupMinimum(
  definition: WireFlowNodeDefinition,
  group: WireFlowDynamicPortGroup,
): number {
  const min = (definition.parameters ?? []).find(
    (parameter) => parameter.name === group.count_parameter,
  )?.editor.min;
  return typeof min === "number" && Number.isFinite(min) ? min : 0;
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
