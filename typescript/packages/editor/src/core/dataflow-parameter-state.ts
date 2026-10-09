import { resolveNumberControlValue } from "./parameter-controls.js";
/** Copyright (c) Scott A Dixon */
import type {
  FlowNode as WireFlowNode,
  FlowNodeDefinition as WireFlowNodeDefinition,
  FlowParameterDefinition as WireFlowParameterDefinition,
} from "@battersea/flow";
import {
  buildResolvedNodeData,
  cloneFlowPortParameterValues,
  type FlowParameterValues,
} from "./flow-node-definitions.js";
import type { FlowPortSide } from "./flow-node-ports.js";
import {
  pruneEdgesForNodeCardinality,
  resolveFlowStudioResolvedPorts,
} from "./flow-node-ports.js";
import { remapNodeSideEdgeHandles } from "./flow-port-order.js";
import type { FlowStudioWorkspaceState } from "./flow-persistence.js";

export function updateWorkspaceNodeParameterValue(options: {
  definition: WireFlowNodeDefinition;
  nodeId: string;
  parameter: WireFlowParameterDefinition;
  remove?: boolean;
  value: unknown;
  workspace: FlowStudioWorkspaceState;
}): FlowStudioWorkspaceState {
  let nextInputCount = 0;
  let nextOutputCount = 0;
  let nextActionCount = 0;
  let nextSignalCount = 0;
  let previousActionPorts: ReturnType<typeof resolveFlowStudioResolvedPorts> =
    [];
  let previousInputPorts: ReturnType<typeof resolveFlowStudioResolvedPorts> =
    [];
  let previousOutputPorts: ReturnType<typeof resolveFlowStudioResolvedPorts> =
    [];
  let previousSignalPorts: ReturnType<typeof resolveFlowStudioResolvedPorts> =
    [];
  let nextActionPorts: ReturnType<typeof resolveFlowStudioResolvedPorts> = [];
  let nextInputPorts: ReturnType<typeof resolveFlowStudioResolvedPorts> = [];
  let nextOutputPorts: ReturnType<typeof resolveFlowStudioResolvedPorts> = [];
  let nextSignalPorts: ReturnType<typeof resolveFlowStudioResolvedPorts> = [];

  const nextNodes = options.workspace.nodes.map((node) => {
    if (node.id !== options.nodeId) {
      return node;
    }

    previousActionPorts = resolveFlowStudioResolvedPorts(
      node.data.actionPorts,
      "action",
      node.data.nodeClass,
    );
    previousInputPorts = resolveFlowStudioResolvedPorts(
      node.data.inputPorts,
      "input",
      node.data.nodeClass,
    );
    previousOutputPorts = resolveFlowStudioResolvedPorts(
      node.data.outputPorts,
      "output",
      node.data.nodeClass,
    );
    previousSignalPorts = resolveFlowStudioResolvedPorts(
      node.data.signalPorts,
      "signal",
      node.data.nodeClass,
    );

    const rawValue =
      options.parameter.editor.kind === "unsigned" ||
      options.parameter.editor.kind === "input_port_count" ||
      options.parameter.editor.kind === "output_port_count"
        ? resolveNumberControlValue(
            options.value,
            Number(options.parameter.editor.min ?? 0),
          )
        : options.value;
    const boundedValue =
      typeof rawValue === "number" && Number.isFinite(rawValue)
        ? Math.max(
            Number(options.parameter.editor.min ?? 0),
            options.parameter.editor.max === null ||
              options.parameter.editor.max === undefined
              ? rawValue
              : Math.min(Number(options.parameter.editor.max), rawValue),
          )
        : rawValue;
    const nextParameterValues: FlowParameterValues = {
      ...node.data.parameterValues,
    };
    if (options.remove) {
      delete nextParameterValues[options.parameter.name];
    } else {
      nextParameterValues[options.parameter.name] = boundedValue;
    }

    const nextData = buildResolvedNodeData({
      controllerPortPlacement: node.data.controllerPortPlacement,
      definition: options.definition,
      instanceName: node.data.instanceName,
      parameterValues: nextParameterValues,
      portOrder: node.data.portOrder,
      portNames: node.data.portNames,
    });
    nextActionCount = nextData.actionPorts.length;
    nextInputCount = nextData.inputPorts.length;
    nextOutputCount = nextData.outputPorts.length;
    nextSignalCount = nextData.signalPorts.length;
    nextActionPorts = resolveFlowStudioResolvedPorts(
      nextData.actionPorts,
      "action",
      nextData.nodeClass,
    );
    nextInputPorts = resolveFlowStudioResolvedPorts(
      nextData.inputPorts,
      "input",
      nextData.nodeClass,
    );
    nextOutputPorts = resolveFlowStudioResolvedPorts(
      nextData.outputPorts,
      "output",
      nextData.nodeClass,
    );
    nextSignalPorts = resolveFlowStudioResolvedPorts(
      nextData.signalPorts,
      "signal",
      nextData.nodeClass,
    );

    return {
      ...node,
      data: nextData,
    };
  });

  return {
    ...options.workspace,
    edges: remapNodeSideEdgeHandles({
      edges: remapNodeSideEdgeHandles({
        edges: remapNodeSideEdgeHandles({
          edges: remapNodeSideEdgeHandles({
            edges: pruneEdgesForNodeCardinality(
              pruneEdgesForNodeCardinality(
                pruneEdgesForNodeCardinality(
                  pruneEdgesForNodeCardinality(
                    options.workspace.edges,
                    options.nodeId,
                    "action",
                    nextActionCount,
                  ),
                  options.nodeId,
                  "input",
                  nextInputCount,
                ),
                options.nodeId,
                "output",
                nextOutputCount,
              ),
              options.nodeId,
              "signal",
              nextSignalCount,
            ),
            nextPorts: nextActionPorts,
            nodeId: options.nodeId,
            previousPorts: previousActionPorts,
            side: "action",
          }),
          nextPorts: nextInputPorts,
          nodeId: options.nodeId,
          previousPorts: previousInputPorts,
          side: "input",
        }),
        nextPorts: nextOutputPorts,
        nodeId: options.nodeId,
        previousPorts: previousOutputPorts,
        side: "output",
      }),
      nextPorts: nextSignalPorts,
      nodeId: options.nodeId,
      previousPorts: previousSignalPorts,
      side: "signal",
    }),
    nodes: nextNodes,
    selectedTarget: resolveNextSelectionTarget(options.workspace, nextNodes),
  };
}

