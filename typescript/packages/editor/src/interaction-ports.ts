/** Copyright (c) Scott A Dixon */
import type { DragEndEvent } from "@dnd-kit/core";
export interface FlowNotification {
  id?: string;
  message: string;
  tone: "ready" | "warning" | "error";
  title?: string;
}
export interface FlowInteractionPorts {
  notify: (notification: FlowNotification) => void;
  isKeyboardBlocked: (event: KeyboardEvent) => boolean;
  resolveDragGrabOffset: (
    event: DragEndEvent,
  ) => { x: number; y: number } | null;
  resolveDragReleasePoint: (
    event: DragEndEvent,
  ) => { x: number; y: number } | null;
}
