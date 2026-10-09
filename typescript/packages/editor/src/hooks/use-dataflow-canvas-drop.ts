/**
 * Copyright (c) Scott A Dixon
 *
 * Manages node drag and drop behaviour for the editor's dataflow canvas.
 */
import React from "react";
import type { DragEndEvent, DragStartEvent } from "@dnd-kit/core";
import type { ReactFlowInstance } from "@xyflow/react";

import type { FlowNodeDefinition as WireFlowNodeDefinition } from "@battersea/flow";

import type {
  FlowStudioEdge,
  FlowStudioNode,
  PendingNodeDropState,
} from "../core/dataflow-editor-state.js";
import type { FlowInteractionPorts } from "../interaction-ports.js";
import {
  createDroppedFlowNode,
  isFlowNodePaletteDragId,
  parseFlowNodeDragPayload,
  resolveDroppedFlowNodePosition,
} from "../core/flow-drag.js";
import { formatFlowDefinitionTitle } from "../core/flow-node-definitions.js";
import {
  findNextFlowNodeIndex,
  type FlowStudioWorkspaceState,
} from "../core/flow-persistence.js";
import {
  buildFlowNodeDropFailureNotification,
  type FlowNodeDropFailureReason,
} from "../core/flow-node-drop-feedback.js";

import { DATAFLOW_CANVAS_DROPZONE_ID } from "../core/flow-drag.js";

export type CanvasDropResult =
  | {
      kind: "failure";
      reason: FlowNodeDropFailureReason;
      title: string;
    }
  | {
      kind: "success";
      nextNode: FlowStudioNode;
      title: string;
    };

/** Whether the drag has every input required to commit a canvas placement. */
export function didDataflowDragResultInPlacement(input: {
  dropZonePresent: boolean;
  overId: string | number | null | undefined;
  pointerPresent: boolean;
  validDragData: boolean;
}): boolean {
  return (
    input.validDragData &&
    input.overId === DATAFLOW_CANVAS_DROPZONE_ID &&
    input.dropZonePresent &&
    input.pointerPresent
  );
}

/** Distinguishes a deliberate drag from a pointer press released in place. */
export function didDataflowPaletteDragMove(input: {
  delta?: { x: number; y: number } | null;
}): boolean {
  const delta = input.delta;
  return (
    delta !== null &&
    delta !== undefined &&
    (Math.abs(delta.x) > 0 || Math.abs(delta.y) > 0)
  );
}

/** Builds either the placed node or the user-facing failure classification. */
export function resolveCanvasDropResult(options: {
  clientX: number;
  clientY: number;
  dragOffset?: { x: number; y: number } | null;
  dragPayload: string;
  flowInstance: ReactFlowInstance<FlowStudioNode, FlowStudioEdge> | null;
  nextNodeIndex: number;
  pendingDropTitle: string;
}): CanvasDropResult {
  if (!options.flowInstance) {
    return {
      kind: "failure",
      reason: "canvas-unavailable",
      title: options.pendingDropTitle,
    };
  }

  const payload = parseFlowNodeDragPayload(options.dragPayload);
  if (!payload) {
    return {
      kind: "failure",
      reason: "invalid-payload",
      title: options.pendingDropTitle,
    };
  }

  try {
    const resolvedPayload = options.dragOffset
      ? {
          ...payload,
          dragOffset: options.dragOffset,
        }
      : payload;

    return {
      kind: "success",
      nextNode: createDroppedFlowNode({
        nextIndex: options.nextNodeIndex,
        payload: resolvedPayload,
        position: resolveDroppedFlowNodePosition({
          cursorPosition: options.flowInstance.screenToFlowPosition({
            x: options.clientX,
            y: options.clientY,
          }),
          payload: resolvedPayload,
        }),
      }),
      title: payload.title,
    };
  } catch {
    return {
      kind: "failure",
      reason: "node-build-failed",
      title: payload.title,
    };
  }
}

