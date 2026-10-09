import type {
  FlowNodeClass as WireFlowNodeClass,
  FlowNode as WireFlowNode,
  FlowNodeDefinition as WireFlowNodeDefinition,
  FlowParameterDataType as WireFlowParameterDataType,
  FlowParameterDefinition as WireFlowParameterDefinition,
  FlowPort as WireFlowPort,
} from "@battersea/flow";
import {
  buildResolvedFlowPort,
  cloneFlowStudioPortNames,
  getFlowPortAlias,
  pruneFlowPortAliases,
  type FlowStudioPortLike,
  type FlowStudioResolvedPort,
} from "./flow-node-ports.js";
import {
  normalizeFlowControllerPortPlacement,
  type FlowControllerPortPlacement,
} from "./flow-controller-port-placement.js";
import {
  applyFlowPortOrder,
  normalizeFlowStudioPortOrder,
  type FlowStudioPortOrder,
} from "./flow-port-order.js";

export type FlowParameterValues = Record<string, unknown>;
export type FlowPortParameterValues = WireFlowNode["port_parameter_values"];

export interface FlowStudioResolvedNodeData extends Record<string, unknown> {
  actionPorts: FlowStudioPortLike[];
  automationPorts: FlowStudioPortLike[];
  controllerPortPlacement?: FlowControllerPortPlacement;
  definitionName: string;
  hasController?: boolean;
  inputPorts: FlowStudioPortLike[];
  instanceName: string;
  longDescription: string;
  nodeClass: WireFlowNodeClass;
  parameterValues: FlowParameterValues;
  portParameterValues?: FlowPortParameterValues;
  portOrder?: FlowStudioPortOrder;
  outputPorts: FlowStudioPortLike[];
  portNames?: WireFlowNode["port_names"];
  signalPorts: FlowStudioPortLike[];
  shortDescription: string;
}

export function createFlowDefinitionLookup(
  definitions: WireFlowNodeDefinition[],
): Record<string, WireFlowNodeDefinition> {
  return Object.fromEntries(
    definitions.map((definition) => [definition.class_name, definition]),
  );
}

export function parameterHasController(
  parameter: WireFlowParameterDefinition,
): boolean {
  return Boolean(parameter.controller);
}

export function definitionHasControllers(
  definition: WireFlowNodeDefinition,
): boolean {
  return (
    (definition.parameters ?? []).some(parameterHasController) ||
    (definition.input_ports ?? []).some((port) =>
      (port.parameters ?? []).some(parameterHasController),
    ) ||
    (definition.output_ports ?? []).some((port) =>
      (port.parameters ?? []).some(parameterHasController),
    ) ||
    (definition.controller_outputs ?? []).length > 0 ||
    (definition.controller_actions ?? []).length > 0
  );
}

export function formatFlowDefinitionTitle(className: string): string {
  return className
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[_-]+/g, " ")
    .trim();
}

export function buildDefaultFlowNodeId(
  className: string,
  nextIndex: number,
): string {
  return `${formatFlowDefinitionTitle(className)
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")}-${nextIndex}`;
}

export function buildDefaultInstanceName(
  className: string,
  nextIndex: number,
): string {
  return `${formatFlowDefinitionTitle(className)}-${nextIndex}`;
}

export function nodeDefinitionIsActivatable(
  definition: WireFlowNodeDefinition,
): boolean {
  return (definition.interfaces ?? []).includes("IFlowNodeActivate");
}

export function getActivationParameterNames(
  definition: WireFlowNodeDefinition,
): string[] {
  return definition.activation_parameters ?? [];
}

export function getPersistedParameters(
  definition: WireFlowNodeDefinition,
): WireFlowParameterDefinition[] {
  const activationParameters = new Set(getActivationParameterNames(definition));
  return (definition.parameters ?? []).filter(
    (parameter) => !activationParameters.has(parameter.name),
  );
}

export function getActivationParameters(
  definition: WireFlowNodeDefinition,
): WireFlowParameterDefinition[] {
  const activationParameters = new Set(getActivationParameterNames(definition));
  return (definition.parameters ?? []).filter((parameter) =>
    activationParameters.has(parameter.name),
  );
}

export function getEffectiveParameterValue(
  definition: WireFlowNodeDefinition,
  parameterName: string,
  parameterValues: FlowParameterValues,
): unknown {
  if (parameterValues[parameterName] !== undefined) {
    return parameterValues[parameterName];
  }

  return (definition.parameters ?? []).find(
    (parameter) => parameter.name === parameterName,
  )?.editor.default_value;
}

