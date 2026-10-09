/**
 * Copyright (c) Scott A Dixon
 *
 * Manages node and edge mutations for the editor's dataflow canvas.
 */
import React from "react";
import {
  applyEdgeChanges,
  applyNodeChanges,
  type Connection,
  type EdgeChange,
  type FinalConnectionState,
  type NodeChange,
} from "@xyflow/react";

import type {
  FlowStudioEdge,
  FlowStudioNode,
} from "../core/dataflow-editor-state.js";
import type { FlowInteractionPorts } from "../interaction-ports.js";
import {
  canAttachConnectionToPorts,
  connectionConflictsWithExistingEdge,
  connectionUsesCompatibleTokenTypes,
  resolveFlowConnectionKind,
  resolveConnectionFromHandlePair,
} from "../core/flow-node-ports.js";
import type { FlowStudioWorkspaceState } from "../core/flow-persistence.js";
import type { DataflowStructuralChangeOptions } from "./use-dataflow-structural-undo.js";

export interface FlowConnectionAppendResult {
  edges: FlowStudioEdge[];
  status: "duplicate-port" | "incompatible-token" | "invalid" | "updated";
}

export function hasStructuralEdgeRemovalChange(
  changes: readonly EdgeChange<FlowStudioEdge>[],
): boolean {
  return changes.some((change) => change.type === "remove");
}

export function resolveConnectionAppendResult(options: {
  connection: Connection;
  edges: readonly FlowStudioEdge[];
  nodes?: readonly FlowStudioNode[];
}): FlowConnectionAppendResult {
  if (!options.connection.source || !options.connection.target) {
    return {
      edges: [...options.edges],
      status: "invalid",
    };
  }

  if (
    options.nodes &&
    resolveFlowConnectionKind(options.connection) === "token" &&
    !connectionUsesCompatibleTokenTypes({
      connection: options.connection,
      nodes: options.nodes,
    })
  ) {
    return {
      edges: [...options.edges],
      status: "incompatible-token",
    };
  }

  if (
    connectionConflictsWithExistingEdge([...options.edges], options.connection)
  ) {
    return {
      edges: [...options.edges],
      status: "duplicate-port",
    };
  }

  const connectionKind = resolveFlowConnectionKind(options.connection);
  if (connectionKind === null) {
    return {
      edges: [...options.edges],
      status: "invalid",
    };
  }

  return {
    edges: [
      ...options.edges,
      {
        id: `${options.connection.source}-${options.connection.sourceHandle ?? "output-0"}-${options.connection.target}-${options.connection.targetHandle ?? "input-0"}-${options.edges.length + 1}`,
        source: options.connection.source,
        sourceHandle: options.connection.sourceHandle ?? "output-0",
        target: options.connection.target,
        targetHandle: options.connection.targetHandle ?? "input-0",
        data: {
          kind: connectionKind,
          order: options.edges.length + 1,
        },
      },
    ],
    status: "updated",
  };
}

function notifyPortAlreadyConnected(
  notify: FlowInteractionPorts["notify"],
): void {
  notify({
    id: "flow-port-already-connected",
    tone: "warning",
    title: "Port already connected",
    message:
      "Inputs and signal ports accept one edge. Remove the existing edge first.",
  });
}

export function useDataflowConnections(options: {
  ports: FlowInteractionPorts;
  commitStructuralChange: (
    label: string,
    transform: (
      workspace: FlowStudioWorkspaceState,
    ) => FlowStudioWorkspaceState,
    options?: DataflowStructuralChangeOptions,
  ) => void;
  edges: readonly FlowStudioEdge[];
  nodes: readonly FlowStudioNode[];
  setWorkspace: React.Dispatch<React.SetStateAction<FlowStudioWorkspaceState>>;
}) {
  const handleNodesChange = React.useCallback(
    (changes: NodeChange<FlowStudioNode>[]) => {
      options.setWorkspace((current) => ({
        ...current,
        nodes: applyNodeChanges(changes, current.nodes),
      }));
    },
    [options.setWorkspace],
  );

  const handleEdgesChange = React.useCallback(
    (changes: EdgeChange<FlowStudioEdge>[]) => {
      if (hasStructuralEdgeRemovalChange(changes)) {
        options.commitStructuralChange("Remove connection", (current) => ({
          ...current,
          edges: applyEdgeChanges(changes, current.edges),
        }));
        return;
      }

      options.setWorkspace((current) => ({
        ...current,
        edges: applyEdgeChanges(changes, current.edges),
      }));
    },
    [options.commitStructuralChange, options.setWorkspace, options.ports],
  );

  const handleConnect = React.useCallback(
    (connection: Connection) => {
      options.commitStructuralChange("Add connection", (current) => {
        const result = resolveConnectionAppendResult({
          connection,
          edges: current.edges,
          nodes: current.nodes,
        });
        if (result.status === "duplicate-port") {
          notifyPortAlreadyConnected(options.ports.notify);
        }

        return result.status === "updated"
          ? {
              ...current,
              edges: result.edges,
            }
          : current;
      });
    },
    [options.commitStructuralChange],
  );

  const isConnectionValid = React.useCallback(
    (connection: Connection | FlowStudioEdge) =>
      canAttachConnectionToPorts([...options.edges], connection, options.nodes),
    [options.edges, options.nodes],
  );

  const handleConnectEnd = React.useCallback(
    (_: MouseEvent | TouchEvent, connectionState: FinalConnectionState) => {
      if (connectionState.isValid !== false) {
        return;
      }

      const attemptedConnection = resolveConnectionFromHandlePair({
        fromHandle: connectionState.fromHandle,
        toHandle: connectionState.toHandle,
      });
      if (!attemptedConnection) {
        return;
      }

      if (
        connectionConflictsWithExistingEdge(
          [...options.edges],
          attemptedConnection,
        )
      ) {
        notifyPortAlreadyConnected(options.ports.notify);
      }
    },
    [options.edges, options.nodes],
  );

  return {
    handleConnect,
    handleConnectEnd,
    handleEdgesChange,
    handleNodesChange,
    isConnectionValid,
  };
}
