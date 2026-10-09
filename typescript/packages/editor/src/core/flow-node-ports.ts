/**
 * Copyright (c) Scott A Dixon
 *
 * Implements flow node ports for the editor's dataflow workspace.
 */
import { tokenConnectionCompatible } from "@battersea/flow";
import type { Edge } from "@xyflow/react";
import type {
  FlowNode as WireFlowNode,
  FlowPortMode,
  FlowPortPhase,
  FlowNodeClass as WireFlowNodeClass,
  FlowParameterDefinition as WireFlowParameterDefinition,
} from "@battersea/flow";

import {
  inspectAuthoringGraphConnection,
  resolveAuthoringGraphConnection,
  type AuthoringGraphHandleDescriptor,
} from "../graph.js";
import {
  hasRotatedControllerPortAxis,
  resolveFlowControllerPortVisualSide,
  type FlowControllerPortPlacement,
} from "./flow-controller-port-placement.js";
import type { FlowStudioEdgeData } from "./dataflow-editor-state.js";

export type FlowPortSide =
  | "action"
  | "automation"
  | "input"
  | "output"
  | "signal";
export type FlowConnectionKind = "signal" | "token";
export type FlowStudioPortNames = WireFlowNode["port_names"];
export type FlowStudioPortDisplayClass = "inline" | "sink" | "source";

export interface FlowStudioResolvedPort {
  mode?: FlowPortMode;
  phase?: FlowPortPhase;
  acceptedTokenTypes?: string[];
  displayClass: FlowStudioPortDisplayClass;
  id: string;
  label: string;
  longDescription?: string;
  name?: string;
  parameters?: WireFlowParameterDefinition[];
  shortDescription?: string;
  side: FlowPortSide;
  tokenType?: string;
}

export type FlowStudioPortLike = FlowStudioResolvedPort | string;

export interface FlowPortConnectionLike {
  source?: string | null;
  sourceHandle?: string | null;
  target?: string | null;
  targetHandle?: string | null;
}

interface FlowConnectionDescriptorNodeLike
  extends FlowStudioConnectionNodeLike {
  data?: FlowStudioConnectionNodeLike["data"] & {
    controllerPortPlacement?: FlowControllerPortPlacement;
    nodeClass?: WireFlowNodeClass | null;
  };
}

export interface FlowPortSlot {
  handleId: string;
  index: number;
  offsetPixels: number;
  offsetPercent: number;
  sideCount: number;
}

export interface FlowPortSlotLayout {
  handleGapPx: number;
  nodeHeightPx: number;
  nodeWidthPx: number;
  pillMaxWidthPx: number;
  slots: FlowPortSlot[];
}

export interface FlowPortSideCounts {
  actionCount: number;
  automationCount: number;
  inputCount: number;
  outputCount: number;
  signalCount: number;
}

const FLOW_NODE_BASE_WIDTH_PX = 228;
const FLOW_NODE_BASE_HEIGHT_PX = 116;
const FLOW_INSTRUMENT_NODE_BASE_WIDTH_PX = 184;
const FLOW_INSTRUMENT_NODE_BASE_HEIGHT_PX = 92;
const FLOW_LOGIC_NODE_BASE_WIDTH_PX = 128;
const FLOW_LOGIC_NODE_BASE_HEIGHT_PX = 116;
const FLOW_NODE_PORT_EDGE_PADDING_PX = 22;
const FLOW_NODE_PORT_GAP_PX = 14;
const FLOW_NODE_PORT_PILL_MAX_WIDTH_PX = 156;
const FLOW_INSTRUMENT_PORT_PILL_MAX_WIDTH_PX = 120;
const FLOW_LOGIC_PORT_PILL_MAX_WIDTH_PX = 72;
const FLOW_NODE_PORT_SLOT_PITCH_PX = 28;

type FlowStudioConnectionNodeLike = {
  data?: {
    actionPorts?: readonly FlowStudioPortLike[];
    automationPorts?: readonly FlowStudioPortLike[];
    inputPorts?: readonly FlowStudioPortLike[];
    outputPorts?: readonly FlowStudioPortLike[];
    signalPorts?: readonly FlowStudioPortLike[];
  };
  id: string;
};

