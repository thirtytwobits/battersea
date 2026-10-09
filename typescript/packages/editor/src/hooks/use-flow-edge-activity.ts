/** Copyright (c) Scott A Dixon */
import React from "react";
import type { FlowStudioEdge } from "../core/dataflow-editor-state.js";
import {
  FLOW_STUDIO_EDGE_ACTIVE_FALLBACK_MS,
  FLOW_STUDIO_EDGE_FADE_OUT_MS,
  resolveFlowStudioEdgeFadeDelayMs,
  pruneFlowStudioEdgeActivationState,
  resolveFlowStudioEdgeActivityEvent,
  type FlowStudioEdgeActivationState,
  type FlowActivityRecord,
} from "../core/flow-edge-activation.js";
interface FlowStudioEdgeTimers {
  activatedAtMs: number | null;
  fallbackFadeTimer: ReturnType<typeof setTimeout> | null;
  fadeStartTimer: ReturnType<typeof setTimeout> | null;
  removeTimer: ReturnType<typeof setTimeout> | null;
}

function createEmptyFlowStudioEdgeTimers(): FlowStudioEdgeTimers {
  return {
    activatedAtMs: null,
    fallbackFadeTimer: null,
    fadeStartTimer: null,
    removeTimer: null,
  };
}

export function useFlowEdgeActivity(options: {
  edges: readonly FlowStudioEdge[];
  subscribe:
    | ((listener: (record: FlowActivityRecord) => void) => () => void)
    | null;
}): FlowStudioEdgeActivationState {
  const [edgeActivationState, setEdgeActivationState] =
    React.useState<FlowStudioEdgeActivationState>({});
  const edgeIdSetRef = React.useRef<ReadonlySet<string>>(new Set());
  const edgeTimersRef = React.useRef<Record<string, FlowStudioEdgeTimers>>({});
  const clearEdgeTimers = React.useCallback((edgeId: string) => {
    const timers = edgeTimersRef.current[edgeId];
    if (!timers) {
      return;
    }

    if (timers.fallbackFadeTimer !== null) {
      clearTimeout(timers.fallbackFadeTimer);
    }
    if (timers.fadeStartTimer !== null) {
      clearTimeout(timers.fadeStartTimer);
    }
    if (timers.removeTimer !== null) {
      clearTimeout(timers.removeTimer);
    }

    delete edgeTimersRef.current[edgeId];
  }, []);

  const clearAllEdgeTimers = React.useCallback(() => {
    for (const edgeId of Object.keys(edgeTimersRef.current)) {
      clearEdgeTimers(edgeId);
    }
  }, [clearEdgeTimers]);

  const startEdgeFade = React.useCallback(
    (edgeId: string) => {
      clearEdgeTimers(edgeId);
      edgeTimersRef.current[edgeId] = createEmptyFlowStudioEdgeTimers();
      setEdgeActivationState((current) => ({
        ...current,
        [edgeId]: "fading",
      }));
      edgeTimersRef.current[edgeId].removeTimer = setTimeout(() => {
        clearEdgeTimers(edgeId);
        setEdgeActivationState((current) => {
          if (!(edgeId in current)) {
            return current;
          }

          const nextState = { ...current };
          delete nextState[edgeId];
          return nextState;
        });
      }, FLOW_STUDIO_EDGE_FADE_OUT_MS);
    },
    [clearEdgeTimers],
  );

  const transitionEdgeToFading = React.useCallback(
    (edgeId: string) => {
      const activatedAtMs = edgeTimersRef.current[edgeId]?.activatedAtMs;
      if (activatedAtMs === null || activatedAtMs === undefined) {
        startEdgeFade(edgeId);
        return;
      }

      const fadeDelayMs = resolveFlowStudioEdgeFadeDelayMs({
        activatedAtMs,
        nowMs: Date.now(),
      });
      if (fadeDelayMs <= 0) {
        startEdgeFade(edgeId);
        return;
      }

      clearEdgeTimers(edgeId);
      edgeTimersRef.current[edgeId] = createEmptyFlowStudioEdgeTimers();
      edgeTimersRef.current[edgeId].activatedAtMs = activatedAtMs;
      edgeTimersRef.current[edgeId].fadeStartTimer = setTimeout(() => {
        startEdgeFade(edgeId);
      }, fadeDelayMs);
    },
    [clearEdgeTimers, startEdgeFade],
  );

  const activateEdge = React.useCallback(
    (edgeId: string) => {
      clearEdgeTimers(edgeId);
      edgeTimersRef.current[edgeId] = createEmptyFlowStudioEdgeTimers();
      edgeTimersRef.current[edgeId].activatedAtMs = Date.now();
      setEdgeActivationState((current) => ({
        ...current,
        [edgeId]: "active",
      }));
      edgeTimersRef.current[edgeId].fallbackFadeTimer = setTimeout(() => {
        transitionEdgeToFading(edgeId);
      }, FLOW_STUDIO_EDGE_ACTIVE_FALLBACK_MS);
    },
    [clearEdgeTimers, transitionEdgeToFading],
  );

  React.useEffect(() => {
    const edgeIds = new Set(options.edges.map((edge) => edge.id));
    edgeIdSetRef.current = edgeIds;
    setEdgeActivationState((current) =>
      pruneFlowStudioEdgeActivationState(current, edgeIds),
    );

    for (const edgeId of Object.keys(edgeTimersRef.current)) {
      if (!edgeIds.has(edgeId)) {
        clearEdgeTimers(edgeId);
      }
    }
  }, [clearEdgeTimers, options.edges]);

  React.useEffect(() => {
    clearAllEdgeTimers();
    setEdgeActivationState({});
    if (!options.subscribe) return;
    const stopFollowing = options.subscribe((record) => {
      const activity = resolveFlowStudioEdgeActivityEvent(record);
      if (!activity) {
        return;
      }

      const visibleEdgeIds = activity.edgeIds.filter((edgeId) =>
        edgeIdSetRef.current.has(edgeId),
      );
      if (visibleEdgeIds.length === 0) {
        return;
      }

      for (const edgeId of visibleEdgeIds) {
        if (activity.transition === "activate") {
          activateEdge(edgeId);
        } else {
          transitionEdgeToFading(edgeId);
        }
      }
    });

    return () => {
      clearAllEdgeTimers();
      setEdgeActivationState({});
      stopFollowing();
    };
  }, [
    activateEdge,
    clearAllEdgeTimers,
    options.subscribe,
    transitionEdgeToFading,
  ]);

  return edgeActivationState;
}
