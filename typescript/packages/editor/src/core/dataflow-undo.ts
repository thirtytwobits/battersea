/**
 * Copyright (c) Scott A Dixon
 *
 * Implements dataflow-specific structural snapshotting and restore logic for
 * authoring undo. Backed directly by `react-amnesia`.
 */
import type { Amnesia } from "react-amnesia";

import type {
  FlowStudioEdge,
  FlowStudioNode,
} from "./dataflow-editor-state.js";
import type { FlowStudioNodeData } from "./flow-drag.js";
import type { FlowStudioWorkspaceState } from "./flow-persistence.js";

export interface DataflowStructuralSnapshot {
  edges: FlowStudioEdge[];
  nodes: FlowStudioNode[];
  execution: FlowStudioWorkspaceState["execution"];
}

export interface DataflowUndoHistoryState {
  restoreLayout: boolean;
  snapshot: DataflowStructuralSnapshot;
}

export function buildDataflowUndoHistoryState(
  snapshot: DataflowStructuralSnapshot,
  restoreLayout: boolean,
): DataflowUndoHistoryState {
  return {
    restoreLayout,
    snapshot,
  };
}

interface ComparableDataflowStructuralEdge {
  data: {
    kind: "signal" | "token";
    order: number;
    queue: NonNullable<FlowStudioEdge["data"]>["queue"];
  };
  id: string;
  source: string;
  sourceHandle: string | null;
  target: string;
  targetHandle: string | null;
}

interface ComparableDataflowStructuralNode {
  data: Omit<FlowStudioNodeData, "controllerPortPlacement">;
  id: string;
}

function cloneJsonValue<Value>(value: Value): Value {
  if (Array.isArray(value)) {
    return value.map((entry) => cloneJsonValue(entry)) as Value;
  }

  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(([key, nested]) => [
        key,
        cloneJsonValue(nested),
      ]),
    ) as Value;
  }

  return value;
}

function stableJsonStringify(value: unknown): string {
  return JSON.stringify(normalizeJsonValue(value));
}

function normalizeJsonValue(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map((entry) => normalizeJsonValue(entry));
  }

  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, nested]) => [key, normalizeJsonValue(nested)]),
    );
  }

  return value;
}

function cloneFlowStudioNodeData(data: FlowStudioNodeData): FlowStudioNodeData {
  return cloneJsonValue(data);
}

function cloneFlowStudioEdgeData(
  data: FlowStudioEdge["data"] | undefined,
): FlowStudioEdge["data"] | undefined {
  return data ? cloneJsonValue(data) : undefined;
}

function cloneFlowStudioNode(node: FlowStudioNode): FlowStudioNode {
  return {
    className: node.className,
    data: cloneFlowStudioNodeData(node.data),
    id: node.id,
    position: {
      x: node.position.x,
      y: node.position.y,
    },
    type: node.type ?? "flowStudio",
  };
}

function cloneFlowStudioEdge(edge: FlowStudioEdge): FlowStudioEdge {
  return {
    data: cloneFlowStudioEdgeData(edge.data),
    id: edge.id,
    source: edge.source,
    sourceHandle: edge.sourceHandle ?? null,
    target: edge.target,
    targetHandle: edge.targetHandle ?? null,
  };
}

function buildComparableStructuralNode(
  node: FlowStudioNode,
): ComparableDataflowStructuralNode {
  const { controllerPortPlacement: _controllerPortPlacement, ...data } =
    cloneFlowStudioNodeData(node.data);

  return {
    data,
    id: node.id,
  };
}

function buildComparableStructuralEdge(
  edge: FlowStudioEdge,
): ComparableDataflowStructuralEdge {
  return {
    data: {
      kind: edge.data?.kind ?? "token",
      order: typeof edge.data?.order === "number" ? edge.data.order : 0,
      queue: cloneJsonValue(edge.data?.queue),
    },
    id: edge.id,
    source: edge.source,
    sourceHandle: edge.sourceHandle ?? null,
    target: edge.target,
    targetHandle: edge.targetHandle ?? null,
  };
}

function edgeStructureMatches(
  currentEdge: FlowStudioEdge,
  targetEdge: FlowStudioEdge,
): boolean {
  return (
    currentEdge.source === targetEdge.source &&
    (currentEdge.sourceHandle ?? null) === (targetEdge.sourceHandle ?? null) &&
    currentEdge.target === targetEdge.target &&
    (currentEdge.targetHandle ?? null) === (targetEdge.targetHandle ?? null) &&
    (currentEdge.data?.kind ?? "token") === (targetEdge.data?.kind ?? "token")
  );
}

