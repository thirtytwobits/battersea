/** Copyright (c) Scott A Dixon */
import React from "react";
import { canEmitNodeFailure, type FlowFailurePolicy, type FlowNodeDefinition, type FlowParameterDefinition } from "@battersea/flow";
import { groupControlsForDetailPane } from "../core/flow-control-layout.js";
import { formatFlowParameterLabel } from "../core/flow-node-definitions.js";
import {
  FlowParameterEditor,
  type FlowParameterRenderer,
} from "./flow-parameter-control.js";
type Group = ReturnType<typeof groupControlsForDetailPane>[number];
export interface FlowInspectorProps {
  failurePolicy?: FlowFailurePolicy;
  failureDefinition?: FlowNodeDefinition;
  onFailurePolicyChange?: (policy: FlowFailurePolicy) => void;
  parameters: readonly FlowParameterDefinition[];
  values?: Readonly<Record<string, unknown>>;
  onChange?: (parameter: FlowParameterDefinition, value: unknown) => void;
  renderers?: readonly FlowParameterRenderer[];
  renderGroup?: (group: Group) => React.ReactNode | undefined;
  renderParameter?: (
    parameter: FlowParameterDefinition,
    group: Group,
  ) => React.ReactNode;
}
/** Grouping and renderer selection are shared; hosts own product-specific sections and framing. */
export function FlowInspector({
  failurePolicy = "fail",
  failureDefinition,
  onFailurePolicyChange,
  parameters,
  values = {},
  onChange,
  renderers,
  renderGroup,
  renderParameter,
}: FlowInspectorProps) {
  return (
    <>
      {onFailurePolicyChange && (
        <label>
          Failure policy
          <select aria-label="Failure policy" value={failurePolicy}
            onChange={event => {
              const value = event.target.value;
              if (value === "fail" || value === "emit" || value === "skip") onFailurePolicyChange(value);
            }}>
            <option value="fail">Fail activation</option>
            <option value="emit" disabled={!failureDefinition || !canEmitNodeFailure(failureDefinition)}>Emit error</option>
            <option value="skip">Skip remaining work</option>
          </select>
        </label>
      )}
      {groupControlsForDetailPane([...parameters]).map((group) => {
        const key = group.parameters
          .map((parameter) => parameter.name)
          .join(":");
        const custom = renderGroup?.(group);
        if (custom !== undefined)
          return <React.Fragment key={key}>{custom}</React.Fragment>;
        return (
          <div
            key={key}
            className={
              group.layout === "row"
                ? "flow-studio-detail-pane__cardinality-row"
                : "authoring-inspector__stack flow-studio-detail-pane__control-stack"
            }
          >
            {group.parameters.map((parameter) => (
              <React.Fragment key={parameter.name}>
                {renderParameter ? (
                  renderParameter(parameter, group)
                ) : (
                  <section>
                    <h3>{formatFlowParameterLabel(parameter.name)}</h3>
                    <FlowParameterEditor
                      parameter={parameter}
                      value={
                        values[parameter.name] ?? parameter.editor.default_value
                      }
                      onChange={(value) => onChange?.(parameter, value)}
                      options={(parameter.editor.values ?? []).map((value) => ({
                        label: value,
                        value,
                      }))}
                      renderers={renderers}
                    />
                  </section>
                )}
              </React.Fragment>
            ))}
          </div>
        );
      })}
    </>
  );
}
