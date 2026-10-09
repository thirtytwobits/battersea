/**
 * Copyright (c) Scott A Dixon
 *
 * Shares editable-edge controller state across authoring canvases.
 */
import React from "react";

import type {
  AuthoringGraphBridge,
  AuthoringGraphPoint,
  AuthoringGraphWaypoint,
} from "./authoring-graph-edge-types.js";

export interface UseAuthoringEditableEdgeControllerOptions<
  TWorkspace,
  TEdgeKey,
  TBridgeSelection,
  TWaypointSelection,
> {
  clearHostSelection?: (workspace: TWorkspace) => TWorkspace;
  commitChange: (
    label: string,
    transform: (workspace: TWorkspace) => TWorkspace,
  ) => void;
  createBridgeSelection: (
    edgeKey: TEdgeKey,
    bridgeIndex: number,
  ) => TBridgeSelection;
  createWaypointSelection: (
    edgeKey: TEdgeKey,
    waypointIndex: number,
  ) => TWaypointSelection;
  getBridgeCount: (workspace: TWorkspace, edgeKey: TEdgeKey) => number;
  hasBridge: (workspace: TWorkspace, selection: TBridgeSelection) => boolean;
  hasWaypoint: (
    workspace: TWorkspace,
    selection: TWaypointSelection,
  ) => boolean;
  insertBridge: (
    workspace: TWorkspace,
    edgeKey: TEdgeKey,
    segmentIndex: number,
    bridge: AuthoringGraphBridge,
  ) => TWorkspace;
  insertWaypoint: (
    workspace: TWorkspace,
    edgeKey: TEdgeKey,
    segmentIndex: number,
    segmentT: number,
    waypoint: AuthoringGraphWaypoint,
  ) => TWorkspace;
  onEdgeInteraction?: () => void;
  removeBridge: (
    workspace: TWorkspace,
    selection: TBridgeSelection,
  ) => TWorkspace;
  removeWaypoint: (
    workspace: TWorkspace,
    selection: TWaypointSelection,
  ) => TWorkspace;
  setWorkspace: React.Dispatch<React.SetStateAction<TWorkspace>>;
  updateBridgeGap: (
    workspace: TWorkspace,
    edgeKey: TEdgeKey,
    bridgeIndex: number,
    gap: number,
  ) => TWorkspace;
  updateBridgePosition: (
    workspace: TWorkspace,
    edgeKey: TEdgeKey,
    bridgeIndex: number,
    segmentIndex: number,
    t: number,
  ) => TWorkspace;
  updateWaypointHandle: (
    workspace: TWorkspace,
    edgeKey: TEdgeKey,
    waypointIndex: number,
    handleKind: "inHandle" | "outHandle",
    independent: boolean,
    position: AuthoringGraphPoint,
  ) => TWorkspace;
  updateWaypointPosition: (
    workspace: TWorkspace,
    edgeKey: TEdgeKey,
    waypointIndex: number,
    position: AuthoringGraphPoint,
  ) => TWorkspace;
  workspace: TWorkspace;
}

export interface AuthoringEditableEdgeControllerState<
  TEdgeKey,
  TBridgeSelection,
  TWaypointSelection,
> {
  clearSelectedEdgeBridge: () => void;
  clearSelectedEdgeAffordances: () => void;
  clearSelectedEdgeWaypoint: () => void;
  handleInsertEdgeBridge: (
    edgeKey: TEdgeKey,
    segmentIndex: number,
    bridge: AuthoringGraphBridge,
  ) => void;
  handleInsertEdgeWaypoint: (
    edgeKey: TEdgeKey,
    segmentIndex: number,
    segmentT: number,
    waypoint: AuthoringGraphWaypoint,
  ) => void;
  handleSelectEdgeBridge: (edgeKey: TEdgeKey, bridgeIndex: number) => void;
  handleSelectEdgeWaypoint: (edgeKey: TEdgeKey, waypointIndex: number) => void;
  handleUpdateEdgeBridgeGap: (
    edgeKey: TEdgeKey,
    bridgeIndex: number,
    gap: number,
  ) => void;
  handleUpdateEdgeBridgePosition: (
    edgeKey: TEdgeKey,
    bridgeIndex: number,
    segmentIndex: number,
    t: number,
  ) => void;
  handleUpdateEdgeWaypointHandle: (
    edgeKey: TEdgeKey,
    waypointIndex: number,
    handleKind: "inHandle" | "outHandle",
    independent: boolean,
    position: AuthoringGraphPoint,
  ) => void;
  handleUpdateEdgeWaypointPosition: (
    edgeKey: TEdgeKey,
    waypointIndex: number,
    position: AuthoringGraphPoint,
  ) => void;
  removeSelectedEdgeBridge: () => void;
  removeSelectedEdgeWaypoint: () => void;
  selectedEdgeBridge: TBridgeSelection | null;
  selectedEdgeWaypoint: TWaypointSelection | null;
}

