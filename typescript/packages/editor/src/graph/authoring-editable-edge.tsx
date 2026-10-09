/**
 * Copyright (c) Scott A Dixon
 *
 * Defines the shared editable-edge contract used by authoring canvases.
 */
import React from "react";
import { type Edge, type EdgeProps } from "@xyflow/react";

import {
  AuthoringGraphEdge,
  eventTargetMatchesWaypointHoverSurface,
  stopWaypointSurfacePropagation,
  type AuthoringGraphEdgeTheme,
} from "./authoring-graph-edge.js";
import type {
  AuthoringGraphBridge,
  AuthoringGraphPoint,
  AuthoringGraphWaypoint,
} from "./authoring-graph-edge-types.js";
import type { AuthoringGraphInteractiveGestureDriver } from "./authoring-graph-gestures.js";

export {
  eventTargetMatchesWaypointHoverSurface,
  stopWaypointSurfacePropagation,
};

export type AuthoringEditableEdgeTheme = AuthoringGraphEdgeTheme;

export interface AuthoringEditableEdgeDataShape
  extends Record<string, unknown> {
  addBridgeLabel?: string;
  addWaypointLabel?: string;
  bridges?: AuthoringGraphBridge[];
  curveOffsetPx?: number;
  directionMarker?: boolean;
  directionMarkerPlacement?: "target-boundary" | "visible-interval";
  directionMarkerTargetBorderRadiusPx?: number;
  gestureDriver?: AuthoringGraphInteractiveGestureDriver;
  onInsertBridge?: (segmentIndex: number, bridge: AuthoringGraphBridge) => void;
  onInsertWaypoint?: (
    segmentIndex: number,
    segmentT: number,
    waypoint: AuthoringGraphWaypoint,
  ) => void;
  onSelectBridge?: (bridgeIndex: number) => void;
  onSelectWaypoint?: (waypointIndex: number) => void;
  onUpdateBridgeGap?: (bridgeIndex: number, gap: number) => void;
  onUpdateBridgePosition?: (
    bridgeIndex: number,
    segmentIndex: number,
    t: number,
  ) => void;
  onUpdateWaypointHandle?: (
    waypointIndex: number,
    handleKind: "inHandle" | "outHandle",
    independent: boolean,
    position: AuthoringGraphPoint,
  ) => void;
  onUpdateWaypointPosition?: (
    waypointIndex: number,
    position: AuthoringGraphPoint,
  ) => void;
  selectedBridgeIndex?: number;
  selectedWaypointIndex?: number;
  showSecondaryPath?: boolean;
  theme?: AuthoringEditableEdgeTheme;
  waypoints?: AuthoringGraphWaypoint[];
}

export type AuthoringEditableEdgeData<
  TExtra extends Record<string, unknown> = Record<string, never>,
> = TExtra & AuthoringEditableEdgeDataShape;

export function AuthoringEditableEdgeView(
  props: EdgeProps<Edge<AuthoringEditableEdgeData>>,
): React.JSX.Element {
  const data = props.data;

  return (
    <AuthoringGraphEdge
      {...props}
      addBridgeLabel={data?.addBridgeLabel}
      addWaypointLabel={data?.addWaypointLabel}
      directionMarker={data?.directionMarker}
      directionMarkerPlacement={data?.directionMarkerPlacement}
      directionMarkerTargetBorderRadiusPx={
        data?.directionMarkerTargetBorderRadiusPx
      }
      gestureDriver={data?.gestureDriver}
      layout={{
        bridges: data?.bridges,
        waypoints: data?.waypoints,
      }}
      curveOffsetPx={data?.curveOffsetPx}
      onInsertBridge={data?.onInsertBridge}
      onInsertWaypoint={data?.onInsertWaypoint}
      onSelectBridge={data?.onSelectBridge}
      onSelectWaypoint={data?.onSelectWaypoint}
      onUpdateBridgeGap={data?.onUpdateBridgeGap}
      onUpdateBridgePosition={data?.onUpdateBridgePosition}
      onUpdateWaypointHandle={data?.onUpdateWaypointHandle}
      onUpdateWaypointPosition={data?.onUpdateWaypointPosition}
      selection={{
        bridgeIndex: data?.selectedBridgeIndex,
        waypointIndex: data?.selectedWaypointIndex,
      }}
      showSecondaryPath={data?.showSecondaryPath}
      theme={data?.theme}
    />
  );
}