export function getFlowPortLabel(portId: string): string {
  return portId
    .replace(/[_-]+/g, " ")
    .replace(/\b\w/g, (character) => character.toUpperCase())
    .trim();
}

export function resolveFlowPortAliasLabel(
  portId: string,
  alias?: string | null,
): string {
  const trimmedAlias = alias?.trim();
  return trimmedAlias ? trimmedAlias : getFlowPortLabel(portId);
}

export function normaliseFlowPortAlias(alias: string): string | null {
  const trimmedAlias = alias.trim();
  return trimmedAlias ? trimmedAlias : null;
}

export function buildResolvedFlowPort(options: {
  mode?: FlowPortMode;
  phase?: FlowPortPhase;
  acceptedTokenTypes?: string[] | null;
  displayClass?: FlowStudioPortDisplayClass | null;
  id: string;
  longDescription?: string | null;
  name?: string | null;
  nodeClass?: WireFlowNodeClass | null;
  parameters?: readonly WireFlowParameterDefinition[] | null;
  shortDescription?: string | null;
  side: FlowPortSide;
  tokenType?: string | null;
}): FlowStudioResolvedPort {
  const alias = normaliseFlowPortAlias(options.name ?? "");

  return {
    id: options.id,
    mode: options.mode,
    phase: options.phase,
    label: resolveFlowPortAliasLabel(options.id, alias),
    acceptedTokenTypes:
      Array.isArray(options.acceptedTokenTypes) &&
      options.acceptedTokenTypes.length > 0
        ? [...options.acceptedTokenTypes]
        : undefined,
    displayClass: resolveFlowPortDisplayClass(
      options.nodeClass,
      options.displayClass,
    ),
    longDescription: options.longDescription ?? undefined,
    name: alias ?? undefined,
    parameters:
      options.parameters && options.parameters.length > 0
        ? [...options.parameters]
        : undefined,
    shortDescription: options.shortDescription ?? undefined,
    side: options.side,
    tokenType: options.tokenType ?? undefined,
  };
}

export function resolveFlowStudioResolvedPorts(
  ports: readonly FlowStudioPortLike[],
  side: FlowPortSide,
  nodeClass?: WireFlowNodeClass | null,
): FlowStudioResolvedPort[] {
  return ports.flatMap((port) => {
    if (typeof port === "string") {
      return [
        buildResolvedFlowPort({
          id: port,
          nodeClass,
          side,
        }),
      ];
    }

    return isFlowStudioResolvedPort(port)
      ? [
          {
            ...port,
            displayClass: resolveFlowPortDisplayClass(
              nodeClass,
              port.displayClass,
            ),
          },
        ]
      : [];
  });
}

export function resolveFlowPortDisplayClass(
  nodeClass?: WireFlowNodeClass | null,
  explicitDisplayClass?: FlowStudioPortDisplayClass | null,
): FlowStudioPortDisplayClass {
  if (
    explicitDisplayClass === "source" ||
    explicitDisplayClass === "inline" ||
    explicitDisplayClass === "sink"
  ) {
    return explicitDisplayClass;
  }

  if (
    nodeClass === "source" ||
    nodeClass === "inline" ||
    nodeClass === "sink"
  ) {
    return nodeClass;
  }

  return "inline";
}

export function cloneFlowStudioPortNames(
  portNames: FlowStudioPortNames | null | undefined,
): FlowStudioPortNames | undefined {
  if (!portNames) {
    return undefined;
  }

  const clone = {
    action: portNames.action ? { ...portNames.action } : undefined,
    automation: portNames.automation ? { ...portNames.automation } : undefined,
    input: portNames.input ? { ...portNames.input } : undefined,
    output: portNames.output ? { ...portNames.output } : undefined,
    signal: portNames.signal ? { ...portNames.signal } : undefined,
  } satisfies FlowStudioPortNames;

  return hasFlowStudioPortNames(clone) ? clone : undefined;
}

export function getFlowPortAlias(
  portNames: FlowStudioPortNames | null | undefined,
  side: FlowPortSide,
  portId: string,
): string | undefined {
  const alias = portNames?.[side]?.[portId];
  const trimmed = alias?.trim();
  return trimmed ? trimmed : undefined;
}

