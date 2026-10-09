/**
 * Copyright (c) Scott A Dixon
 *
 * Composes focused dataflow canvas hooks for the editor's dataflow workspace.
 */
import React from "react";
import type { EdgeChange } from "@xyflow/react";
import { useAuthoringEditableEdgeController } from "../graph.js";
import { createAuthoringInteractiveGestureDriver } from "../graph.js";

import type {
  FlowStudioEdge,
  FlowStudioNode,
  FlowStudioEdgeBridge,
  FlowStudioEdgeWaypoint,
} from "../core/dataflow-editor-state.js";
import { clearNativeDataflowEdgeSelection } from "../core/dataflow-edge-interactions.js";
import {
  insertBridgeIntoEdge,
  moveBridgeInEdge,
  removeBridgeFromEdge,
  resizeBridgeInEdge,
} from "../core/flow-edge-bridges.js";
import {
  insertWaypointIntoEdge,
  moveWaypointHandleInEdge,
  moveWaypointInEdge,
  removeWaypointFromEdge,
} from "../core/flow-edge-waypoints.js";
import {
  resolveFlowPortMoveControlState,
  resolveFlowPortMoveTargetIndex,
  type FlowPortMoveDirection,
} from "../core/flow-port-move-controls.js";
import {
  reorderNodePortsInWorkspace,
  setNodeControllerPortPlacementInWorkspace,
  swapNodeControllerPortPlacementInWorkspace,
} from "../core/flow-port-reorder.js";
import {
  normalizeFlowControllerPortPlacement,
  type FlowControllerPortPlacement,
} from "../core/flow-controller-port-placement.js";
import { removeNodeFromWorkspace } from "../core/flow-persistence.js";

import { useDataflowCanvasDrop } from "./use-dataflow-canvas-drop.js";
import { useDataflowConnections } from "./use-dataflow-connections.js";
import {
  shouldClearSelectionForEdgeChanges,
  useDataflowSelection,
} from "./use-dataflow-selection.js";
import {
  resolveFlowStudioResolvedPorts,
  type FlowPortSide,
  type FlowStudioResolvedPort,
} from "../core/flow-node-ports.js";

import type { FlowNodeDefinition } from "@battersea/flow";
import type { FlowInteractionPorts } from "../interaction-ports.js";
import type { DataflowStructuralUndoState } from "./use-dataflow-structural-undo.js";
import type { FlowStudioWorkspaceState } from "../core/flow-persistence.js";
export interface FlowCanvasEditingOptions
  extends Pick<
    DataflowStructuralUndoState,
    | "captureSnapshot"
    | "commitLayoutChange"
    | "commitStructuralChange"
    | "pushPresentLayoutChange"
    | "undoSelectionResetToken"
  > {
  ports: FlowInteractionPorts;
  workspace: FlowStudioWorkspaceState;
  setWorkspace: React.Dispatch<React.SetStateAction<FlowStudioWorkspaceState>>;
  nodeDefinitionLookup: Record<string, FlowNodeDefinition>;
  selectedNode: FlowStudioNode | null;
  selectedNodeDefinition: FlowNodeDefinition | null;
  selectedPort: FlowStudioResolvedPort | null;
}

