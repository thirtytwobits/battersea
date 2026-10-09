/**
 * Copyright (c) Scott A Dixon
 *
 * Implements flow drag for the editor's dataflow workspace.
 */
import type { XYPosition, Node } from "@xyflow/react";
import type {
  FlowNode as WireFlowNode,
  FlowNodeDefinition as WireFlowNodeDefinition,
} from "@battersea/flow";
import type { FlowControllerPortPlacement } from "./flow-controller-port-placement.js";
import {
  buildDefaultFlowNodeId,
  buildDefaultInstanceName,
  buildDefaultParameterValues,
  buildResolvedNodeData,
} from "./flow-node-definitions.js";
import type {
  FlowStudioPortLike,
  FlowStudioResolvedPort,
} from "./flow-node-ports.js";
import type { FlowStudioPortOrder } from "./flow-port-order.js";

export interface FlowStudioNodeData extends Record<string, unknown> {
  actionPorts: FlowStudioPortLike[];
  automationPorts: FlowStudioPortLike[];
  controllerPortPlacement?: FlowControllerPortPlacement;
  definitionName: string;
  hasController?: boolean;
  inputPorts: FlowStudioPortLike[];
  instanceName: string;
  longDescription: string;
  nodeClass: WireFlowNodeDefinition["kind"];
  parameterValues: Record<string, unknown>;
  portParameterValues?: WireFlowNode["port_parameter_values"];
  portOrder?: FlowStudioPortOrder;
  outputPorts: FlowStudioPortLike[];
  portNames?: Record<string, Record<string, string> | undefined>;
  signalPorts: FlowStudioPortLike[];
  shortDescription: string;
}

export interface FlowNodeDragPayload {
  actionPorts: FlowStudioPortLike[];
  automationPorts: FlowStudioPortLike[];
  definitionName: string;
  dragOffset?: XYPosition;
  hasController: boolean;
  inputPorts: FlowStudioPortLike[];
  longDescription: string;
  nodeClass: WireFlowNodeDefinition["kind"];
  outputPorts: FlowStudioPortLike[];
  parameterValues: Record<string, unknown>;
  signalPorts: FlowStudioPortLike[];
  shortDescription: string;
  title: string;
}

export const FLOW_NODE_PALETTE_DRAG_ID_PREFIX = "dataflow-palette:";

export function buildFlowNodePaletteDragId(className: string): string {
  return `${FLOW_NODE_PALETTE_DRAG_ID_PREFIX}${className}`;
}

export function isFlowNodePaletteDragId(value: string | number): boolean {
  return (
    typeof value === "string" &&
    value.startsWith(FLOW_NODE_PALETTE_DRAG_ID_PREFIX)
  );
}

export function createFlowNodeDragPayload(
  nodeDefinition: WireFlowNodeDefinition,
  dragOffset?: XYPosition,
): FlowNodeDragPayload {
  const parameterValues = buildDefaultParameterValues(nodeDefinition);
  const resolvedData = buildResolvedNodeData({
    definition: nodeDefinition,
    instanceName: nodeDefinition.class_name,
    parameterValues,
  });

  return {
    actionPorts: resolvedData.actionPorts,
    automationPorts: resolvedData.automationPorts,
    definitionName: nodeDefinition.class_name,
    dragOffset,
    hasController: resolvedData.hasController === true,
    inputPorts: resolvedData.inputPorts,
    longDescription: nodeDefinition.long_description,
    nodeClass: nodeDefinition.kind,
    outputPorts: resolvedData.outputPorts,
    parameterValues,
    signalPorts: resolvedData.signalPorts,
    shortDescription: nodeDefinition.short_description,
    title: nodeDefinition.class_name,
  };
}

export function serialiseFlowNodeDragPayload(
  payload: FlowNodeDragPayload,
): string {
  return JSON.stringify(payload);
}