export function setFlowPortAlias(options: {
  nextAlias: string | null;
  portId: string;
  portNames: FlowStudioPortNames | null | undefined;
  side: FlowPortSide;
}): FlowStudioPortNames | undefined {
  const current = cloneFlowStudioPortNames(options.portNames) ?? {};
  const nextSide = { ...(current[options.side] ?? {}) };

  if (options.nextAlias) {
    nextSide[options.portId] = options.nextAlias;
  } else {
    delete nextSide[options.portId];
  }

  const nextPortNames = {
    ...current,
    [options.side]: Object.keys(nextSide).length > 0 ? nextSide : undefined,
  } satisfies FlowStudioPortNames;

  return hasFlowStudioPortNames(nextPortNames) ? nextPortNames : undefined;
}

export function pruneFlowPortAliases(
  portNames: FlowStudioPortNames | null | undefined,
  side: FlowPortSide,
  validPortIds: readonly string[],
): FlowStudioPortNames | undefined {
  const current = cloneFlowStudioPortNames(portNames);
  if (!current?.[side]) {
    return current;
  }

  const validIds = new Set(validPortIds);
  const nextSide = Object.fromEntries(
    Object.entries(current[side] ?? {}).filter(([portId]) =>
      validIds.has(portId),
    ),
  );
  const nextPortNames = {
    ...current,
    [side]: Object.keys(nextSide).length > 0 ? nextSide : undefined,
  } satisfies FlowStudioPortNames;

  return hasFlowStudioPortNames(nextPortNames) ? nextPortNames : undefined;
}

export function hasFlowStudioPortNames(
  portNames: FlowStudioPortNames | null | undefined,
): boolean {
  return Boolean(
    portNames &&
      (Object.keys(portNames.action ?? {}).length > 0 ||
        Object.keys(portNames.automation ?? {}).length > 0 ||
        Object.keys(portNames.input ?? {}).length > 0 ||
        Object.keys(portNames.output ?? {}).length > 0 ||
        Object.keys(portNames.signal ?? {}).length > 0),
  );
}

function isFlowStudioResolvedPort(
  value: unknown,
): value is FlowStudioResolvedPort {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return false;
  }

  const candidate = value as Partial<FlowStudioResolvedPort>;
  return (
    typeof candidate.id === "string" &&
    typeof candidate.label === "string" &&
    (candidate.displayClass === "source" ||
      candidate.displayClass === "inline" ||
      candidate.displayClass === "sink") &&
    (candidate.side === "action" ||
      candidate.side === "automation" ||
      candidate.side === "input" ||
      candidate.side === "output" ||
      candidate.side === "signal")
  );
}

export function createFlowPortHandleId(
  side: FlowPortSide,
  index: number,
): string {
  return `${side}-${index}`;
}

function inferHandleSide(
  handleId: string | null | undefined,
): FlowPortSide | null {
  if (typeof handleId !== "string") {
    return null;
  }

  if (handleId.startsWith("action-")) {
    return "action";
  }
  if (handleId.startsWith("automation-")) {
    return "automation";
  }
  if (handleId.startsWith("input-")) {
    return "input";
  }
  if (handleId.startsWith("output-")) {
    return "output";
  }
  if (handleId.startsWith("signal-")) {
    return "signal";
  }

  return null;
}

function getFlowFamilyForSide(side: FlowPortSide): FlowConnectionKind {
  return side === "action" || side === "signal" ? "signal" : "token";
}

function getFlowRenderedHandleSide(
  side: FlowPortSide,
  controllerPortPlacement?: FlowControllerPortPlacement,
  nodeClass?: WireFlowNodeClass | null,
): "bottom" | "left" | "right" | "top" {
  if (side === "input") {
    return "left";
  }
  if (side === "output") {
    return "right";
  }

  return resolveFlowControllerPortVisualSide({
    nodeClass,
    placement: controllerPortPlacement,
    side,
  });
}