export function getEffectivePortParameterValue(options: {
  definition: WireFlowNodeDefinition;
  parameter: WireFlowParameterDefinition;
  parameterValues?: FlowPortParameterValues | null;
  portId: string;
  side: "input" | "output";
}): unknown {
  const explicitValue =
    options.parameterValues?.[options.side]?.[options.portId]?.[
      options.parameter.name
    ];
  if (explicitValue !== undefined) {
    return explicitValue;
  }

  const fixedPorts =
    options.side === "input"
      ? (options.definition.input_ports ?? [])
      : (options.definition.output_ports ?? []);
  const fixedDefault = fixedPorts
    .find((port) => port.name === options.portId)
    ?.parameters?.find((parameter) => parameter.name === options.parameter.name)
    ?.editor.default_value;
  if (fixedDefault !== undefined) {
    return fixedDefault;
  }

  // Fall back to the dynamic-port group's per-port parameter defaults: e.g.
  // Concatenate's expanded `input-0`/`input-1`/... ports each inherit the
  // `array_delimiter` declaration from `dynamic_input_ports[*].parameters`.
  const dynamicGroups =
    options.side === "input"
      ? (options.definition.dynamic_input_ports ?? [])
      : (options.definition.dynamic_output_ports ?? []);
  for (const group of dynamicGroups) {
    if (!portNameMatchesTemplate(options.portId, group.name_template)) continue;
    const dynamicDefault = group.parameters?.find(
      (parameter) => parameter.name === options.parameter.name,
    )?.editor.default_value;
    if (dynamicDefault !== undefined) {
      return dynamicDefault;
    }
  }
  return undefined;
}

function portNameMatchesTemplate(portName: string, template: string): boolean {
  const tokenIndex = template.indexOf("{index}");
  if (tokenIndex < 0) return portName === template;
  const prefix = template.slice(0, tokenIndex);
  const suffix = template.slice(tokenIndex + "{index}".length);
  if (!portName.startsWith(prefix) || !portName.endsWith(suffix)) return false;
  const middle = portName.slice(prefix.length, portName.length - suffix.length);
  return /^\d+$/.test(middle);
}

export function buildDefaultParameterValues(
  definition: WireFlowNodeDefinition,
): FlowParameterValues {
  return Object.fromEntries(
    getPersistedParameters(definition)
      .filter((parameter) => parameter.editor.default_value !== undefined)
      .map((parameter) => [parameter.name, parameter.editor.default_value]),
  );
}

export function expandDefinitionPorts(
  definition: WireFlowNodeDefinition,
  parameterValues: FlowParameterValues,
  side: "input" | "output",
): WireFlowPort[] {
  const fixedPorts =
    side === "input"
      ? (definition.input_ports ?? [])
      : (definition.output_ports ?? []);
  const dynamicGroups =
    side === "input"
      ? (definition.dynamic_input_ports ?? [])
      : (definition.dynamic_output_ports ?? []);
  const ports = [...fixedPorts];

  for (const group of dynamicGroups) {
    const rawCount = getEffectiveParameterValue(
      definition,
      group.count_parameter,
      parameterValues,
    );
    const count =
      typeof rawCount === "number" && Number.isFinite(rawCount) && rawCount >= 0
        ? Math.floor(rawCount)
        : 0;

    for (let index = 0; index < count; index += 1) {
      ports.push({
        accepted_token_types:
          side === "input" ? group.accepted_token_types : undefined,
        kind: side,
        long_description: group.long_description,
        name: group.name_template.replace("{index}", String(index)),
        // Every expanded port inherits the group's per-port parameter
        // declarations so the detail-pane UI can render an editor for each
        // (e.g. Concatenate input ports' `array_delimiter`). Mirrors
        // `expand_ports` in battersea-flow/src/ports.rs.
        parameters: group.parameters ?? [],
        short_description: group.short_description,
        token_type: group.token_type,
      });
    }
  }

  return ports;
}

export function resolveActionPorts(
  definition: WireFlowNodeDefinition,
  parameterValues: FlowParameterValues,
  portNames?: WireFlowNode["port_names"] | null,
): FlowStudioResolvedPort[] {
  return expandDefinitionSignalPorts(definition, parameterValues, "action").map(
    (port) =>
      buildResolvedFlowPort({
        displayClass: port.display_class,
        id: port.name,
        longDescription: port.long_description,
        name: getFlowPortAlias(portNames, "action", port.name),
        nodeClass: definition.kind,
        shortDescription: port.short_description,
        side: "action",
      }),
  );
}

export function resolveSignalPorts(
  definition: WireFlowNodeDefinition,
  parameterValues: FlowParameterValues,
  portNames?: WireFlowNode["port_names"] | null,
): FlowStudioResolvedPort[] {
  return expandDefinitionSignalPorts(definition, parameterValues, "signal").map(
    (port) =>
      buildResolvedFlowPort({
        displayClass: port.display_class,
        id: port.name,
        longDescription: port.long_description,
        name: getFlowPortAlias(portNames, "signal", port.name),
        nodeClass: definition.kind,
        shortDescription: port.short_description,
        side: "signal",
      }),
  );
}

