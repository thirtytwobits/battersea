/** Copyright (c) Scott A Dixon */
import type {
  FlowDocument,
  FlowNodeDefinition,
  FlowValidationResult,
} from "@battersea/flow";
import type { FlowNotification } from "./interaction-ports.js";
export interface FlowSummary {
  flow_key: string;
  title: string;
  unavailable_reason?: string;
}
export interface FlowDocumentPort {
  read: (key: string) => Promise<FlowDocument>;
  list: () => Promise<readonly FlowSummary[]>;
  save: (document: FlowDocument) => Promise<{ flow: FlowDocument }>;
  clone: (request: {
    current_flow_key: string;
    next_flow_key: string;
    next_title: string;
  }) => Promise<{ flow: FlowDocument }>;
  delete: (request: { flow_key: string }) => Promise<unknown>;
}
export interface FlowConfirmOptions {
  cancelLabel?: string;
  actionLabel: string;
  message: string;
  title: string;
}
export interface FlowDirtyRegistration {
  canSave: boolean;
  isDirty: boolean;
  save: () => Promise<void>;
  confirmDiscard: (options: FlowConfirmOptions) => Promise<boolean>;
  saveLabel: string;
  saveTitle: string;
}
export interface FlowLifecycleHost {
  confirm: (options: FlowConfirmOptions) => Promise<boolean>;
  confirmDiscard: (options: FlowConfirmOptions) => Promise<boolean>;
  registerDirty: (registration: FlowDirtyRegistration) => void;
  unregisterDirty: () => void;
  notify: (notification: FlowNotification) => void;
  deleteConfirmation: (title: string, dirty: boolean) => string;
}
export type FlowActivationCommand<TResult = FlowActivation> = (
  request: FlowActivationRequest,
) => Promise<TResult>;
export interface FlowActivationRequest {
  flow_key: string;
  node_id: string;
  parameters: Record<string, unknown>;
}
export interface FlowActivation {
  id: string;
  status: "running" | "succeeded" | "failed" | "cancelled";
}
export interface FlowDiagnostic {
  activation_id: string;
  node_id?: string;
  sequence: number;
  phase: string;
  detail?: unknown;
}
/** The embedding application supplies I/O, execution admission and subscription lifetimes. */
export interface FlowEditorPorts {
  documents: FlowDocumentPort;
  catalogue: { read: () => Promise<readonly FlowNodeDefinition[]> };
  validation: {
    validate: (document: FlowDocument) => Promise<FlowValidationResult>;
  };
  activation: {
    activate: FlowActivationCommand;
    cancel: (id: string) => Promise<void>;
    subscribe: (
      id: string,
      listener: (event: FlowDiagnostic) => void,
    ) => () => void;
  };
}
export interface DataflowDocumentFieldState {
  message: string;
  status: "idle" | "loading" | "error";
}
export type FlowNameModalMode = "clone" | "name" | "rename" | "rename-node";
export interface FlowNameDialogState {
  initialValue: string;
  isOpen: boolean;
  mode: FlowNameModalMode;
  pendingAction: "clone" | "save" | "rename" | "rename-node" | null;
  targetNodeId: string | null;
  targetFlowKey: string | null;
}
