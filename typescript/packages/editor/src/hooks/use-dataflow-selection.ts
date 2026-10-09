/**
 * Copyright (c) Scott A Dixon
 *
 * Manages selection, node context menu, and global keyboard behaviour for the editor's dataflow canvas.
 */
import React from "react";
import type { EdgeChange } from "@xyflow/react";

import type { FlowInteractionPorts } from "../interaction-ports.js";

import type {
  FlowStudioEdge,
  FlowStudioSelectedEdgeBridge,
  FlowStudioNode,
  FlowStudioSelectedEdgeWaypoint,
  FlowStudioSelectionTarget,
  NodeContextMenuState,
} from "../core/dataflow-editor-state.js";
import type { FlowStudioWorkspaceState } from "../core/flow-persistence.js";
import type { FlowPortSide } from "../core/flow-node-ports.js";
import type { FlowPortMoveDirection } from "../core/flow-port-move-controls.js";

export type SelectionKeyboardAction =
  | "clear-selection"
  | "ignore"
  | "move-port-toward-end"
  | "move-port-toward-start"
  | "remove-bridge-selection"
  | "remove-node-selection"
  | "remove-waypoint-selection";

export function resolveSelectionKeyboardAction(options: {
  hasNodeContextMenu: boolean;
  isEditableTarget: boolean;
  key: string;
  repeat: boolean;
  selectedEdgeBridge?: FlowStudioSelectedEdgeBridge | null;
  selectedEdgeWaypoint?: FlowStudioSelectedEdgeWaypoint | null;
  selectedTarget?: FlowStudioSelectionTarget;
}): SelectionKeyboardAction {
  const selectedTarget = options.selectedTarget ?? { kind: "none" };
  const hasSelection =
    selectedTarget.kind !== "none" ||
    options.selectedEdgeBridge !== null ||
    options.selectedEdgeWaypoint !== null;
  if (options.repeat || options.isEditableTarget) {
    return "ignore";
  }

  if (options.key === "Escape") {
    return hasSelection || options.hasNodeContextMenu
      ? "clear-selection"
      : "ignore";
  }

  if (options.key === "Delete" || options.key === "Backspace") {
    if (options.selectedEdgeWaypoint) {
      return "remove-waypoint-selection";
    }
    if (options.selectedEdgeBridge) {
      return "remove-bridge-selection";
    }

    return selectedTarget.kind === "node" ? "remove-node-selection" : "ignore";
  }

  if (selectedTarget.kind === "port") {
    if (options.key === "ArrowUp") {
      return "move-port-toward-start";
    }

    if (options.key === "ArrowDown") {
      return "move-port-toward-end";
    }
  }

  return "ignore";
}

export function shouldDismissNodeContextMenu(
  target: EventTarget | null,
): boolean {
  if (!target || typeof target !== "object" || !("closest" in target)) {
    return true;
  }

  return (
    typeof target.closest !== "function" ||
    !target.closest(".flow-studio-node-context-menu")
  );
}

export function shouldClearSelectionForEdgeChanges(
  changes: readonly EdgeChange<FlowStudioEdge>[],
): boolean {
  return changes.some((change) => change.type === "select" && change.selected);
}

export function selectFlowInWorkspace(
  workspace: FlowStudioWorkspaceState,
): FlowStudioWorkspaceState {
  return {
    ...workspace,
    selectedTarget: { kind: "flow" },
  };
}