export function buildFlowHandleDescriptors(options: {
  actionPorts?: readonly FlowStudioPortLike[];
  automationPorts?: readonly FlowStudioPortLike[];
  controllerPortPlacement?: FlowControllerPortPlacement;
  inputPorts?: readonly FlowStudioPortLike[];
  nodeClass?: WireFlowNodeClass | null;
  outputPorts?: readonly FlowStudioPortLike[];
  signalPorts?: readonly FlowStudioPortLike[];
}): AuthoringGraphHandleDescriptor<FlowConnectionKind>[] {
  return [
    ...buildFlowSideHandleDescriptors(
      "action",
      options.actionPorts ?? [],
      options.controllerPortPlacement,
      options.nodeClass,
    ),
    ...buildFlowSideHandleDescriptors(
      "automation",
      options.automationPorts ?? [],
      options.controllerPortPlacement,
      options.nodeClass,
    ),
    ...buildFlowSideHandleDescriptors(
      "input",
      options.inputPorts ?? [],
      options.controllerPortPlacement,
      options.nodeClass,
    ),
    ...buildFlowSideHandleDescriptors(
      "output",
      options.outputPorts ?? [],
      options.controllerPortPlacement,
      options.nodeClass,
    ),
    ...buildFlowSideHandleDescriptors(
      "signal",
      options.signalPorts ?? [],
      options.controllerPortPlacement,
      options.nodeClass,
    ),
  ];
}

function buildFlowSideHandleDescriptors(
  side: FlowPortSide,
  ports: readonly FlowStudioPortLike[],
  controllerPortPlacement?: FlowControllerPortPlacement,
  nodeClass?: WireFlowNodeClass | null,
): AuthoringGraphHandleDescriptor<FlowConnectionKind>[] {
  return resolveFlowStudioResolvedPorts(ports, side).map((port, index) => ({
    data: {
      logicalSide: side,
      portId: port.id,
    },
    direction: side === "output" || side === "signal" ? "source" : "target",
    family: getFlowFamilyForSide(side),
    handleId: createFlowPortHandleId(side, index),
    label: port.label,
    orderIndex: index,
    side: getFlowRenderedHandleSide(side, controllerPortPlacement, nodeClass),
  }));
}

function buildFlowFallbackHandleDescriptor(
  handleId: string | null | undefined,
): AuthoringGraphHandleDescriptor<FlowConnectionKind> | null {
  const side = inferHandleSide(handleId);
  if (!side || !handleId) {
    return null;
  }

  const index = extractFlowPortHandleIndex(side, handleId) ?? 0;
  return {
    data: {
      logicalSide: side,
    },
    direction: side === "output" || side === "signal" ? "source" : "target",
    family: getFlowFamilyForSide(side),
    handleId,
    orderIndex: index,
    side: getFlowRenderedHandleSide(side),
  };
}

function getFlowDefaultHandleIds(family: FlowConnectionKind): {
  sourceHandleId: string;
  targetHandleId: string;
} {
  return family === "signal"
    ? {
        sourceHandleId: "signal-0",
        targetHandleId: "action-0",
      }
    : {
        sourceHandleId: "output-0",
        targetHandleId: "input-0",
      };
}

function resolveFlowConnection(
  connection: FlowPortConnectionLike,
  nodes?: ReadonlyArray<FlowConnectionDescriptorNodeLike>,
) {
  const sourceNode = nodes?.find((node) => node.id === connection.source);
  const targetNode = nodes?.find((node) => node.id === connection.target);
  const sourceHandles = sourceNode
    ? buildFlowHandleDescriptors({
        actionPorts: sourceNode.data?.actionPorts,
        automationPorts: sourceNode.data?.automationPorts,
        controllerPortPlacement: sourceNode.data?.controllerPortPlacement,
        inputPorts: sourceNode.data?.inputPorts,
        nodeClass: sourceNode.data?.nodeClass,
        outputPorts: sourceNode.data?.outputPorts,
        signalPorts: sourceNode.data?.signalPorts,
      })
    : [buildFlowFallbackHandleDescriptor(connection.sourceHandle)].filter(
        (
          descriptor,
        ): descriptor is AuthoringGraphHandleDescriptor<FlowConnectionKind> =>
          descriptor !== null,
      );
  const targetHandles = targetNode
    ? buildFlowHandleDescriptors({
        actionPorts: targetNode.data?.actionPorts,
        automationPorts: targetNode.data?.automationPorts,
        controllerPortPlacement: targetNode.data?.controllerPortPlacement,
        inputPorts: targetNode.data?.inputPorts,
        nodeClass: targetNode.data?.nodeClass,
        outputPorts: targetNode.data?.outputPorts,
        signalPorts: targetNode.data?.signalPorts,
      })
    : [buildFlowFallbackHandleDescriptor(connection.targetHandle)].filter(
        (
          descriptor,
        ): descriptor is AuthoringGraphHandleDescriptor<FlowConnectionKind> =>
          descriptor !== null,
      );

  const preferredDefaults = getFlowDefaultHandleIds("token");

  return resolveAuthoringGraphConnection({
    allowSelfLoops: true,
    connection,
    defaultSourceHandleId: preferredDefaults.sourceHandleId,
    defaultTargetHandleId: preferredDefaults.targetHandleId,
    sourceHandles,
    targetHandles,
  });
}

