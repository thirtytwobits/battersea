/**
 * Copyright (c) Scott A Dixon
 *
 * Declares shared interactive gesture lifecycle contracts for authoring-graph canvases.
 */

export type AuthoringGraphGestureLabel =
  | "Move node"
  | "Move bridge"
  | "Move waypoint"
  | "Move waypoint handle"
  | "Resize bridge";

export interface AuthoringGraphInteractiveGestureDriver {
  beginInteractiveGesture: (label: AuthoringGraphGestureLabel) => void;
  cancelInteractiveGesture: () => void;
  commitInteractiveGesture: () => void;
}

export interface AuthoringGraphInteractiveGestureSession {
  beginGesture: (
    driver: AuthoringGraphInteractiveGestureDriver | undefined,
    label: AuthoringGraphGestureLabel,
  ) => void;
  cancelGesture: (
    driver: AuthoringGraphInteractiveGestureDriver | undefined,
  ) => void;
  commitGesture: (
    driver: AuthoringGraphInteractiveGestureDriver | undefined,
  ) => void;
  getActiveLabel: () => AuthoringGraphGestureLabel | null;
}

export type AuthoringGraphEdgeGestureHandleKind =
  | "anchor"
  | "bridge"
  | "bridgeGap"
  | "inHandle"
  | "outHandle";

export function resolveAuthoringGraphEdgeGestureLabel(
  handleKind: AuthoringGraphEdgeGestureHandleKind,
): AuthoringGraphGestureLabel {
  if (handleKind === "anchor") {
    return "Move waypoint";
  }

  if (handleKind === "inHandle" || handleKind === "outHandle") {
    return "Move waypoint handle";
  }

  if (handleKind === "bridge") {
    return "Move bridge";
  }

  return "Resize bridge";
}

export function createAuthoringGraphInteractiveGestureSession(): AuthoringGraphInteractiveGestureSession {
  let activeLabel: AuthoringGraphGestureLabel | null = null;

  return {
    beginGesture: (driver, label) => {
      if (activeLabel === label) {
        return;
      }

      if (activeLabel !== null) {
        driver?.cancelInteractiveGesture();
      }

      activeLabel = label;
      driver?.beginInteractiveGesture(label);
    },
    cancelGesture: (driver) => {
      if (activeLabel === null) {
        return;
      }

      activeLabel = null;
      driver?.cancelInteractiveGesture();
    },
    commitGesture: (driver) => {
      if (activeLabel === null) {
        return;
      }

      activeLabel = null;
      driver?.commitInteractiveGesture();
    },
    getActiveLabel: () => activeLabel,
  };
}