export function useDataflowSelection(options: {
  ports: FlowInteractionPorts;
  clearSelectedEdgeBridge: () => void;
  clearSelectedEdgeWaypoint: () => void;
  moveSelectedPort: (direction: FlowPortMoveDirection) => void;
  removeNode: (nodeId: string) => void;
  removeSelectedEdgeBridge: () => void;
  removeSelectedEdgeWaypoint: () => void;
  selectedEdgeBridge: FlowStudioSelectedEdgeBridge | null;
  selectedEdgeWaypoint: FlowStudioSelectedEdgeWaypoint | null;
  selectedTarget: FlowStudioSelectionTarget;
  setWorkspace: React.Dispatch<React.SetStateAction<FlowStudioWorkspaceState>>;
}) {
  const [nodeContextMenu, setNodeContextMenu] =
    React.useState<NodeContextMenuState | null>(null);

  const clearSelection = React.useCallback(() => {
    options.setWorkspace((current) => ({
      ...current,
      selectedTarget: { kind: "none" },
    }));
    options.clearSelectedEdgeBridge();
    options.clearSelectedEdgeWaypoint();
    setNodeContextMenu(null);
  }, [
    options.clearSelectedEdgeBridge,
    options.clearSelectedEdgeWaypoint,
    options.setWorkspace,
  ]);

  const selectFlow = React.useCallback(() => {
    options.setWorkspace(selectFlowInWorkspace);
    options.clearSelectedEdgeBridge();
    options.clearSelectedEdgeWaypoint();
    setNodeContextMenu(null);
  }, [
    options.clearSelectedEdgeBridge,
    options.clearSelectedEdgeWaypoint,
    options.setWorkspace,
  ]);

  React.useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (options.ports.isKeyboardBlocked(event)) {
        return;
      }

      const target = event.target;
      const isPortSelectionTarget =
        target instanceof HTMLElement &&
        target.closest(".flow-studio-node-port-select") !== null;
      const isEditableTarget =
        target instanceof HTMLElement &&
        (target.isContentEditable ||
          target instanceof HTMLInputElement ||
          target instanceof HTMLTextAreaElement ||
          target instanceof HTMLSelectElement);
      if (
        isPortSelectionTarget &&
        options.selectedTarget.kind === "port" &&
        (event.key === "ArrowUp" || event.key === "ArrowDown")
      ) {
        return;
      }
      const action = resolveSelectionKeyboardAction({
        hasNodeContextMenu: nodeContextMenu !== null,
        isEditableTarget,
        key: event.key,
        repeat: event.repeat,
        selectedEdgeBridge: options.selectedEdgeBridge,
        selectedEdgeWaypoint: options.selectedEdgeWaypoint,
        selectedTarget: options.selectedTarget,
      });

      if (action === "clear-selection") {
        event.preventDefault();
        clearSelection();
        return;
      }

      if (action === "remove-node-selection") {
        event.preventDefault();
        options.removeNode(
          options.selectedTarget.kind === "node"
            ? options.selectedTarget.nodeId
            : "",
        );
        return;
      }

      if (action === "move-port-toward-start") {
        event.preventDefault();
        event.stopPropagation();
        event.stopImmediatePropagation?.();
        options.moveSelectedPort("toward-start");
        return;
      }

      if (action === "move-port-toward-end") {
        event.preventDefault();
        event.stopPropagation();
        event.stopImmediatePropagation?.();
        options.moveSelectedPort("toward-end");
        return;
      }

      if (action === "remove-waypoint-selection") {
        event.preventDefault();
        options.removeSelectedEdgeWaypoint();
        return;
      }

      if (action === "remove-bridge-selection") {
        event.preventDefault();
        options.removeSelectedEdgeBridge();
      }
    };

    window.addEventListener("keydown", handleKeyDown, true);
    return () => {
      window.removeEventListener("keydown", handleKeyDown, true);
    };
  }, [
    clearSelection,
    options.moveSelectedPort,
    nodeContextMenu,
    options.removeNode,
    options.removeSelectedEdgeBridge,
    options.removeSelectedEdgeWaypoint,
    options.selectedEdgeBridge,
    options.selectedEdgeWaypoint,
    options.selectedTarget,
  ]);

  React.useEffect(() => {
    if (!nodeContextMenu) {
      return;
    }

    const handlePointerDown = (event: PointerEvent) => {
      if (!shouldDismissNodeContextMenu(event.target)) {
        return;
      }

      setNodeContextMenu(null);
    };
    const handleWindowBlur = () => {
      setNodeContextMenu(null);
    };

    window.addEventListener("pointerdown", handlePointerDown);
    window.addEventListener("blur", handleWindowBlur);
    return () => {
      window.removeEventListener("pointerdown", handlePointerDown);
      window.removeEventListener("blur", handleWindowBlur);
    };
  }, [nodeContextMenu]);

  const handleNodeClick = React.useCallback(
    (_: React.MouseEvent, node: FlowStudioNode) => {
      options.setWorkspace((current) => ({
        ...current,
        selectedTarget: {
          kind: "node",
          nodeId: node.id,
        },
      }));
      options.clearSelectedEdgeBridge();
      options.clearSelectedEdgeWaypoint();
      setNodeContextMenu(null);
    },
    [
      options.clearSelectedEdgeBridge,
      options.clearSelectedEdgeWaypoint,
      options.setWorkspace,
    ],
  );

  const handleNodeContextMenu = React.useCallback(
    (event: React.MouseEvent, node: FlowStudioNode) => {
      event.preventDefault();
      event.stopPropagation();

      options.setWorkspace((current) => ({
        ...current,
        selectedTarget: {
          kind: "node",
          nodeId: node.id,
        },
      }));
      options.clearSelectedEdgeBridge();
      options.clearSelectedEdgeWaypoint();
      setNodeContextMenu({
        nodeId: node.id,
        x: event.clientX,
        y: event.clientY,
      });
    },
    [
      options.clearSelectedEdgeBridge,
      options.clearSelectedEdgeWaypoint,
      options.setWorkspace,
    ],
  );

  const handleDetailPaneClose = React.useCallback(() => {
    clearSelection();
  }, [clearSelection]);

  const handlePortSelect = React.useCallback(
    (optionsForPort: {
      nodeId: string;
      portId: string;
      side: FlowPortSide;
    }) => {
      options.setWorkspace((current) => ({
        ...current,
        selectedTarget: {
          kind: "port",
          nodeId: optionsForPort.nodeId,
          portId: optionsForPort.portId,
          side: optionsForPort.side,
        },
      }));
      options.clearSelectedEdgeBridge();
      options.clearSelectedEdgeWaypoint();
      setNodeContextMenu(null);
    },
    [
      options.clearSelectedEdgeBridge,
      options.clearSelectedEdgeWaypoint,
      options.setWorkspace,
    ],
  );

  return {
    clearSelection,
    handleDetailPaneClose,
    handleNodeClick,
    handleNodeContextMenu,
    handlePortSelect,
    nodeContextMenu,
    selectFlow,
    setNodeContextMenu,
  };
}