export function useDataflowCanvasDrop(options: {
  ports: FlowInteractionPorts;
  commitStructuralChange: (
    label: string,
    transform: (
      workspace: FlowStudioWorkspaceState,
    ) => FlowStudioWorkspaceState,
  ) => void;
  nodes: readonly FlowStudioNode[];
  setWorkspace: React.Dispatch<React.SetStateAction<FlowStudioWorkspaceState>>;
}) {
  const flowInstanceRef = React.useRef<ReactFlowInstance<
    FlowStudioNode,
    FlowStudioEdge
  > | null>(null);
  const nextNodeIndexRef = React.useRef(1);
  const pendingNodeDropRef = React.useRef<PendingNodeDropState>({
    status: "idle",
    title: "",
  });
  const [activeDragDefinition, setActiveDragDefinition] =
    React.useState<WireFlowNodeDefinition | null>(null);
  const [animateRejectedDrop, setAnimateRejectedDrop] = React.useState(true);

  React.useEffect(() => {
    nextNodeIndexRef.current = findNextFlowNodeIndex([...options.nodes]);
  }, [options.nodes]);

  const notifyNodeDropFailure = React.useCallback(
    (nodeTitle: string, reason: FlowNodeDropFailureReason) => {
      const notification = buildFlowNodeDropFailureNotification({
        nodeTitle,
        reason,
      });

      options.ports.notify({
        id: "flow-node-drop-failure",
        message: notification.message,
        title: notification.title,
        tone: "warning",
      });
    },
    [options.ports],
  );

  const handlePaletteDragStart = React.useCallback((event: DragStartEvent) => {
    if (!isFlowNodePaletteDragId(event.active.id)) {
      setActiveDragDefinition(null);
      pendingNodeDropRef.current = {
        status: "idle",
        title: "",
      };
      return;
    }

    const dragData = readFlowNodePaletteDragData(event.active.data.current);
    setAnimateRejectedDrop(true);

    if (!dragData) {
      setActiveDragDefinition(null);
      pendingNodeDropRef.current = {
        status: "idle",
        title: "",
      };
      return;
    }

    setActiveDragDefinition(dragData.nodeDefinition);
    pendingNodeDropRef.current = {
      status: "pending",
      title: formatFlowDefinitionTitle(dragData.nodeDefinition.class_name),
    };
  }, []);

  const handlePaletteDragCancel = React.useCallback(() => {
    setAnimateRejectedDrop(true);
    setActiveDragDefinition(null);
    pendingNodeDropRef.current = {
      status: "idle",
      title: "",
    };
  }, []);

  const handlePaletteDragEnd = React.useCallback(
    (event: DragEndEvent) => {
      if (!isFlowNodePaletteDragId(event.active.id)) {
        setAnimateRejectedDrop(true);
        setActiveDragDefinition(null);
        pendingNodeDropRef.current = {
          status: "idle",
          title: "",
        };
        return;
      }

      if (!didDataflowPaletteDragMove({ delta: event.delta })) {
        setAnimateRejectedDrop(true);
        setActiveDragDefinition(null);
        pendingNodeDropRef.current = {
          status: "idle",
          title: "",
        };
        return;
      }

      const dragData = readFlowNodePaletteDragData(event.active.data.current);
      const pointer = options.ports.resolveDragReleasePoint(event);
      const didPlaceNode = didDataflowDragResultInPlacement({
        dropZonePresent: flowInstanceRef.current !== null,
        overId: event.over?.id,
        pointerPresent: pointer !== null,
        validDragData: dragData !== null,
      });

      setAnimateRejectedDrop(!didPlaceNode);
      setActiveDragDefinition(null);

      if (event.over?.id !== DATAFLOW_CANVAS_DROPZONE_ID) {
        pendingNodeDropRef.current = {
          status: "idle",
          title: "",
        };
        return;
      }

      const pendingDropTitle = pendingNodeDropRef.current.title;

      if (!dragData) {
        pendingNodeDropRef.current = {
          status: "failed",
          title: pendingDropTitle,
        };
        notifyNodeDropFailure(pendingDropTitle, "invalid-payload");
        return;
      }

      if (!pointer) {
        pendingNodeDropRef.current = {
          status: "failed",
          title: pendingDropTitle,
        };
        notifyNodeDropFailure(pendingDropTitle, "canvas-unavailable");
        return;
      }

      const result = resolveCanvasDropResult({
        clientX: pointer.x,
        clientY: pointer.y,
        dragOffset: options.ports.resolveDragGrabOffset(event),
        dragPayload: dragData.flowNodeDragPayload,
        flowInstance: flowInstanceRef.current,
        nextNodeIndex: nextNodeIndexRef.current,
        pendingDropTitle,
      });

      if (result.kind === "failure") {
        pendingNodeDropRef.current = {
          status: "failed",
          title: result.title,
        };
        notifyNodeDropFailure(result.title, result.reason);
        return;
      }

      nextNodeIndexRef.current += 1;
      options.commitStructuralChange("Add node", (current) => ({
        ...current,
        nodes: current.nodes.concat(result.nextNode),
        selectedTarget: {
          kind: "node",
          nodeId: result.nextNode.id,
        },
      }));

      pendingNodeDropRef.current = {
        status: "succeeded",
        title: result.title,
      };
    },
    [notifyNodeDropFailure, options.commitStructuralChange, options.ports],
  );

  return {
    activeDragDefinition,
    animateRejectedDrop,
    flowInstanceRef,
    handlePaletteDragCancel,
    handlePaletteDragEnd,
    handlePaletteDragStart,
  };
}

function readFlowNodePaletteDragData(value: unknown): {
  flowNodeDragPayload: string;
  nodeDefinition: WireFlowNodeDefinition;
} | null {
  if (!value || typeof value !== "object") {
    return null;
  }

  const candidate = value as Partial<{
    flowNodeDragPayload: string;
    nodeDefinition: WireFlowNodeDefinition;
  }>;

  if (typeof candidate.flowNodeDragPayload !== "string") {
    return null;
  }

  if (
    !candidate.nodeDefinition ||
    typeof candidate.nodeDefinition.class_name !== "string"
  ) {
    return null;
  }

  return {
    flowNodeDragPayload: candidate.flowNodeDragPayload,
    nodeDefinition: candidate.nodeDefinition,
  };
}