export function buildDataflowStructuralSnapshot(
  workspace: Pick<FlowStudioWorkspaceState, "edges" | "nodes" | "execution">,
): DataflowStructuralSnapshot {
  return {
    execution: cloneJsonValue(workspace.execution),
    edges: workspace.edges.map((edge) => cloneFlowStudioEdge(edge)),
    nodes: workspace.nodes.map((node) => cloneFlowStudioNode(node)),
  };
}

export function areDataflowStructuralSnapshotsEqual(
  left: DataflowStructuralSnapshot,
  right: DataflowStructuralSnapshot,
): boolean {
  return (
    stableJsonStringify({
      execution: left.execution,
      edges: [...left.edges]
        .sort((first, second) => first.id.localeCompare(second.id))
        .map((edge) => buildComparableStructuralEdge(edge)),
      nodes: [...left.nodes]
        .sort((first, second) => first.id.localeCompare(second.id))
        .map((node) => buildComparableStructuralNode(node)),
    }) ===
    stableJsonStringify({
      execution: right.execution,
      edges: [...right.edges]
        .sort((first, second) => first.id.localeCompare(second.id))
        .map((edge) => buildComparableStructuralEdge(edge)),
      nodes: [...right.nodes]
        .sort((first, second) => first.id.localeCompare(second.id))
        .map((node) => buildComparableStructuralNode(node)),
    })
  );
}

export function areDataflowStructuralSnapshotsExactlyEqual(
  left: DataflowStructuralSnapshot,
  right: DataflowStructuralSnapshot,
): boolean {
  return stableJsonStringify(left) === stableJsonStringify(right);
}

/**
 * Push a structural-or-layout transformation onto the supplied amnesia store
 * and return the resulting workspace.
 *
 * `previousState` is the last `(snapshot, restoreLayout)` pair amnesia
 * captured (initial state, or the most recent push / amend / undo / redo
 * target). It is *not* necessarily derived from `workspace`: when the
 * workspace was live-mutated outside the undo path (e.g. node dragging via
 * `setWorkspace` direct), the live mutation is silently absorbed into the
 * next commit, matching the in-house `SnapshotHistory.getPresent()`
 * semantics.
 *
 * For layout commits (`restoreLayout: true`) the undo closure restores the
 * snapshot of the workspace at commit time *with* its layout, so undoing a
 * layout edit always snaps back to the layout the user had immediately
 * before the edit.
 *
 * For structural commits (`restoreLayout: false`) the undo closure restores
 * the previous lastCaptured state — which may itself carry
 * `restoreLayout: true` from a prior layout commit. Live layout drift
 * between two structural commits is preserved (the next structural commit's
 * undo restores the user's current layout).
 */
export function commitDataflowSnapshotChange(input: {
  amnesia: Amnesia;
  compareSnapshots: (
    left: DataflowStructuralSnapshot,
    right: DataflowStructuralSnapshot,
  ) => boolean;
  label: string;
  options?: {
    mergeKey?: string | null;
  };
  previousState: DataflowUndoHistoryState;
  restoreLayout: boolean;
  restoreState: (state: DataflowUndoHistoryState) => void;
  transform: (workspace: FlowStudioWorkspaceState) => FlowStudioWorkspaceState;
  workspace: FlowStudioWorkspaceState;
}): FlowStudioWorkspaceState {
  const nextWorkspace = input.transform(input.workspace);
  if (nextWorkspace === input.workspace) {
    return input.workspace;
  }

  const currentSnapshot = buildDataflowStructuralSnapshot(input.workspace);
  const nextSnapshot = buildDataflowStructuralSnapshot(nextWorkspace);
  if (input.compareSnapshots(currentSnapshot, nextSnapshot)) {
    return nextWorkspace;
  }

  // For layout commits, the undo closure restores the live snapshot at
  // commit time with `restoreLayout: true` so undoing reapplies the user's
  // layout from the moment of the commit. For structural commits, the undo
  // closure restores whatever amnesia last captured — letting prior layout
  // edits (or fresh live drift) ride through.
  const undoState: DataflowUndoHistoryState = input.restoreLayout
    ? buildDataflowUndoHistoryState(currentSnapshot, true)
    : input.previousState;
  const redoState: DataflowUndoHistoryState = buildDataflowUndoHistoryState(
    nextSnapshot,
    input.restoreLayout,
  );

  const restore = input.restoreState;
  const mergeKey = input.options?.mergeKey ?? null;

  void input.amnesia.push(
    {
      coalesceKey: mergeKey ?? undefined,
      coalesceWindowMs: Number.POSITIVE_INFINITY,
      label: input.label,
      redo: () => {
        restore(redoState);
      },
      undo: () => {
        restore(undoState);
      },
    },
    {
      applied: true,
    },
  );

  return nextWorkspace;
}

