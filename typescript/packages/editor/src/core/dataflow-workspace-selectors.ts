import type { FlowNodeDefinition } from "@battersea/flow";

import type { FlowStudioWorkspaceState } from "./flow-persistence.js";
import type {
  FlowStudioNode,
  FlowStudioPortSelectionTarget,
} from "./dataflow-editor-state.js";
import {
  resolveFlowStudioResolvedPorts,
  type FlowStudioResolvedPort,
} from "./flow-node-ports.js";

export function findSelectedFlowStudioNode(
  workspace: FlowStudioWorkspaceState,
): FlowStudioNode | null {
  const selectedTarget = workspace.selectedTarget;
  if (selectedTarget.kind !== "node" && selectedTarget.kind !== "port") {
    return null;
  }

  return (
    workspace.nodes.find((node) => node.id === selectedTarget.nodeId) ?? null
  );
}

export function buildRenderedFlowStudioNodes(
  workspace: FlowStudioWorkspaceState,
): FlowStudioNode[] {
  const selectedNodeId =
    workspace.selectedTarget.kind === "node" ||
    workspace.selectedTarget.kind === "port"
      ? workspace.selectedTarget.nodeId
      : null;

  return workspace.nodes.map((node) => ({
    ...node,
    selected: node.id === selectedNodeId,
  }));
}

export function findSelectedFlowStudioPort(
  selectedNode: FlowStudioNode | null,
  selectedTarget: FlowStudioWorkspaceState["selectedTarget"],
): FlowStudioResolvedPort | null {
  if (!selectedNode || selectedTarget.kind !== "port") {
    return null;
  }

  return findNodePortByTarget(selectedNode, selectedTarget);
}

export function resolveSelectedNodeDefinition(
  selectedNode: FlowStudioNode | null,
  nodeDefinitionLookup: Record<string, FlowNodeDefinition>,
): FlowNodeDefinition | null {
  return selectedNode
    ? (nodeDefinitionLookup[selectedNode.data.definitionName] ?? null)
    : null;
}

function findNodePortByTarget(
  node: FlowStudioNode,
  target: FlowStudioPortSelectionTarget,
): FlowStudioResolvedPort | null {
  const ports =
    target.side === "action"
      ? resolveFlowStudioResolvedPorts(node.data.actionPorts, "action")
      : target.side === "automation"
        ? resolveFlowStudioResolvedPorts(
            node.data.automationPorts,
            "automation",
          )
        : target.side === "input"
          ? resolveFlowStudioResolvedPorts(node.data.inputPorts, "input")
          : target.side === "output"
            ? resolveFlowStudioResolvedPorts(node.data.outputPorts, "output")
            : resolveFlowStudioResolvedPorts(node.data.signalPorts, "signal");

  return ports.find((port) => port.id === target.portId) ?? null;
}