export function parseFlowNodeDragPayload(
  rawPayload: string,
): FlowNodeDragPayload | null {
  try {
    const parsed = JSON.parse(rawPayload) as Partial<FlowNodeDragPayload>;

    if (
      (parsed.dragOffset !== undefined &&
        (typeof parsed.dragOffset?.x !== "number" ||
          typeof parsed.dragOffset?.y !== "number")) ||
      typeof parsed.definitionName !== "string" ||
      typeof parsed.title !== "string" ||
      typeof parsed.shortDescription !== "string" ||
      typeof parsed.longDescription !== "string" ||
      typeof parsed.hasController !== "boolean" ||
      !Array.isArray(parsed.actionPorts) ||
      !parsed.actionPorts.every(isFlowStudioResolvedPort) ||
      !Array.isArray(parsed.inputPorts) ||
      !parsed.inputPorts.every(isFlowStudioResolvedPort) ||
      !Array.isArray(parsed.outputPorts) ||
      !parsed.outputPorts.every(isFlowStudioResolvedPort) ||
      !Array.isArray(parsed.signalPorts) ||
      !parsed.signalPorts.every(isFlowStudioResolvedPort) ||
      typeof parsed.parameterValues !== "object" ||
      parsed.parameterValues === null ||
      Array.isArray(parsed.parameterValues) ||
      (parsed.nodeClass !== "source" &&
        parsed.nodeClass !== "control" &&
        parsed.nodeClass !== "instrument" &&
        parsed.nodeClass !== "logic" &&
        parsed.nodeClass !== "hybrid" &&
        parsed.nodeClass !== "inline" &&
        parsed.nodeClass !== "sink")
    ) {
      return null;
    }

    return {
      actionPorts: parsed.actionPorts,
      automationPorts: parsed.automationPorts ?? [],
      definitionName: parsed.definitionName,
      dragOffset: parsed.dragOffset,
      hasController: parsed.hasController,
      inputPorts: parsed.inputPorts,
      longDescription: parsed.longDescription,
      nodeClass: parsed.nodeClass,
      outputPorts: parsed.outputPorts,
      parameterValues: parsed.parameterValues as Record<string, unknown>,
      signalPorts: parsed.signalPorts,
      shortDescription: parsed.shortDescription,
      title: parsed.title,
    };
  } catch {
    return null;
  }
}

export function createDroppedFlowNode(params: {
  nextIndex: number;
  payload: FlowNodeDragPayload;
  position: XYPosition;
}): Node<FlowStudioNodeData> {
  const { nextIndex, payload, position } = params;
  const instanceName = buildDefaultInstanceName(payload.title, nextIndex);

  return {
    id: buildDefaultFlowNodeId(payload.title, nextIndex),
    className: `flow-studio-node flow-studio-node--${payload.nodeClass}`,
    data: {
      actionPorts: payload.actionPorts,
      automationPorts: payload.automationPorts,
      controllerPortPlacement: "default",
      definitionName: payload.definitionName,
      hasController: payload.hasController,
      inputPorts: payload.inputPorts,
      instanceName,
      longDescription: payload.longDescription,
      nodeClass: payload.nodeClass,
      parameterValues: payload.parameterValues,
      portParameterValues: undefined,
      portOrder: undefined,
      outputPorts: payload.outputPorts,
      portNames: undefined,
      signalPorts: payload.signalPorts,
      shortDescription: payload.shortDescription,
    },
    position,
    type: "flowStudio",
  };
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
    typeof candidate.side === "string" &&
    (candidate.side === "action" ||
      candidate.side === "input" ||
      candidate.side === "output" ||
      candidate.side === "signal") &&
    (candidate.displayClass === undefined ||
      candidate.displayClass === "source" ||
      candidate.displayClass === "inline" ||
      candidate.displayClass === "sink") &&
    (candidate.name === undefined || typeof candidate.name === "string") &&
    (candidate.shortDescription === undefined ||
      typeof candidate.shortDescription === "string") &&
    (candidate.longDescription === undefined ||
      typeof candidate.longDescription === "string") &&
    (candidate.acceptedTokenTypes === undefined ||
      (Array.isArray(candidate.acceptedTokenTypes) &&
        candidate.acceptedTokenTypes.every(
          (value) => typeof value === "string",
        ))) &&
    (candidate.tokenType === undefined ||
      typeof candidate.tokenType === "string")
  );
}

export function resolveDroppedFlowNodePosition(params: {
  cursorPosition: XYPosition;
  payload: FlowNodeDragPayload;
}): XYPosition {
  const { cursorPosition, payload } = params;

  if (!payload.dragOffset) {
    return cursorPosition;
  }

  return {
    x: cursorPosition.x - payload.dragOffset.x,
    y: cursorPosition.y - payload.dragOffset.y,
  };
}

export const DATAFLOW_CANVAS_DROPZONE_ID = "dataflow-canvas-dropzone";