/**
 * Retroactively record a layout transition onto the amnesia stack.
 *
 * `previousSnapshot` is the layout the user had at the start of the gesture
 * (captured via `captureSnapshot` before live-dragging began). The current
 * workspace already reflects the post-gesture layout. This call inserts a
 * single undoable entry whose undo restores `previousSnapshot` with layout
 * and whose redo restores the current snapshot.
 */
export function pushDataflowPresentSnapshotChange(input: {
  amnesia: Amnesia;
  label: string;
  options?: {
    mergeKey?: string | null;
  };
  previousSnapshot: DataflowStructuralSnapshot;
  restoreLayout?: boolean;
  restoreState: (state: DataflowUndoHistoryState) => void;
  workspace: FlowStudioWorkspaceState;
}): boolean {
  const nextSnapshot = buildDataflowStructuralSnapshot(input.workspace);
  if (
    areDataflowStructuralSnapshotsExactlyEqual(
      input.previousSnapshot,
      nextSnapshot,
    )
  ) {
    return false;
  }

  const restoreLayout = input.restoreLayout ?? true;
  const undoState = buildDataflowUndoHistoryState(input.previousSnapshot, true);
  const redoState = buildDataflowUndoHistoryState(nextSnapshot, restoreLayout);

  const restore = input.restoreState;
  const mergeKey = input.options?.mergeKey ?? null;

  void input.amnesia.push(
    {
      coalesceKey: mergeKey ?? undefined,
      coalesceWindowMs: Number.POSITIVE_INFINITY,
      label: input.label,
      redo: () => {
        restore(redoState);
      },
      undo: () => {
        restore(undoState);
      },
    },
    {
      applied: true,
    },
  );

  return true;
}

export function restoreDataflowStructuralWorkspace(options: {
  currentWorkspace: FlowStudioWorkspaceState;
  preserveLayout?: boolean;
  snapshot: DataflowStructuralSnapshot;
}): FlowStudioWorkspaceState {
  const preserveLayout = options.preserveLayout ?? true;
  const currentNodesById = new Map(
    options.currentWorkspace.nodes.map((node) => [node.id, node] as const),
  );
  const currentEdgesById = new Map(
    options.currentWorkspace.edges.map((edge) => [edge.id, edge] as const),
  );

  const nodes = options.snapshot.nodes.map((targetNode) => {
    const currentNode = currentNodesById.get(targetNode.id);
    if (!currentNode) {
      return cloneFlowStudioNode(targetNode);
    }

    return {
      ...cloneFlowStudioNode(targetNode),
      data: {
        ...cloneFlowStudioNodeData(targetNode.data),
        controllerPortPlacement: preserveLayout
          ? currentNode.data.controllerPortPlacement
          : targetNode.data.controllerPortPlacement,
      },
      position: {
        x: preserveLayout ? currentNode.position.x : targetNode.position.x,
        y: preserveLayout ? currentNode.position.y : targetNode.position.y,
      },
    };
  });

  const edges = options.snapshot.edges.map((targetEdge) => {
    const currentEdge = currentEdgesById.get(targetEdge.id);
    if (!currentEdge || !edgeStructureMatches(currentEdge, targetEdge)) {
      return cloneFlowStudioEdge(targetEdge);
    }

    const targetEdgeData = cloneFlowStudioEdgeData(targetEdge.data) ?? {
      kind: "token" as const,
      order: 0,
    };

    return {
      ...cloneFlowStudioEdge(targetEdge),
      data: {
        ...targetEdgeData,
        bridges: preserveLayout
          ? currentEdge.data?.bridges
            ? cloneJsonValue(currentEdge.data.bridges)
            : undefined
          : targetEdgeData.bridges,
        waypoints: preserveLayout
          ? currentEdge.data?.waypoints
            ? cloneJsonValue(currentEdge.data.waypoints)
            : undefined
          : targetEdgeData.waypoints,
      },
    };
  });

  return {
    ...options.currentWorkspace,
    execution: cloneJsonValue(options.snapshot.execution),
    edges,
    nodes,
    selectedTarget: { kind: "none" },
  };
}