export function resolveFlowConnectionKind(
  connection: FlowPortConnectionLike,
): FlowConnectionKind | null {
  return resolveFlowConnection(connection)?.family ?? null;
}

function getEdgeKind(
  edge: Pick<
    Edge<FlowStudioEdgeData>,
    "data" | "sourceHandle" | "targetHandle"
  >,
): FlowConnectionKind | null {
  if (edge.data?.kind === "signal") {
    return "signal";
  }
  if (edge.data?.kind === "token") {
    return "token";
  }

  return resolveFlowConnectionKind(edge);
}

export function buildFlowPortSlots(
  side: FlowPortSide,
  sideCounts: FlowPortSideCounts,
  nodeClass?: WireFlowNodeClass | null,
): FlowPortSlotLayout {
  const count = getFlowPortSideCount(side, sideCounts);
  const rotatedControllerAxis = hasRotatedControllerPortAxis(nodeClass);
  const logicNode = nodeClass === "logic";
  const nodeHeightPx = resolveFlowPortAxisLength(
    logicNode
      ? FLOW_LOGIC_NODE_BASE_HEIGHT_PX
      : rotatedControllerAxis
        ? FLOW_INSTRUMENT_NODE_BASE_HEIGHT_PX
        : FLOW_NODE_BASE_HEIGHT_PX,
    rotatedControllerAxis
      ? Math.max(
          sideCounts.inputCount,
          sideCounts.outputCount,
          sideCounts.actionCount,
          sideCounts.signalCount,
        )
      : Math.max(sideCounts.inputCount, sideCounts.outputCount),
  );
  const nodeWidthPx = resolveFlowPortAxisLength(
    logicNode
      ? FLOW_LOGIC_NODE_BASE_WIDTH_PX
      : rotatedControllerAxis
        ? FLOW_INSTRUMENT_NODE_BASE_WIDTH_PX
        : FLOW_NODE_BASE_WIDTH_PX,
    rotatedControllerAxis
      ? 0
      : Math.max(sideCounts.actionCount, sideCounts.signalCount),
  );
  const axisLengthPx =
    side === "input" ||
    side === "output" ||
    (rotatedControllerAxis &&
      (side === "action" || side === "automation" || side === "signal"))
      ? nodeHeightPx
      : nodeWidthPx;

  return {
    handleGapPx: FLOW_NODE_PORT_GAP_PX,
    nodeHeightPx,
    nodeWidthPx,
    pillMaxWidthPx: logicNode
      ? FLOW_LOGIC_PORT_PILL_MAX_WIDTH_PX
      : rotatedControllerAxis
        ? FLOW_INSTRUMENT_PORT_PILL_MAX_WIDTH_PX
        : FLOW_NODE_PORT_PILL_MAX_WIDTH_PX,
    slots: Array.from({ length: count }, (_, index) => {
      const offsetPixels = resolveFlowPortOffsetPixels(
        axisLengthPx,
        count,
        index,
      );

      return {
        handleId: createFlowPortHandleId(side, index),
        index,
        offsetPixels,
        offsetPercent: Number(((offsetPixels / axisLengthPx) * 100).toFixed(3)),
        sideCount: count,
      };
    }),
  };
}

function getFlowPortSideCount(
  side: FlowPortSide,
  counts: FlowPortSideCounts,
): number {
  if (side === "action") {
    return counts.actionCount;
  }
  if (side === "automation") {
    return counts.automationCount;
  }
  if (side === "input") {
    return counts.inputCount;
  }
  if (side === "output") {
    return counts.outputCount;
  }

  return counts.signalCount;
}