export function resolveAutomationPorts(
  definition: WireFlowNodeDefinition,
  portNames?: WireFlowNode["port_names"] | null,
): FlowStudioResolvedPort[] {
  return (definition.automation_ports ?? []).map((port) =>
    buildResolvedFlowPort({
      acceptedTokenTypes: port.accepted_token_types,
      displayClass: port.display_class,
      id: port.name,
      longDescription: port.long_description,
      name: getFlowPortAlias(portNames, "automation", port.name),
      nodeClass: definition.kind,
      shortDescription: port.short_description,
      side: "automation",
      tokenType: port.token_type,
    }),
  );
}

function expandDefinitionSignalPorts(
  definition: WireFlowNodeDefinition,
  parameterValues: FlowParameterValues,
  side: "action" | "signal",
): Array<{
  display_class?: "inline" | "sink" | "source";
  long_description?: string;
  name: string;
  short_description?: string;
}> {
  const fixedPorts =
    side === "action"
      ? (definition.action_ports ?? [])
      : (definition.signal_ports ?? []);
  const dynamicGroups =
    side === "action"
      ? (definition.dynamic_action_ports ?? [])
      : (definition.dynamic_signal_ports ?? []);
  const ports = [...fixedPorts];

  for (const group of dynamicGroups) {
    const rawCount = getEffectiveParameterValue(
      definition,
      group.count_parameter,
      parameterValues,
    );
    const count =
      typeof rawCount === "number" && Number.isFinite(rawCount) && rawCount >= 0
        ? Math.floor(rawCount)
        : 0;

    for (let index = 0; index < count; index += 1) {
      ports.push({
        display_class: group.display_class,
        long_description: group.long_description,
        name: group.name_template.replace("{index}", String(index)),
        short_description: group.short_description,
      });
    }
  }

  return ports;
}

export function resolveInputPorts(
  definition: WireFlowNodeDefinition,
  parameterValues: FlowParameterValues,
  portNames?: WireFlowNode["port_names"] | null,
): FlowStudioResolvedPort[] {
  return expandDefinitionPorts(definition, parameterValues, "input").map(
    (port) =>
      buildResolvedFlowPort({
        acceptedTokenTypes: port.accepted_token_types,
        displayClass: port.display_class,
        id: port.name,
        longDescription: port.long_description,
        name: getFlowPortAlias(portNames, "input", port.name),
        nodeClass: definition.kind,
        parameters: port.parameters,
        shortDescription: port.short_description,
        side: "input",
        tokenType: port.token_type,
      }),
  );
}

export function resolveOutputPorts(
  definition: WireFlowNodeDefinition,
  parameterValues: FlowParameterValues,
  portNames?: WireFlowNode["port_names"] | null,
): FlowStudioResolvedPort[] {
  return expandDefinitionPorts(definition, parameterValues, "output").map(
    (port) =>
      buildResolvedFlowPort({
        displayClass: port.display_class,
        id: port.name,
        longDescription: port.long_description,
        name: getFlowPortAlias(portNames, "output", port.name),
        nodeClass: definition.kind,
        parameters: port.parameters,
        shortDescription: port.short_description,
        side: "output",
        tokenType: port.token_type,
      }),
  );
}

export function prunePortNamesForResolvedPorts(options: {
  actionPorts: readonly FlowStudioResolvedPort[];
  automationPorts: readonly FlowStudioResolvedPort[];
  inputPorts: readonly FlowStudioResolvedPort[];
  outputPorts: readonly FlowStudioResolvedPort[];
  portNames?: WireFlowNode["port_names"] | null;
  signalPorts: readonly FlowStudioResolvedPort[];
}): WireFlowNode["port_names"] | undefined {
  const actionIds = options.actionPorts.map((port) => port.id);
  const automationIds = options.automationPorts.map((port) => port.id);
  const inputIds = options.inputPorts.map((port) => port.id);
  const outputIds = options.outputPorts.map((port) => port.id);
  const signalIds = options.signalPorts.map((port) => port.id);

  return pruneFlowPortAliases(
    pruneFlowPortAliases(
      pruneFlowPortAliases(
        pruneFlowPortAliases(
          pruneFlowPortAliases(options.portNames, "action", actionIds),
          "automation",
          automationIds,
        ),
        "input",
        inputIds,
      ),
      "output",
      outputIds,
    ),
    "signal",
    signalIds,
  );
}

export function formatFlowPortLabel(portName: string): string {
  return portName
    .replace(/[_-]+/g, " ")
    .replace(/\b\w/g, (character) => character.toUpperCase())
    .trim();
}

