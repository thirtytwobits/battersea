/** Copyright (c) Scott A Dixon */
import React from "react";
import type {
  FlowDocument,
  FlowEdge,
  FlowExecutionPolicy,
  FlowQueueLimits,
} from "@battersea/flow";
import {
  moveFlowSource,
  validateFlowExecutionSettings,
  type FlowExecutionConnection,
} from "../core/flow-execution-settings.js";
import {
  nativeParameterPresentation,
  type ParameterPresentation,
} from "./flow-parameter-control.js";

type SettingsPresentation = Pick<ParameterPresentation, "Field" | "Select"> & {
  Button?: React.ComponentType<React.ButtonHTMLAttributes<HTMLButtonElement>>;
};
export interface FlowExecutionSettingsProps {
  document: FlowDocument;
  connections: readonly FlowExecutionConnection[];
  onExecutionChange: (execution: FlowExecutionPolicy) => void;
  onConnectionChange: (
    id: string,
    change: Pick<FlowEdge, "order" | "queue">,
  ) => void;
  presentation?: SettingsPresentation;
}
const queueFields = [
  ["items", "Queue items"],
  ["bytes", "Queue bytes"],
  ["max_event_bytes", "Maximum event bytes"],
] as const;
const limitFields = [
  ["pending_events", "Pending events"],
  ["retained_bytes", "Activation retained bytes"],
  ["node_retained_bytes", "Node retained bytes"],
] as const;
const number = (value: string) => (value.trim() === "" ? NaN : Number(value));
function queueDraft(queue: FlowQueueLimits) {
  return Object.fromEntries(
    queueFields.map(([key]) => [key, String(queue[key])]),
  ) as Record<(typeof queueFields)[number][0], string>;
}
function readQueue(
  draft: ReturnType<typeof queueDraft>,
  policy: FlowQueueLimits["policy"],
): FlowQueueLimits {
  return {
    items: number(draft.items),
    bytes: number(draft.bytes),
    max_event_bytes: number(draft.max_event_bytes),
    policy,
  };
}
function NumberField({
  label,
  value,
  onChange,
  presentation,
  minimum = 1,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  presentation: SettingsPresentation;
  minimum?: number;
}) {
  const { Field } = presentation;
  return (
    <Field label={label}>
      <input
        aria-label={label}
        type="number"
        min={minimum}
        max={0xffffffff}
        step={1}
        required
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </Field>
  );
}
function Errors({ errors }: { errors: readonly string[] }) {
  return errors.length ? (
    <ul role="alert" data-execution-state="invalid">
      {errors.map((error, i) => (
        <li key={i}>{error}</li>
      ))}
    </ul>
  ) : null;
}
function LimitsForm({
  document,
  onExecutionChange,
  presentation,
}: FlowExecutionSettingsProps & { presentation: SettingsPresentation }) {
  const Button = presentation.Button ?? "button";
  const limits = document.execution.limits;
  const [draft, setDraft] = React.useState(() =>
    Object.fromEntries(limitFields.map(([key]) => [key, String(limits[key])])),
  );
  const [queue, setQueue] = React.useState(() =>
    queueDraft(limits.provider_queue),
  );
  const [errors, setErrors] = React.useState<string[]>([]);
  return (
    <form
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        const execution = {
          ...document.execution,
          limits: {
            pending_events: number(draft.pending_events),
            retained_bytes: number(draft.retained_bytes),
            node_retained_bytes: number(draft.node_retained_bytes),
            provider_queue: readQueue(queue, limits.provider_queue.policy),
          },
        };
        const issues = validateFlowExecutionSettings(execution, document.edges);
        setErrors(issues);
        if (!issues.length) onExecutionChange(execution);
      }}
    >
      <h3>Activation limits</h3>
      {limitFields.map(([key, label]) => (
        <NumberField
          key={key}
          label={label}
          value={draft[key]}
          presentation={presentation}
          onChange={(value) => setDraft({ ...draft, [key]: value })}
        />
      ))}
      <h3>Provider queue</h3>
      <p>Backpressure waits for capacity without dropping events.</p>
      {queueFields.map(([key, label]) => (
        <NumberField
          key={key}
          label={`Provider ${label.toLowerCase()}`}
          value={queue[key]}
          presentation={presentation}
          onChange={(value) => setQueue({ ...queue, [key]: value })}
        />
      ))}
      <Errors errors={errors} />
      <Button type="submit">Apply activation limits</Button>
    </form>
  );
}
function ConnectionForm({
  document,
  edge,
  connection,
  onConnectionChange,
  presentation,
}: FlowExecutionSettingsProps & {
  edge: FlowEdge;
  connection: FlowExecutionConnection;
  presentation: SettingsPresentation;
}) {
  const [order, setOrder] = React.useState(String(edge.order));
  const [queue, setQueue] = React.useState(() =>
    edge.queue ? queueDraft(edge.queue) : null,
  );
  const [policy, setPolicy] = React.useState(
    edge.queue?.policy ?? "backpressure",
  );
  const [errors, setErrors] = React.useState<string[]>([]);
  const { Field, Select } = presentation;
  const Button = presentation.Button ?? "button";
  return (
    <form
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        const updated = {
          ...edge,
          order: number(order),
          queue: queue ? readQueue(queue, policy) : undefined,
        };
        const issues = validateFlowExecutionSettings(
          document.execution,
          document.edges.map((candidate) =>
            candidate.id === edge.id ? updated : candidate,
          ),
        );
        setErrors(issues);
        if (!issues.length)
          onConnectionChange(edge.id, {
            order: updated.order,
            queue: updated.queue,
          });
      }}
    >
      <p data-execution-mode={connection.mode}>
        Mode: {connection.mode.replaceAll("_", " ")}
        <br />
        Phase: {connection.sourcePhase} → {connection.targetPhase}
      </p>
      <NumberField
        label="Connection order"
        value={order}
        onChange={setOrder}
        presentation={presentation}
        minimum={0}
      />
      <p>Fan-out uses ascending order, then connection ID.</p>
      {queue ? (
        <>
          {queueFields.map(([key, label]) => (
            <NumberField
              key={key}
              label={label}
              value={queue[key]}
              presentation={presentation}
              onChange={(value) => setQueue({ ...queue, [key]: value })}
            />
          ))}
          <Field label="Overflow policy">
            <Select
              aria-label="Overflow policy"
              value={policy}
              onChange={(event) =>
                setPolicy(event.target.value as FlowQueueLimits["policy"])
              }
            >
              <option value="backpressure">Backpressure</option>
              <option value="drop_oldest">Drop oldest (lossy)</option>
            </Select>
          </Field>
          <p data-execution-policy={policy}>
            {policy === "drop_oldest"
              ? "When full, discard the oldest queued stream events to make space."
              : "Wait for capacity without dropping stream events."}
          </p>
        </>
      ) : null}
      <Errors errors={errors} />
      <Button type="submit">Apply connection settings</Button>
    </form>
  );
}