export function updateWorkspaceNodePortParameterValue(options: {
  definition: WireFlowNodeDefinition;
  nodeId: string;
  parameter: WireFlowParameterDefinition;
  portId: string;
  remove?: boolean;
  side: Extract<FlowPortSide, "input" | "output">;
  value: unknown;
  workspace: FlowStudioWorkspaceState;
}): FlowStudioWorkspaceState {
  return {
    ...options.workspace,
    nodes: options.workspace.nodes.map((node) => {
      if (node.id !== options.nodeId) {
        return node;
      }

      const nextPortParameterValues: NonNullable<
        WireFlowNode["port_parameter_values"]
      > = cloneFlowPortParameterValues(node.data.portParameterValues) ?? {};
      const nextSideValues: NonNullable<
        WireFlowNode["port_parameter_values"]
      >[typeof options.side] = {
        ...(nextPortParameterValues[options.side] ?? {}),
      };
      const nextPortValues = {
        ...(nextSideValues[options.portId] ?? {}),
      };

      if (options.remove) {
        delete nextPortValues[options.parameter.name];
      } else {
        nextPortValues[options.parameter.name] = options.value;
      }

      if (Object.keys(nextPortValues).length > 0) {
        nextSideValues[options.portId] = nextPortValues;
      } else {
        delete nextSideValues[options.portId];
      }

      if (Object.keys(nextSideValues).length > 0) {
        nextPortParameterValues[options.side] = nextSideValues;
      } else {
        delete nextPortParameterValues[options.side];
      }

      return {
        ...node,
        data: buildResolvedNodeData({
          controllerPortPlacement: node.data.controllerPortPlacement,
          definition: options.definition,
          instanceName: node.data.instanceName,
          parameterValues: node.data.parameterValues,
          portParameterValues: nextPortParameterValues,
          portOrder: node.data.portOrder,
          portNames: node.data.portNames,
        }),
      };
    }),
  };
}

function resolveNextSelectionTarget(
  workspace: FlowStudioWorkspaceState,
  nodes: FlowStudioWorkspaceState["nodes"],
): FlowStudioWorkspaceState["selectedTarget"] {
  const selectedTarget = workspace.selectedTarget;
  if (selectedTarget.kind !== "port") {
    return selectedTarget;
  }

  const node = nodes.find(
    (candidate) => candidate.id === selectedTarget.nodeId,
  );
  if (!node) {
    return { kind: "none" };
  }

  const ports =
    selectedTarget.side === "action"
      ? resolveFlowStudioResolvedPorts(node.data.actionPorts, "action")
      : selectedTarget.side === "input"
        ? resolveFlowStudioResolvedPorts(node.data.inputPorts, "input")
        : selectedTarget.side === "output"
          ? resolveFlowStudioResolvedPorts(node.data.outputPorts, "output")
          : resolveFlowStudioResolvedPorts(node.data.signalPorts, "signal");
  return ports.some((port) => port.id === selectedTarget.portId)
    ? selectedTarget
    : { kind: "none" };
}
