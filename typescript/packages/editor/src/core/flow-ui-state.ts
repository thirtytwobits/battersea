/** Copyright (c) Scott A Dixon */
import type {
  FlowNameModalMode,
  FlowSummary as WireFlowSummary,
} from "../ports.js";
import {
  buildDefaultFlowWorkspace,
  slugifyFlowKey,
  type FlowStudioWorkspaceState,
} from "./flow-persistence.js";
export type FlowSelectionChangeResult =
  | "ignored"
  | "loaded"
  | "cancelled"
  | "failed";

export function shouldPromptForFlowName(title: string): boolean {
  return title.trim().length === 0;
}

export function getLoadedFlowKey(workspace: FlowStudioWorkspaceState): string {
  return workspace.baselineFlow?.flow_key?.trim() ?? "";
}

export function getLoadedFlowTitle(
  workspace: FlowStudioWorkspaceState,
): string {
  return workspace.baselineFlow?.title?.trim() ?? "";
}

export function hasLoadedNamedFlow(
  workspace: FlowStudioWorkspaceState,
): boolean {
  return Boolean(getLoadedFlowKey(workspace) && getLoadedFlowTitle(workspace));
}

export function buildCloneFlowTitleSuggestion(
  sourceTitle: string,
  flowSummaries: readonly Pick<WireFlowSummary, "flow_key" | "title">[],
): string {
  const trimmedSourceTitle = sourceTitle.trim() || "Untitled flow";
  const occupiedTitles = new Set(
    flowSummaries
      .map((summary) => summary.title.trim().toLowerCase())
      .filter(Boolean),
  );
  const occupiedKeys = new Set(
    flowSummaries
      .map((summary) => summary.flow_key.trim().toLowerCase())
      .filter(Boolean),
  );

  let suffix = "";
  let index = 1;

  while (true) {
    const candidate = `${trimmedSourceTitle} copy${suffix}`;
    if (
      !occupiedTitles.has(candidate.toLowerCase()) &&
      !occupiedKeys.has(slugifyFlowKey(candidate).toLowerCase())
    ) {
      return candidate;
    }

    index += 1;
    suffix = ` ${index}`;
  }
}

export function getFlowNameModalInitialValue(
  mode: FlowNameModalMode,
  loadedFlowTitle: string,
  flowSummaries: readonly Pick<WireFlowSummary, "flow_key" | "title">[] = [],
): string {
  if (mode === "clone") {
    return buildCloneFlowTitleSuggestion(loadedFlowTitle, flowSummaries);
  }

  return mode === "rename" || mode === "rename-node"
    ? loadedFlowTitle.trim()
    : "";
}

export function buildWorkspaceForFailedFlowLoad(): FlowStudioWorkspaceState {
  return buildDefaultFlowWorkspace();
}

export function resolveFlowValidationMessage(rawMessage: string): string {
  const detail = rawMessage.trim();
  if (!detail) {
    return "The dataflow could not be validated.";
  }

  if (/No flow exists with key\s*"[^"]*"\./i.test(detail)) {
    return "This draft could not be validated. The editor tried to validate it as a saved flow instead of the current draft.";
  }

  return detail;
}

export async function resolveFlowSelectionChange(options: {
  currentLoadedFlowKey: string;
  isDirty: boolean;
  loadFlow: (flowKey: string) => Promise<boolean>;
  nextFlowKey: string;
  confirmDiscard: () => Promise<boolean>;
}): Promise<FlowSelectionChangeResult> {
  if (
    !options.nextFlowKey ||
    options.nextFlowKey === options.currentLoadedFlowKey
  ) {
    return "ignored";
  }

  if (options.isDirty) {
    const discardConfirmed = await options.confirmDiscard();
    if (!discardConfirmed) {
      return "cancelled";
    }
  }

  return (await options.loadFlow(options.nextFlowKey)) ? "loaded" : "failed";
}
