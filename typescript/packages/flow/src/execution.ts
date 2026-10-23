import type { FlowDocument, FlowExecutionPolicy, FlowFailurePolicy, FlowNodeDefinition } from "./contract.js";
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

export function requireFlowFailurePolicy(value: unknown): FlowFailurePolicy {
  if (value === undefined) return "fail";
  if (value === "fail" || value === "emit" || value === "skip") return value;
  throw new Error("Failure policy must be fail, emit or skip.");
}

/** Mirrors the callback phases validated by the Rust execution contract. */
export function canEmitNodeFailure(definition: FlowNodeDefinition): boolean {
  const phases: Array<"snapshot" | "execution"> = [];
  if (definition.kind === "source" || definition.kind === "hybrid") phases.push("snapshot");
  if (definition.kind !== "source"
    || definition.output_ports.some(port => port.phase === "execution")
    || definition.dynamic_output_ports.some(port => port.phase === "execution")
    || definition.action_ports.some(port => port.name !== "disable")
    || Boolean(definition.dynamic_action_ports?.length)
    || Boolean(definition.controller_actions?.length)) phases.push("execution");
  return phases.every(phase => definition.failure_ports?.filter(port => port.phase === phase).length === 1);
}
