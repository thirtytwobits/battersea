import type { FlowDocument, FlowExecutionPolicy } from "./contract.js";
import { DEFAULT_FLOW_EXECUTION_LIMITS, FLOW_DOCUMENT_VERSION } from "./defaults.js";

/** Execution policy for a newly authored document. Stored documents require an explicit upgrade. */
export function createFlowExecutionPolicy(sourceOrder: string[] = []): FlowExecutionPolicy {
  return { source_order: [...sourceOrder], limits: structuredClone(DEFAULT_FLOW_EXECUTION_LIMITS) };
}

/** Version admission; complete graph validation belongs to the catalogue. */
export function requireSupportedFlowVersion(document: Pick<FlowDocument, "version" | "execution">): void {
  if (document.version !== FLOW_DOCUMENT_VERSION) {
    throw new Error(`Flow document version ${document.version} requires an explicit upgrade to version ${FLOW_DOCUMENT_VERSION}.`);
  }
  if (!document.execution || !Array.isArray(document.execution.source_order) || !document.execution.limits) {
    throw new Error("A flow document requires an explicit execution policy.");
  }
}
