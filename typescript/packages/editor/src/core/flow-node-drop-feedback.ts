/**
 * Copyright (c) Scott A Dixon
 *
 * Implements flow node drop feedback for the editor's dataflow workspace.
 */
export type FlowNodeDropFailureReason =
  | "canvas-unavailable"
  | "invalid-payload"
  | "node-build-failed";

export interface FlowNodeDropNotification {
  message: string;
  title: string;
}

export function buildFlowNodeDropFailureNotification(params: {
  nodeTitle: string;
  reason: FlowNodeDropFailureReason;
}): FlowNodeDropNotification {
  const trimmedTitle = params.nodeTitle.trim();
  const title = trimmedTitle
    ? `Couldn't add ${trimmedTitle}`
    : "Couldn't add node";

  switch (params.reason) {
    case "canvas-unavailable":
      return {
        title,
        message:
          "The canvas was not ready to accept that node. Try again in a moment.",
      };
    case "invalid-payload":
      return {
        title,
        message: "The dragged node data was invalid. Try dragging it in again.",
      };
    case "node-build-failed":
      return {
        title,
        message:
          "Flow Studio hit an error while creating that node. Try again.",
      };
    default:
      return {
        title,
        message: "Flow Studio could not add that node. Try again.",
      };
  }
}