function resolveFlowPortAxisLength(
  baseLengthPx: number,
  count: number,
): number {
  if (count <= 1) {
    return baseLengthPx;
  }

  return Math.max(
    baseLengthPx,
    FLOW_NODE_PORT_EDGE_PADDING_PX * 2 +
      (count - 1) * FLOW_NODE_PORT_SLOT_PITCH_PX,
  );
}

function resolveFlowPortOffsetPixels(
  axisLengthPx: number,
  count: number,
  index: number,
): number {
  if (count <= 1) {
    return Number((axisLengthPx / 2).toFixed(3));
  }

  const usableSpanPx = axisLengthPx - FLOW_NODE_PORT_EDGE_PADDING_PX * 2;
  const offsetPixels =
    FLOW_NODE_PORT_EDGE_PADDING_PX + (usableSpanPx / (count - 1)) * index;

  return Number(offsetPixels.toFixed(3));
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

export function pruneEdgesForNodeCardinality(
  edges: Edge<FlowStudioEdgeData>[],
  nodeId: string,
  side: FlowPortSide,
  nextCount: number,
): Edge<FlowStudioEdgeData>[] {
  return edges.filter((edge) => {
    if (
      (side === "input" || side === "action" || side === "automation") &&
      edge.target === nodeId
    ) {
      const index = extractFlowPortHandleIndex(side, edge.targetHandle);
      if (index === null) {
        return true;
      }
      if (nextCount === 0) {
        return false;
      }

      return index < nextCount;
    }

    if ((side === "output" || side === "signal") && edge.source === nodeId) {
      const index = extractFlowPortHandleIndex(side, edge.sourceHandle);
      if (index === null) {
        return true;
      }
      if (nextCount === 0) {
        return false;
      }

      return index < nextCount;
    }

    return true;
  });
}

export function canAttachConnectionToPorts(
  edges: Array<
    Pick<
      Edge<FlowStudioEdgeData>,
      "data" | "source" | "sourceHandle" | "target" | "targetHandle"
    >
  >,
  connection: FlowPortConnectionLike,
  nodes?: ReadonlyArray<FlowConnectionDescriptorNodeLike>,
): boolean {
  const resolvedConnection = resolveFlowConnection(connection, nodes);
  if (!resolvedConnection) {
    return false;
  }

  if (
    resolvedConnection.family === "token" &&
    nodes &&
    !connectionUsesCompatibleTokenTypes({
      connection,
      nodes,
    })
  ) {
    return false;
  }

  return !connectionConflictsWithExistingEdge(edges, connection);
}

export function connectionConflictsWithExistingEdge(
  edges: Array<
    Pick<
      Edge<FlowStudioEdgeData>,
      "data" | "source" | "sourceHandle" | "target" | "targetHandle"
    >
  >,
  connection: FlowPortConnectionLike,
): boolean {
  const resolvedConnection = resolveFlowConnection(connection);
  if (!resolvedConnection) {
    return false;
  }

  const defaultHandles = getFlowDefaultHandleIds(resolvedConnection.family);

  return edges.some(
    (edge) =>
      (getEdgeKind(edge) === resolvedConnection.family &&
        edge.source === resolvedConnection.source &&
        (edge.sourceHandle ?? defaultHandles.sourceHandleId) ===
          resolvedConnection.sourceHandleId) ||
      (getEdgeKind(edge) === resolvedConnection.family &&
        edge.target === resolvedConnection.target &&
        (edge.targetHandle ?? defaultHandles.targetHandleId) ===
          resolvedConnection.targetHandleId),
  );
}

export function resolveConnectionFromHandlePair(params: {
  fromHandle: { id?: string | null; nodeId: string } | null;
  toHandle: { id?: string | null; nodeId: string } | null;
}): FlowPortConnectionLike | null {
  if (!params.fromHandle || !params.toHandle) {
    return null;
  }

  const connection = {
    source: params.fromHandle.nodeId,
    sourceHandle: params.fromHandle.id ?? null,
    target: params.toHandle.nodeId,
    targetHandle: params.toHandle.id ?? null,
  };

  const inspection = inspectAuthoringGraphConnection({
    allowSelfLoops: true,
    connection,
    sourceHandles: [
      buildFlowFallbackHandleDescriptor(connection.sourceHandle),
    ].filter(
      (
        descriptor,
      ): descriptor is AuthoringGraphHandleDescriptor<FlowConnectionKind> =>
        descriptor !== null,
    ),
    targetHandles: [
      buildFlowFallbackHandleDescriptor(connection.targetHandle),
    ].filter(
      (
        descriptor,
      ): descriptor is AuthoringGraphHandleDescriptor<FlowConnectionKind> =>
        descriptor !== null,
    ),
  });

  return inspection.valid ? connection : null;
}

function resolvePortByHandle(options: {
  handleId: string | null | undefined;
  node: FlowStudioConnectionNodeLike | undefined;
  side: FlowPortSide;
}): FlowStudioResolvedPort | null {
  if (!options.node) {
    return null;
  }

  const ports =
    options.side === "action"
      ? resolveFlowStudioResolvedPorts(
          options.node.data?.actionPorts ?? [],
          "action",
        )
      : options.side === "automation"
        ? resolveFlowStudioResolvedPorts(
            options.node.data?.automationPorts ?? [],
            "automation",
          )
        : options.side === "input"
          ? resolveFlowStudioResolvedPorts(
              options.node.data?.inputPorts ?? [],
              "input",
            )
          : options.side === "output"
            ? resolveFlowStudioResolvedPorts(
                options.node.data?.outputPorts ?? [],
                "output",
              )
            : resolveFlowStudioResolvedPorts(
                options.node.data?.signalPorts ?? [],
                "signal",
              );
  const index = extractFlowPortHandleIndex(options.side, options.handleId);
  if (index === null) {
    return ports[0] ?? null;
  }

  return ports[index] ?? null;
}

// Mirror of the engine's `effective_accepted_token_types`
// (battersea-flow/src/ports.rs): a port's real
// accepted set is its `acceptedTokenTypes` when non-empty, otherwise
// just its declared `tokenType`. Kept identical on purpose so the
// editor's drag-time check and the engine's validation agree.
function effectiveAcceptedTokenTypes(port: {
  tokenType?: string | null;
  acceptedTokenTypes?: string[];
}): string[] {
  if (port.acceptedTokenTypes?.length) {
    return port.acceptedTokenTypes;
  }
  return port.tokenType ? [port.tokenType] : [];
}

export function connectionUsesCompatibleTokenTypes(options: {
  connection: FlowPortConnectionLike;
  nodes: ReadonlyArray<FlowConnectionDescriptorNodeLike>;
}): boolean {
  const sourceNode = options.nodes.find(
    (node) => node.id === options.connection.source,
  );
  const targetNode = options.nodes.find(
    (node) => node.id === options.connection.target,
  );
  const sourcePort = resolvePortByHandle({
    handleId: options.connection.sourceHandle,
    node: sourceNode,
    side: "output",
  });
  // Token edges may target either an input port or a (typed) automation port.
  // The automation case is rare today but follows the same compatibility
  // rule as input ports because both are typed sinks.
  const targetSide =
    inferHandleSide(options.connection.targetHandle) === "automation"
      ? "automation"
      : "input";
  const targetPort = resolvePortByHandle({
    handleId: options.connection.targetHandle,
    node: targetNode,
    side: targetSide,
  });

  if (!sourcePort?.tokenType || !targetPort?.tokenType) {
    return false;
  }

  // An "auto" source can only ever carry one of its node's input
  // accepted types; gather that union for the shared rule below.
  const sourceNodeInputAccepted =
    sourcePort.tokenType === "auto"
      ? resolveFlowStudioResolvedPorts(
          sourceNode?.data?.inputPorts ?? [],
          "input",
        ).flatMap(effectiveAcceptedTokenTypes)
      : [];

  return tokenConnectionCompatible(
    sourcePort.tokenType,
    sourceNodeInputAccepted,
    effectiveAcceptedTokenTypes(targetPort),
  );
}

export function resolveConnectionSourceTokenType(options: {
  connection: FlowPortConnectionLike;
  nodes: ReadonlyArray<FlowConnectionDescriptorNodeLike>;
}): string | null {
  if (resolveFlowConnectionKind(options.connection) !== "token") {
    return null;
  }

  const sourceNode = options.nodes.find(
    (node) => node.id === options.connection.source,
  );
  const sourcePort = resolvePortByHandle({
    handleId: options.connection.sourceHandle,
    node: sourceNode,
    side: "output",
  });

  return sourcePort?.tokenType ?? null;
}
