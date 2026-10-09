/**
 * Copyright (c) Scott A Dixon
 *
 * Adapts shared authoring-graph gesture lifecycles onto feature-specific undo.
 */
import type {
  AuthoringGraphGestureLabel,
  AuthoringGraphInteractiveGestureDriver,
} from "./authoring-graph-gestures.js";

export function createAuthoringInteractiveGestureDriver<TSnapshot>(input: {
  capture: () => TSnapshot;
  commit: (label: AuthoringGraphGestureLabel, snapshot: TSnapshot) => void;
}): AuthoringGraphInteractiveGestureDriver {
  let activeLabel: AuthoringGraphGestureLabel | null = null;
  let activeSnapshot: TSnapshot | null = null;

  return {
    beginInteractiveGesture: (label) => {
      activeLabel = label;
      activeSnapshot = input.capture();
    },
    cancelInteractiveGesture: () => {
      activeLabel = null;
      activeSnapshot = null;
    },
    commitInteractiveGesture: () => {
      if (!activeLabel || !activeSnapshot) {
        return;
      }

      const label = activeLabel;
      const snapshot = activeSnapshot;
      activeLabel = null;
      activeSnapshot = null;
      input.commit(label, snapshot);
    },
  };
}
