/** Copyright (c) Scott A Dixon */
import type {
  FlowEdge,
  FlowExecutionPolicy,
  FlowQueueLimits,
} from "@battersea/flow";
import { resolveFlowStudioResolvedPorts } from "./flow-node-ports.js";
import type { FlowStudioWorkspaceState } from "./flow-persistence.js";

/** Source priority is independent of canvas layout and serialisation order. */
export function reconcileFlowSourceOrder(
  policy: FlowExecutionPolicy,
  nodes: FlowStudioWorkspaceState["nodes"],
): FlowExecutionPolicy {
  const sources = nodes
    .filter(
      (node) =>
        node.data.nodeClass === "source" || node.data.nodeClass === "hybrid",
    )
    .map((node) => node.id);
  const order = policy.source_order.filter((id) => sources.includes(id));
  for (const id of sources) if (!order.includes(id)) order.push(id);
  return { ...structuredClone(policy), source_order: order };
}

export function moveFlowSource(
  policy: FlowExecutionPolicy,
  id: string,
  direction: -1 | 1,
): FlowExecutionPolicy {
  const index = policy.source_order.indexOf(id);
  const target = index + direction;
  if (index < 0 || target < 0 || target >= policy.source_order.length)
    return policy;
  const order = [...policy.source_order];
  [order[index], order[target]] = [order[target], order[index]];
  return { ...policy, source_order: order };
}

/** Mirrors the capacity constraints in contracts/execution.md; server validation is authoritative. */
export function validateFlowExecutionSettings(
  policy: FlowExecutionPolicy,
  edges: readonly Pick<FlowEdge, "id" | "order" | "queue">[],
): string[] {
  const errors: string[] = [];
  const integer = (value: number, label: string, minimum = 1) => {
    if (!Number.isInteger(value) || value < minimum || value > 0xffffffff)
      errors.push(
        `${label} must be a whole number from ${minimum} to 4294967295.`,
      );
  };
  const limits = policy.limits;
  integer(limits.pending_events, "Pending events");
  integer(limits.retained_bytes, "Activation retained bytes");
  integer(limits.node_retained_bytes, "Node retained bytes");
  if (limits.node_retained_bytes > limits.retained_bytes)
    errors.push(
      "Node retained bytes must fit within activation retained bytes.",
    );
  const queue = (value: FlowQueueLimits, label: string) => {
    integer(value.items, `${label} items`);
    integer(value.bytes, `${label} bytes`);
    integer(value.max_event_bytes, `${label} maximum event bytes`);
    if (value.max_event_bytes > value.bytes)
      errors.push(
        `${label} maximum event bytes must fit within its queue bytes.`,
      );
    if (value.items > limits.pending_events)
      errors.push(`${label} items must fit within pending events.`);
    if (value.bytes > limits.retained_bytes)
      errors.push(`${label} bytes must fit within activation retained bytes.`);
    if (value.policy !== "backpressure" && value.policy !== "drop_oldest")
      errors.push(`${label} has an unsupported overflow policy.`);
  };
  queue(limits.provider_queue, "Provider queue");
  if (limits.provider_queue.policy !== "backpressure")
    errors.push("Provider queues require backpressure.");
  for (const edge of edges) {
    integer(edge.order, `Connection ${edge.id} order`, 0);
    if (edge.queue) queue(edge.queue, `Connection ${edge.id}`);
  }
  return errors;
}

export interface FlowExecutionConnection {
  id: string;
  label: string;
  mode: string;
  sourcePhase: string;
  targetPhase: string;
}

export function buildFlowExecutionConnections(
  workspace: FlowStudioWorkspaceState,
  edges: readonly FlowEdge[],
): FlowExecutionConnection[] {
  return edges.map((edge) => {
    const source = workspace.nodes.find(
      (node) => node.id === edge.source_node_id,
    );
    const target = workspace.nodes.find(
      (node) => node.id === edge.target_node_id,
    );
    const output = resolveFlowStudioResolvedPorts(
      source?.data.outputPorts ?? [],
      "output",
    ).find((port) => port.id === edge.source_port);
    const input = [
      ...resolveFlowStudioResolvedPorts(target?.data.inputPorts ?? [], "input"),
      ...resolveFlowStudioResolvedPorts(
        target?.data.automationPorts ?? [],
        "automation",
      ),
    ].find((port) => port.id === edge.target_port);
    return {
      id: edge.id,
      label: `${source?.data.instanceName ?? edge.source_node_id}.${edge.source_port} → ${target?.data.instanceName ?? edge.target_node_id}.${edge.target_port} (${edge.id})`,
      mode: edge.kind === "signal" ? "signal" : (output?.mode ?? "unknown"),
      sourcePhase:
        edge.kind === "signal" ? "execution" : (output?.phase ?? "unknown"),
      targetPhase:
        edge.kind === "signal" ? "execution" : (input?.phase ?? "unknown"),
    };
  });
}