export function formatFlowParameterLabel(parameterName: string): string {
  return parameterName
    .replace(/[_-]+/g, " ")
    .replace(/\b\w/g, (character) => character.toUpperCase())
    .trim();
}

export function resolveParameterEditorValue(
  definition: WireFlowNodeDefinition,
  parameter: WireFlowParameterDefinition,
  parameterValues: FlowParameterValues,
): unknown {
  return getEffectiveParameterValue(
    definition,
    parameter.name,
    parameterValues,
  );
}

export function isStringListParameter(
  parameter: WireFlowParameterDefinition,
): boolean {
  return (
    parameter.datatype.kind === "list" &&
    (parameter.datatype.item_type as WireFlowParameterDataType).kind ===
      "string"
  );
}

export function isSupportedFlowParameter(
  parameter: WireFlowParameterDefinition,
): boolean {
  switch (parameter.editor.kind) {
    case "boolean":
    case "enum":
    case "input_port_count":
    case "output_port_count":
    case "unsigned":
    case "string":
    case "text":
      return true;
    case "list":
      return isStringListParameter(parameter);
    default:
      return false;
  }
}

export function getParameterOptions(
  parameter: WireFlowParameterDefinition,
): string[] {
  return parameter.editor.values ?? [];
}

export function getNodeClassLabel(nodeClass: WireFlowNodeClass): string {
  return nodeClass.charAt(0).toUpperCase() + nodeClass.slice(1);
}

export function buildResolvedNodeData(options: {
  controllerPortPlacement?: FlowControllerPortPlacement | null;
  definition: WireFlowNodeDefinition;
  instanceName: string;
  portOrder?: WireFlowNode["port_order"] | null;
  parameterValues: FlowParameterValues;
  portParameterValues?: FlowPortParameterValues | null;
  portNames?: WireFlowNode["port_names"] | null;
}): FlowStudioResolvedNodeData {
  const actionPorts = applyFlowPortOrder(
    resolveActionPorts(
      options.definition,
      options.parameterValues,
      options.portNames,
    ),
    "action",
    options.portOrder,
  );
  const automationPorts = applyFlowPortOrder(
    resolveAutomationPorts(options.definition, options.portNames),
    "automation",
    options.portOrder,
  );
  const inputPorts = applyFlowPortOrder(
    resolveInputPorts(
      options.definition,
      options.parameterValues,
      options.portNames,
    ),
    "input",
    options.portOrder,
  );
  const outputPorts = applyFlowPortOrder(
    resolveOutputPorts(
      options.definition,
      options.parameterValues,
      options.portNames,
    ),
    "output",
    options.portOrder,
  );
  const signalPorts = applyFlowPortOrder(
    resolveSignalPorts(
      options.definition,
      options.parameterValues,
      options.portNames,
    ),
    "signal",
    options.portOrder,
  );
  const portNames = prunePortNamesForResolvedPorts({
    actionPorts,
    automationPorts,
    inputPorts,
    outputPorts,
    portNames: cloneFlowStudioPortNames(options.portNames),
    signalPorts,
  });
  const portOrder = normalizeFlowStudioPortOrder({
    actionPorts,
    automationPorts,
    inputPorts,
    outputPorts,
    portOrder: options.portOrder,
    signalPorts,
  });

  return {
    actionPorts,
    automationPorts,
    controllerPortPlacement: normalizeFlowControllerPortPlacement(
      options.controllerPortPlacement,
    ),
    definitionName: options.definition.class_name,
    hasController: definitionHasControllers(options.definition),
    inputPorts,
    instanceName:
      options.instanceName.trim() ||
      formatFlowDefinitionTitle(options.definition.class_name),
    longDescription: options.definition.long_description,
    nodeClass: options.definition.kind,
    parameterValues: { ...options.parameterValues },
    portParameterValues: cloneFlowPortParameterValues(
      options.portParameterValues,
    ),
    portOrder,
    outputPorts,
    portNames,
    signalPorts,
    shortDescription: options.definition.short_description,
  };
}

export function cloneFlowPortParameterValues(
  portParameterValues: FlowPortParameterValues | null | undefined,
): FlowPortParameterValues | undefined {
  if (!portParameterValues) {
    return undefined;
  }

  const cloneSide = (side?: Record<string, Record<string, unknown>>) => {
    if (!side) {
      return undefined;
    }
    const entries = Object.entries(side)
      .map(([portId, values]) => [portId, { ...values }] as const)
      .filter(([, values]) => Object.keys(values).length > 0);
    return entries.length > 0 ? Object.fromEntries(entries) : undefined;
  };
  const clone = {
    input: cloneSide(portParameterValues.input),
    output: cloneSide(portParameterValues.output),
  } satisfies FlowPortParameterValues;

  return clone.input || clone.output ? clone : undefined;
}
