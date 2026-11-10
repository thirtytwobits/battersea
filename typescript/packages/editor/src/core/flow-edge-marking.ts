/**
 * Copyright (c) Scott A Dixon
 *
 * Folds the runtime's edge marking into the state the editor draws for each edge.
 */
import type { Runtime } from "@battersea/flow";

/** A record the editor follows: an execution record or a runtime view record's state. */
export interface FlowActivityRecord {
  phase: string;
  detail?: unknown;
}

export type FlowEdgeMarking = Runtime.EdgeMarking;
/** Each edge's marking, keyed by edge id. An edge that holds nothing is absent. */
export type FlowEdgeMarkingState = Readonly<Record<string, FlowEdgeMarking>>;

export const FLOW_EDGE_MARKING_PHASE = "flow.edge.marking";

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function readMarking(value: unknown): FlowEdgeMarking | null {
  if (!isRecord(value)) return null;
  const { queued, close } = value;
  if (typeof queued !== "number" || !Number.isInteger(queued) || queued < 0) {
    return null;
  }
  if (close === undefined) return { queued };
  if (
    !isRecord(close) ||
    typeof close.delivered !== "boolean" ||
    typeof close.failed !== "boolean"
  ) {
    return null;
  }
  return { queued, close: { delivered: close.delivered, failed: close.failed } };
}

function readEdges(value: unknown): Array<[string, FlowEdgeMarking]> | null {
  if (!isRecord(value) || !isRecord(value.edges)) return null;
  return Object.entries(value.edges).flatMap(([edgeId, entry]) => {
    const marking = readMarking(entry);
    return marking ? [[edgeId, marking] as [string, FlowEdgeMarking]] : [];
  });
}

function isEmpty(marking: FlowEdgeMarking): boolean {
  return marking.queued === 0 && marking.close === undefined;
}

/**
 * Applies one followed record to the edge marking.
 *
 * A runtime view `marking` state replaces the marking. A `flow.edge.marking` execution record
 * replaces the entry of each edge it names. Other records leave the marking unchanged.
 */
export function applyFlowEdgeMarking(
  state: FlowEdgeMarkingState,
  record: FlowActivityRecord,
): FlowEdgeMarkingState {
  if (isRecord(record.detail) && record.detail.kind === "marking") {
    const edges = readEdges(record.detail);
    return edges
      ? Object.fromEntries(edges.filter(([, marking]) => !isEmpty(marking)))
      : state;
  }
  if (record.phase !== FLOW_EDGE_MARKING_PHASE) return state;
  const edges = readEdges(record.detail);
  if (!edges || edges.length === 0) return state;
  const next: Record<string, FlowEdgeMarking> = { ...state };
  for (const [edgeId, marking] of edges) {
    if (isEmpty(marking)) delete next[edgeId];
    else next[edgeId] = marking;
  }
  return next;
}

/** The class names that describe an edge's marking. */
export function resolveFlowEdgeMarkingClassNames(
  marking: FlowEdgeMarking | null | undefined,
): string[] {
  if (!marking) return [];
  return [
    marking.queued > 0 ? "flow-studio-edge--occupied" : null,
    marking.close && !marking.close.delivered ? "flow-studio-edge--closing" : null,
    marking.close?.delivered ? "flow-studio-edge--closed" : null,
    marking.close?.failed ? "flow-studio-edge--failed" : null,
  ].filter((name): name is string => name !== null);
}
