/**
 * Copyright (c) Scott A Dixon
 *
 * Resolves live edge-activation state for the editor's dataflow workspace.
 */
export interface FlowActivityRecord {
  phase: string;
  detail?: unknown;
}

export const FLOW_STUDIO_EDGE_ACTIVE_FALLBACK_MS = 800;
export const FLOW_STUDIO_EDGE_FADE_OUT_MS = 1200;
export const FLOW_STUDIO_EDGE_MIN_ACTIVE_MS = 280;

export type FlowStudioEdgeActivationPhase = "active" | "fading";
export type FlowStudioEdgeActivationState = Record<
  string,
  FlowStudioEdgeActivationPhase
>;

export interface FlowStudioEdgeActivityEvent {
  edgeIds: string[];
  transition: "activate" | "fade";
}

export function resolveFlowStudioEdgeFadeDelayMs(options: {
  activatedAtMs: number;
  minimumActiveMs?: number;
  nowMs: number;
}): number {
  const minimumActiveMs =
    options.minimumActiveMs ?? FLOW_STUDIO_EDGE_MIN_ACTIVE_MS;
  return Math.max(options.activatedAtMs + minimumActiveMs - options.nowMs, 0);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function readStringProperty(
  record: Record<string, unknown>,
  key: string,
): string | null {
  return typeof record[key] === "string" && record[key].trim().length > 0
    ? record[key]
    : null;
}

function readReceiveEdgeId(detail: unknown): string | null {
  if (!isRecord(detail)) {
    return null;
  }

  return (
    readStringProperty(detail, "edgeId") ??
    readStringProperty(detail, "edge_id")
  );
}

function readEmitEdgeIds(detail: unknown): string[] {
  if (!isRecord(detail) || !Array.isArray(detail.targets)) {
    return [];
  }

  const edgeIds = detail.targets
    .flatMap((target) =>
      isRecord(target) ? [readStringProperty(target, "edgeId")] : [],
    )
    .filter((edgeId): edgeId is string => edgeId !== null);

  return [...new Set(edgeIds)];
}

export function resolveFlowStudioEdgeActivityEvent<
  T extends FlowActivityRecord,
>(record: T): FlowStudioEdgeActivityEvent | null {
  switch (record.phase) {
    case "flow.signal.emit":
    case "flow.token.emit": {
      const edgeIds = readEmitEdgeIds(record.detail);
      return edgeIds.length > 0
        ? {
            edgeIds,
            transition: "activate",
          }
        : null;
    }
    case "flow.signal.receive":
    case "flow.token.receive": {
      const edgeId = readReceiveEdgeId(record.detail);
      return edgeId
        ? {
            edgeIds: [edgeId],
            transition: "fade",
          }
        : null;
    }
    default:
      return null;
  }
}

export function pruneFlowStudioEdgeActivationState(
  state: FlowStudioEdgeActivationState,
  validEdgeIds: ReadonlySet<string>,
): FlowStudioEdgeActivationState {
  const remainingEntries = Object.entries(state).filter(([edgeId]) =>
    validEdgeIds.has(edgeId),
  );

  return remainingEntries.length === Object.keys(state).length
    ? state
    : Object.fromEntries(remainingEntries);
}