export function useFlowCanvasEditing(options: FlowCanvasEditingOptions) {
  const nodeDragSnapshotRef = React.useRef<ReturnType<
    typeof options.captureSnapshot
  > | null>(null);
  const [movedPortPulse, setMovedPortPulse] = React.useState<{
    nodeId: string;
    portId: string;
    replay: "a" | "b";
    side: FlowPortSide;
  } | null>(null);
  const removeNode = React.useCallback(
    (nodeId: string) => {
      options.commitStructuralChange("Remove node", (current) =>
        removeNodeFromWorkspace(current, nodeId),
      );
    },
    [options.commitStructuralChange],
  );
  const handleMovePort = React.useCallback(
    (target: {
      direction: FlowPortMoveDirection;
      nodeId: string;
      portId: string;
      side: FlowPortSide;
    }) => {
      const node = options.workspace.nodes.find(
        (candidate) => candidate.id === target.nodeId,
      );
      if (!node) {
        return;
      }

      const definition = options.nodeDefinitionLookup[node.data.definitionName];
      if (!definition) {
        return;
      }

      const currentPorts = resolveNodePortsForSide(node, target.side);
      const moveState = resolveFlowPortMoveControlState({
        controllerPortPlacement: node.data.controllerPortPlacement,
        nodeClass: node.data.nodeClass,
        portId: target.portId,
        ports: currentPorts,
        side: target.side,
      });
      if (!moveState) {
        return;
      }

      if (moveState.kind === "swap-side") {
        if (moveState.direction !== target.direction) {
          return;
        }

        options.commitLayoutChange("Reorder ports", (current) =>
          swapNodeControllerPortPlacementInWorkspace({
            definition,
            nodeId: target.nodeId,
            workspace: current,
          }),
        );
        setMovedPortPulse((current) => ({
          nodeId: target.nodeId,
          portId: target.portId,
          replay: current?.replay === "a" ? "b" : "a",
          side: target.side,
        }));
        return;
      }

      const targetIndex = resolveFlowPortMoveTargetIndex({
        currentIndex: moveState.currentIndex,
        direction: target.direction,
        portCount: currentPorts.length,
      });
      if (targetIndex === null) {
        return;
      }

      const nextWorkspace = reorderNodePortsInWorkspace({
        activePortId: target.portId,
        definition,
        nodeId: target.nodeId,
        previewIndex: targetIndex,
        side: target.side,
        workspace: options.workspace,
      });
      if (nextWorkspace === options.workspace) {
        return;
      }

      options.commitStructuralChange("Reorder ports", (current) =>
        reorderNodePortsInWorkspace({
          activePortId: target.portId,
          definition,
          nodeId: target.nodeId,
          previewIndex: targetIndex,
          side: target.side,
          workspace: current,
        }),
      );
      setMovedPortPulse((current) => ({
        nodeId: target.nodeId,
        portId: target.portId,
        replay: current?.replay === "a" ? "b" : "a",
        side: target.side,
      }));
    },
    [
      options.commitLayoutChange,
      options.commitStructuralChange,
      options.nodeDefinitionLookup,
      options.workspace,
    ],
  );
  const clearDataflowHostSelection = React.useCallback(
    (workspace: typeof options.workspace) => {
      const clearedWorkspace = clearNativeDataflowEdgeSelection(workspace);
      return clearedWorkspace.selectedTarget.kind === "none"
        ? clearedWorkspace
        : {
            ...clearedWorkspace,
            selectedTarget: { kind: "none" } as const,
          };
    },
    [],
  );
  const edgeGestureDriver = React.useMemo(
    () =>
      createAuthoringInteractiveGestureDriver({
        capture: options.captureSnapshot,
        commit: (label, snapshot) => {
          options.pushPresentLayoutChange(label, snapshot);
        },
      }),
    [options.captureSnapshot, options.pushPresentLayoutChange],
  );
  const editableEdgeController = useAuthoringEditableEdgeController<
    typeof options.workspace,
    string,
    { bridgeIndex: number; edgeId: string },
    { edgeId: string; waypointIndex: number }
  >({
    clearHostSelection: clearDataflowHostSelection,
    commitChange: (label, transform) => {
      options.commitLayoutChange(label, transform);
    },
    createBridgeSelection: (edgeId, bridgeIndex) => ({
      bridgeIndex,
      edgeId,
    }),
    createWaypointSelection: (edgeId, waypointIndex) => ({
      edgeId,
      waypointIndex,
    }),
    getBridgeCount: (workspace, edgeId) =>
      workspace.edges.find((edge) => edge.id === edgeId)?.data?.bridges
        ?.length ?? 0,
    hasBridge: (workspace, selection) =>
      Boolean(
        workspace.edges.find((edge) => edge.id === selection.edgeId)?.data
          ?.bridges?.[selection.bridgeIndex],
      ),
    hasWaypoint: (workspace, selection) =>
      Boolean(
        workspace.edges.find((edge) => edge.id === selection.edgeId)?.data
          ?.waypoints?.[selection.waypointIndex],
      ),
    insertBridge: (workspace, edgeId, _segmentIndex, bridge) => ({
      ...workspace,
      edges: workspace.edges.map((edge) =>
        edge.id === edgeId ? insertBridgeIntoEdge(edge, bridge) : edge,
      ),
    }),
    insertWaypoint: (workspace, edgeId, segmentIndex, segmentT, waypoint) => ({
      ...workspace,
      edges: workspace.edges.map((edge) =>
        edge.id === edgeId
          ? insertWaypointIntoEdge(edge, segmentIndex, waypoint, segmentT)
          : edge,
      ),
    }),
    removeBridge: (workspace, selection) => ({
      ...workspace,
      edges: workspace.edges.map((edge) =>
        edge.id === selection.edgeId
          ? removeBridgeFromEdge(edge, selection.bridgeIndex)
          : edge,
      ),
    }),
    removeWaypoint: (workspace, selection) => ({
      ...workspace,
      edges: workspace.edges.map((edge) =>
        edge.id === selection.edgeId
          ? removeWaypointFromEdge(edge, selection.waypointIndex)
          : edge,
      ),
    }),
    setWorkspace: options.setWorkspace,
    updateBridgeGap: (workspace, edgeId, bridgeIndex, gap) => ({
      ...workspace,
      edges: workspace.edges.map((edge) =>
        edge.id === edgeId ? resizeBridgeInEdge(edge, bridgeIndex, gap) : edge,
      ),
    }),
    updateBridgePosition: (
      workspace,
      edgeId,
      bridgeIndex,
      segmentIndex,
      t,
    ) => ({
      ...workspace,
      edges: workspace.edges.map((edge) =>
        edge.id === edgeId
          ? moveBridgeInEdge(edge, bridgeIndex, { segmentIndex, t })
          : edge,
      ),
    }),
    updateWaypointHandle: (
      workspace,
      edgeId,
      waypointIndex,
      handleKind,
      independent,
      position,
    ) => ({
      ...workspace,
      edges: workspace.edges.map((edge) =>
        edge.id === edgeId
          ? moveWaypointHandleInEdge(
              edge,
              waypointIndex,
              handleKind,
              independent,
              position,
            )
          : edge,
      ),
    }),
    updateWaypointPosition: (workspace, edgeId, waypointIndex, position) => ({
      ...workspace,
      edges: workspace.edges.map((edge) =>
        edge.id === edgeId
          ? moveWaypointInEdge(edge, waypointIndex, position)
          : edge,
      ),
    }),
    workspace: options.workspace,
  });
  const moveSelectedPort = React.useCallback(
    (direction: FlowPortMoveDirection) => {
      if (options.workspace.selectedTarget.kind !== "port") {
        return;
      }

      handleMovePort({
        direction,
        nodeId: options.workspace.selectedTarget.nodeId,
        portId: options.workspace.selectedTarget.portId,
        side: options.workspace.selectedTarget.side,
      });
    },
    [handleMovePort, options.workspace.selectedTarget],
  );
  const handleLogicVisualDirectionChange = React.useCallback(
    (placement: FlowControllerPortPlacement) => {
      const node = options.selectedNode;
      const definition = options.selectedNodeDefinition;
      if (
        !node ||
        !definition ||
        node.data.nodeClass !== "logic" ||
        definition.kind !== "logic"
      ) {
        return;
      }

      const nextPlacement = normalizeFlowControllerPortPlacement(placement);
      if (
        nextPlacement ===
        normalizeFlowControllerPortPlacement(node.data.controllerPortPlacement)
      ) {
        return;
      }

      options.commitLayoutChange("Change logic visual direction", (current) =>
        setNodeControllerPortPlacementInWorkspace({
          definition,
          nodeId: node.id,
          placement: nextPlacement,
          workspace: current,
        }),
      );
    },
    [
      options.commitLayoutChange,
      options.selectedNode,
      options.selectedNodeDefinition,
    ],
  );

  const selection = useDataflowSelection({
    ports: options.ports,
    clearSelectedEdgeBridge: editableEdgeController.clearSelectedEdgeBridge,
    clearSelectedEdgeWaypoint: editableEdgeController.clearSelectedEdgeWaypoint,
    moveSelectedPort,
    removeNode,
    removeSelectedEdgeBridge: editableEdgeController.removeSelectedEdgeBridge,
    removeSelectedEdgeWaypoint:
      editableEdgeController.removeSelectedEdgeWaypoint,
    selectedEdgeBridge: editableEdgeController.selectedEdgeBridge,
    selectedEdgeWaypoint: editableEdgeController.selectedEdgeWaypoint,
    selectedTarget: options.workspace.selectedTarget,
    setWorkspace: options.setWorkspace,
  });
  const canvasDrop = useDataflowCanvasDrop({
    ports: options.ports,
    commitStructuralChange: options.commitStructuralChange,
    nodes: options.workspace.nodes,
    setWorkspace: options.setWorkspace,
  });
  const connections = useDataflowConnections({
    ports: options.ports,
    commitStructuralChange: options.commitStructuralChange,
    edges: options.workspace.edges,
    nodes: options.workspace.nodes,
    setWorkspace: options.setWorkspace,
  });
  const handleNodeDragStart = React.useCallback(() => {
    nodeDragSnapshotRef.current = options.captureSnapshot();
  }, [options.captureSnapshot]);
  const handleNodeDragStop = React.useCallback(() => {
    const previousSnapshot = nodeDragSnapshotRef.current;
    nodeDragSnapshotRef.current = null;
    if (!previousSnapshot) {
      return;
    }

    options.pushPresentLayoutChange("Move node", previousSnapshot);
  }, [options.pushPresentLayoutChange]);
  React.useEffect(() => {
    editableEdgeController.clearSelectedEdgeAffordances();
    selection.setNodeContextMenu(null);
  }, [
    editableEdgeController.clearSelectedEdgeAffordances,
    options.undoSelectionResetToken,
    selection.setNodeContextMenu,
  ]);
  const handleEdgesChange = React.useCallback(
    (changes: EdgeChange<FlowStudioEdge>[]) => {
      if (shouldClearSelectionForEdgeChanges(changes)) {
        selection.clearSelection();
      }

      connections.handleEdgesChange(changes);
    },
    [connections.handleEdgesChange, selection.clearSelection],
  );

  return {
    activeDragDefinition: canvasDrop.activeDragDefinition,
    animateRejectedDrop: canvasDrop.animateRejectedDrop,
    clearSelection: selection.clearSelection,
    flowInstanceRef: canvasDrop.flowInstanceRef,
    handleConnect: connections.handleConnect,
    handleConnectEnd: connections.handleConnectEnd,
    handleDetailPaneClose: selection.handleDetailPaneClose,
    edgeGestureDriver,
    handleEdgesChange,
    handleInsertEdgeBridge: editableEdgeController.handleInsertEdgeBridge,
    handleInsertEdgeWaypoint: editableEdgeController.handleInsertEdgeWaypoint,
    handleNodeClick: selection.handleNodeClick,
    handleNodeContextMenu: selection.handleNodeContextMenu,
    handleNodeDragStart,
    handleNodeDragStop,
    handleNodesChange: connections.handleNodesChange,
    handlePaneClick: selection.selectFlow,
    handlePortSelect: selection.handlePortSelect,
    handlePaletteDragCancel: canvasDrop.handlePaletteDragCancel,
    handlePaletteDragEnd: canvasDrop.handlePaletteDragEnd,
    handlePaletteDragStart: canvasDrop.handlePaletteDragStart,
    handleLogicVisualDirectionChange,
    handleMovePort,
    handleSelectEdgeBridge: editableEdgeController.handleSelectEdgeBridge,
    handleSelectEdgeWaypoint: editableEdgeController.handleSelectEdgeWaypoint,
    handleUpdateEdgeBridgeGap: editableEdgeController.handleUpdateEdgeBridgeGap,
    handleUpdateEdgeBridgePosition:
      editableEdgeController.handleUpdateEdgeBridgePosition,
    handleUpdateEdgeWaypointHandle:
      editableEdgeController.handleUpdateEdgeWaypointHandle,
    handleUpdateEdgeWaypointPosition:
      editableEdgeController.handleUpdateEdgeWaypointPosition,
    isConnectionValid: connections.isConnectionValid,
    movedPortPulse,
    nodeContextMenu: selection.nodeContextMenu,
    removeNode,
    selectedEdgeBridge: editableEdgeController.selectedEdgeBridge,
    selectedEdgeWaypoint: editableEdgeController.selectedEdgeWaypoint,
    selectedPort: options.selectedPort,
  };
}

function resolveNodePortsForSide(
  node: FlowStudioNode,
  side: FlowPortSide,
): FlowStudioResolvedPort[] {
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
