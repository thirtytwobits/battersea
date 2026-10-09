/**
 * Copyright (c) Scott A Dixon
 *
 * Declares shared editable-edge types for authoring-graph canvases.
 */

export interface AuthoringGraphPoint {
  x: number;
  y: number;
}

export interface AuthoringGraphWaypoint {
  inHandle: AuthoringGraphPoint;
  outHandle: AuthoringGraphPoint;
  position: AuthoringGraphPoint;
}

export interface AuthoringGraphBridge {
  gap: number;
  segmentIndex: number;
  t: number;
}

export interface AuthoringGraphEdgeLayout {
  bridges?: AuthoringGraphBridge[];
  waypoints?: AuthoringGraphWaypoint[];
}

export interface AuthoringGraphEdgeSelection {
  bridgeIndex?: number;
  waypointIndex?: number;
}
