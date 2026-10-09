/** Copyright (c) Scott A Dixon */
import React from "react";
import type { FlowParameterDefinition } from "@battersea/flow";
import {
  isSupportedFlowParameter,
  formatFlowParameterLabel,
} from "../core/flow-node-definitions.js";
import {
  buildCardinalityOptions,
  isFixedCardinality,
  resolveCardinalityValue,
} from "../core/dataflow-cardinality.js";
import {
  resolveMultiSelectValue,
  resolveNumberControlValue,
  resolveTextControlValue,
} from "../core/parameter-controls.js";
import {
  flowListUsesSingleSelect,
  normaliseSingleSelectValue,
  resolveSingleSelectValue,
} from "../core/flow-list-control.js";
import { flowEnumUsesSwitch } from "../core/flow-enum-switch.js";
export interface ParameterOption {
  label: React.ReactNode;
  value: string;
  disabled?: boolean;
}
interface ChoiceProps {
  ariaLabel: string;
  disabled?: boolean;
  fullWidth?: boolean;
  minSegmentWidthPx?: number;
  onChange: (value: string) => void;
  options: readonly [ParameterOption, ...ParameterOption[]];
  value: string;
  wrap?: boolean;
}
export interface ParameterPresentation {
  Field: React.ComponentType<{
    as?: "label";
    className?: string;
    compact?: boolean;
    label?: React.ReactNode;
    children: React.ReactNode;
  }>;
  Select: React.ComponentType<React.SelectHTMLAttributes<HTMLSelectElement>>;
  Switch: React.ComponentType<
    Omit<ChoiceProps, "options"> & {
      options: [ParameterOption, ParameterOption];
    }
  >;
  Segments: React.ComponentType<ChoiceProps>;
}
function NativeChoice({
  ariaLabel,
  disabled,
  onChange,
  options,
  value,
}: ChoiceProps) {
  return (
    <select
      aria-label={ariaLabel}
      disabled={disabled}
      onChange={(event) => onChange(event.target.value)}
      value={value}
    >
      {options.map((option) => (
        <option
          key={option.value}
          value={option.value}
          disabled={option.disabled}
        >
          {option.label}
        </option>
      ))}
    </select>
  );
}
export const nativeParameterPresentation: ParameterPresentation = {
  Field: ({ children, className, label }) => (
    <label className={className}>
      {label}
      {children}
    </label>
  ),
  Select: (props) => <select {...props} />,
  Switch: NativeChoice,
  Segments: NativeChoice,
};
export interface FlowParameterControlProps {
  parameter: FlowParameterDefinition;
  value: unknown;
  onChange: (value: unknown) => void;
  options?: readonly ParameterOption[];
  presentation?: ParameterPresentation;
  compact?: boolean;
  heading?: string;
  fieldLabel?: React.ReactNode;
  disabled?: boolean;
  help?: React.ReactNode;
  onUnsupportedInteraction?: () => void;
}
/** Custom source editors are supplied by the host's renderer before this primitive control. */
export function FlowParameterControl({
  parameter,
  value,
  onChange,
  options = [],
  presentation = nativeParameterPresentation,
  compact = false,
  heading = formatFlowParameterLabel(parameter.name),
  fieldLabel,
  disabled,
  help,
  onUnsupportedInteraction,
}: FlowParameterControlProps): React.JSX.Element {
  const { Field, Select, Switch, Segments } = presentation;
  const field = {
    as: "label" as const,
    className: "flow-studio-detail-pane__field",
    label: fieldLabel,
  };
  if (!isSupportedFlowParameter(parameter))
    return (
      <Field {...field}>
        <input disabled value="Unsupported parameter type" readOnly />
      </Field>
    );
  if (parameter.editor.kind === "boolean")
    return (
      <div className="flow-studio-detail-pane__query-control">
        {help}
        <Switch
          ariaLabel={heading}
          disabled={disabled}
          fullWidth
          onChange={(next) => onChange(next === "enabled")}
          options={[
            { label: "Disabled", value: "disabled" },
            { label: "Enabled", value: "enabled" },
          ]}
          value={value === true ? "enabled" : "disabled"}
        />
      </div>
    );
  if (parameter.editor.kind === "enum") {
    if (
      flowEnumUsesSwitch(
        options.map((option) => ({
          label: String(option.label),
          value: option.value,
        })),
      )
    )
      return (
        <Switch
          ariaLabel={heading}
          disabled={disabled}
          fullWidth
          onChange={onChange}
          options={[options[0]!, options[1]!]}
          value={
            resolveTextControlValue(value) === options[1]!.value
              ? options[1]!.value
              : options[0]!.value
          }
        />
      );
    if (options.length === 0)
      return (
        <Field {...field}>
          <input disabled readOnly value="No options available" />
        </Field>
      );
    return (
      <Segments
        ariaLabel={heading}
        disabled={disabled}
        fullWidth
        minSegmentWidthPx={110}
        onChange={onChange}
        options={options as [ParameterOption, ...ParameterOption[]]}
        value={resolveTextControlValue(value)}
        wrap
      />
    );
  }
  if (parameter.editor.kind === "list")
    return (
      <Field {...field}>
        {flowListUsesSingleSelect(parameter) ? (
          <Select
            aria-label={heading}
            disabled={disabled}
            onClick={onUnsupportedInteraction}
            value={resolveSingleSelectValue(value)}
            onChange={(event) =>
              onChange(normaliseSingleSelectValue(event.target.value))
            }
          >
            <option value="">None</option>
            {options.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </Select>
        ) : (
          <Select
            aria-label={heading}
            disabled={disabled}
            multiple
            onClick={onUnsupportedInteraction}
            value={resolveMultiSelectValue(value)}
            onChange={(event) =>
              onChange(
                Array.from(
                  event.target.selectedOptions,
                  (option) => option.value,
                ),
              )
            }
          >
            {options.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </Select>
        )}
      </Field>
    );
  if (parameter.editor.kind === "string" || parameter.editor.kind === "text")
    return (
      <Field {...field}>
        <textarea
          aria-label={heading}
          disabled={disabled}
          onChange={(event) => onChange(event.target.value)}
          rows={parameter.editor.kind === "text" ? 10 : 4}
          value={resolveTextControlValue(value)}
        />
      </Field>
    );
  if (
    !["unsigned", "input_port_count", "output_port_count"].includes(
      parameter.editor.kind,
    )
  )
    return (
      <Field {...field}>
        <input
          disabled
          readOnly
          value="A host parameter renderer is required"
        />
      </Field>
    );
  const min = Number(parameter.editor.min ?? 0);
  const max = parameter.editor.max ?? null;
  return (
    <Field
      {...field}
      compact={compact}
      className={`${field.className} ${compact ? "flow-studio-detail-pane__field--compact" : ""}`}
      label={compact ? "Count" : fieldLabel}
    >
      {max !== null ? (
        <Select
          aria-label={heading}
          disabled={disabled || isFixedCardinality(min, max)}
          value={resolveCardinalityValue(value, min)}
          onChange={(event) =>
            onChange(Number.parseInt(event.target.value, 10))
          }
        >
          {buildCardinalityOptions(min, max).map((option) => (
            <option key={option} value={option}>
              {option}
            </option>
          ))}
        </Select>
      ) : (
        <input
          aria-label={heading}
          disabled={disabled}
          type="number"
          min={parameter.editor.min ?? undefined}
          value={resolveNumberControlValue(value, min)}
          onChange={(event) =>
            onChange(resolveNumberControlValue(event.target.value, min))
          }
        />
      )}
    </Field>
  );
}
export type FlowParameterRenderer = (
  props: FlowParameterControlProps,
) => React.ReactNode | undefined;
/** Undefined delegates to the built-in control; null intentionally hides a host-rendered field. */
export function FlowParameterEditor({
  renderers = [],
  ...props
}: FlowParameterControlProps & {
  renderers?: readonly FlowParameterRenderer[];
}) {
  for (const render of renderers) {
    const rendered = render(props);
    if (rendered !== undefined) return <>{rendered}</>;
  }
  return <FlowParameterControl {...props} />;
}