function identityWorkspace<TWorkspace>(workspace: TWorkspace): TWorkspace {
  return workspace;
}

export function useAuthoringEditableEdgeController<
  TWorkspace,
  TEdgeKey,
  TBridgeSelection,
  TWaypointSelection,
>(
  options: UseAuthoringEditableEdgeControllerOptions<
    TWorkspace,
    TEdgeKey,
    TBridgeSelection,
    TWaypointSelection
  >,
): AuthoringEditableEdgeControllerState<
  TEdgeKey,
  TBridgeSelection,
  TWaypointSelection
> {
  const [selectedEdgeBridge, setSelectedEdgeBridge] =
    React.useState<TBridgeSelection | null>(null);
  const [selectedEdgeWaypoint, setSelectedEdgeWaypoint] =
    React.useState<TWaypointSelection | null>(null);
  const clearHostSelection =
    options.clearHostSelection ?? identityWorkspace<TWorkspace>;

  const clearSelectedEdgeAffordances = React.useCallback(() => {
    setSelectedEdgeBridge(null);
    setSelectedEdgeWaypoint(null);
  }, []);
  const clearSelectedEdgeBridge = React.useCallback(() => {
    setSelectedEdgeBridge(null);
  }, []);
  const clearSelectedEdgeWaypoint = React.useCallback(() => {
    setSelectedEdgeWaypoint(null);
  }, []);

  const handleSelectEdgeBridge = React.useCallback(
    (edgeKey: TEdgeKey, bridgeIndex: number) => {
      options.setWorkspace((currentWorkspace) =>
        clearHostSelection(currentWorkspace),
      );
      setSelectedEdgeWaypoint(null);
      setSelectedEdgeBridge(
        options.createBridgeSelection(edgeKey, bridgeIndex),
      );
      options.onEdgeInteraction?.();
    },
    [clearHostSelection, options],
  );

  const handleSelectEdgeWaypoint = React.useCallback(
    (edgeKey: TEdgeKey, waypointIndex: number) => {
      options.setWorkspace((currentWorkspace) =>
        clearHostSelection(currentWorkspace),
      );
      setSelectedEdgeBridge(null);
      setSelectedEdgeWaypoint(
        options.createWaypointSelection(edgeKey, waypointIndex),
      );
      options.onEdgeInteraction?.();
    },
    [clearHostSelection, options],
  );

  const handleInsertEdgeBridge = React.useCallback(
    (edgeKey: TEdgeKey, segmentIndex: number, bridge: AuthoringGraphBridge) => {
      options.commitChange("Add bridge", (currentWorkspace) =>
        options.insertBridge(
          clearHostSelection(currentWorkspace),
          edgeKey,
          segmentIndex,
          bridge,
        ),
      );
      setSelectedEdgeWaypoint(null);
      setSelectedEdgeBridge(
        options.createBridgeSelection(
          edgeKey,
          options.getBridgeCount(options.workspace, edgeKey),
        ),
      );
      options.onEdgeInteraction?.();
    },
    [clearHostSelection, options],
  );

  const handleInsertEdgeWaypoint = React.useCallback(
    (
      edgeKey: TEdgeKey,
      segmentIndex: number,
      segmentT: number,
      waypoint: AuthoringGraphWaypoint,
    ) => {
      options.commitChange("Add waypoint", (currentWorkspace) =>
        options.insertWaypoint(
          clearHostSelection(currentWorkspace),
          edgeKey,
          segmentIndex,
          segmentT,
          waypoint,
        ),
      );
      setSelectedEdgeBridge(null);
      setSelectedEdgeWaypoint(
        options.createWaypointSelection(edgeKey, segmentIndex),
      );
      options.onEdgeInteraction?.();
    },
    [clearHostSelection, options],
  );

  const handleUpdateEdgeBridgePosition = React.useCallback(
    (
      edgeKey: TEdgeKey,
      bridgeIndex: number,
      segmentIndex: number,
      t: number,
    ) => {
      options.setWorkspace((currentWorkspace) =>
        options.updateBridgePosition(
          currentWorkspace,
          edgeKey,
          bridgeIndex,
          segmentIndex,
          t,
        ),
      );
    },
    [options],
  );

  const handleUpdateEdgeBridgeGap = React.useCallback(
    (edgeKey: TEdgeKey, bridgeIndex: number, gap: number) => {
      options.setWorkspace((currentWorkspace) =>
        options.updateBridgeGap(currentWorkspace, edgeKey, bridgeIndex, gap),
      );
    },
    [options],
  );

  const handleUpdateEdgeWaypointPosition = React.useCallback(
    (
      edgeKey: TEdgeKey,
      waypointIndex: number,
      position: AuthoringGraphPoint,
    ) => {
      options.setWorkspace((currentWorkspace) =>
        options.updateWaypointPosition(
          currentWorkspace,
          edgeKey,
          waypointIndex,
          position,
        ),
      );
    },
    [options],
  );

  const handleUpdateEdgeWaypointHandle = React.useCallback(
    (
      edgeKey: TEdgeKey,
      waypointIndex: number,
      handleKind: "inHandle" | "outHandle",
      independent: boolean,
      position: AuthoringGraphPoint,
    ) => {
      options.setWorkspace((currentWorkspace) =>
        options.updateWaypointHandle(
          currentWorkspace,
          edgeKey,
          waypointIndex,
          handleKind,
          independent,
          position,
        ),
      );
    },
    [options],
  );

  const removeSelectedEdgeBridge = React.useCallback(() => {
    setSelectedEdgeBridge((currentSelection) => {
      if (!currentSelection) {
        return currentSelection;
      }

      options.commitChange("Remove bridge", (currentWorkspace) =>
        options.removeBridge(currentWorkspace, currentSelection),
      );
      return null;
    });
  }, [options]);

  const removeSelectedEdgeWaypoint = React.useCallback(() => {
    setSelectedEdgeWaypoint((currentSelection) => {
      if (!currentSelection) {
        return currentSelection;
      }

      options.commitChange("Remove waypoint", (currentWorkspace) =>
        options.removeWaypoint(currentWorkspace, currentSelection),
      );
      return null;
    });
  }, [options]);

  React.useEffect(() => {
    if (
      selectedEdgeBridge &&
      !options.hasBridge(options.workspace, selectedEdgeBridge)
    ) {
      setSelectedEdgeBridge(null);
    }
  }, [options, selectedEdgeBridge]);

  React.useEffect(() => {
    if (
      selectedEdgeWaypoint &&
      !options.hasWaypoint(options.workspace, selectedEdgeWaypoint)
    ) {
      setSelectedEdgeWaypoint(null);
    }
  }, [options, selectedEdgeWaypoint]);

  return {
    clearSelectedEdgeBridge,
    clearSelectedEdgeAffordances,
    clearSelectedEdgeWaypoint,
    handleInsertEdgeBridge,
    handleInsertEdgeWaypoint,
    handleSelectEdgeBridge,
    handleSelectEdgeWaypoint,
    handleUpdateEdgeBridgeGap,
    handleUpdateEdgeBridgePosition,
    handleUpdateEdgeWaypointHandle,
    handleUpdateEdgeWaypointPosition,
    removeSelectedEdgeBridge,
    removeSelectedEdgeWaypoint,
    selectedEdgeBridge,
    selectedEdgeWaypoint,
  };
}