/** Editable scheduler policy with catalogue-owned mode and phase displayed read-only. */
export function FlowExecutionSettings(props: FlowExecutionSettingsProps) {
  const { document, connections, onExecutionChange } = props;
  const presentation: SettingsPresentation = props.presentation ?? nativeParameterPresentation;
  const [selectedId, setSelectedId] = React.useState("");
  const connection =
    connections.find((item) => item.id === selectedId) ?? connections[0];
  const edge = document.edges.find((item) => item.id === connection?.id);
  const { Field, Select } = presentation;
  const Button = presentation.Button ?? "button";
  return (
    <details className="battersea-execution-settings">
      <summary>Execution settings</summary>
      <h3>Source priority</h3>
      <p>Snapshot dependencies take precedence over this order.</p>
      <ol aria-label="Source priority">
        {document.execution.source_order.map((id, index, order) => {
          const label = `${document.nodes.find((node) => node.id === id)?.instance_name ?? id} (${id})`;
          return (
            <li key={id} data-source-id={id}>
              <span>{label}</span>
              <div>
                <Button
                  type="button"
                  aria-label={`Move ${label} earlier`}
                  disabled={index === 0}
                  onClick={() =>
                    onExecutionChange(
                      moveFlowSource(document.execution, id, -1),
                    )
                  }
                >
                  Earlier
                </Button>
                <Button
                  type="button"
                  aria-label={`Move ${label} later`}
                  disabled={index === order.length - 1}
                  onClick={() =>
                    onExecutionChange(moveFlowSource(document.execution, id, 1))
                  }
                >
                  Later
                </Button>
              </div>
            </li>
          );
        })}
      </ol>
      <LimitsForm
        key={JSON.stringify([document.flow_key, document.execution.limits])}
        {...props}
        presentation={presentation}
      />
      <h3>Connections</h3>
      {connection && edge ? (
        <>
          <Field label="Connection">
            <Select
              aria-label="Connection"
              value={connection.id}
              onChange={(event) => setSelectedId(event.target.value)}
            >
              {connections.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.label}
                </option>
              ))}
            </Select>
          </Field>
          <ConnectionForm
            key={JSON.stringify([document.flow_key, edge])}
            {...props}
            edge={edge}
            connection={connection}
            presentation={presentation}
          />
        </>
      ) : (
        <p>Add a connection to set its delivery order and queue.</p>
      )}
    </details>
  );
}
