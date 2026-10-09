/**
 * Copyright (c) Scott A Dixon
 *
 * Shapes grouped palette data for the dataflow workspace.
 */
import type { FlowNodeClass, FlowNodeDefinition } from "@battersea/flow";

interface AuthoringPaletteEntryBase {
  badges?: readonly {
    classifier: FlowNodeClass | "controller";
    icon?: string;
    label: string;
    shape?: "icon" | "pill";
    title?: string;
  }[];
  description: string;
  groupId: string;
  groupLabel: string;
  title: string;
  variant: string;
}
function buildAuthoringPaletteGroups(
  entries: readonly DataflowNodePaletteEntry[],
) {
  const groups = new Map<
    string,
    { id: string; label: string; items: DataflowNodePaletteEntry[] }
  >();
  for (const entry of entries) {
    const group = groups.get(entry.groupId) ?? {
      id: entry.groupId,
      label: entry.groupLabel,
      items: [],
    };
    group.items.push(entry);
    groups.set(group.id, group);
  }
  return [...groups.values()];
}
import {
  definitionHasControllers,
  formatFlowDefinitionTitle,
  getNodeClassLabel,
} from "./flow-node-definitions.js";

const FLOW_NODE_PALETTE_GROUP_ORDER: readonly FlowNodeClass[] = [
  "source",
  "control",
  "instrument",
  "logic",
  "hybrid",
  "inline",
  "sink",
] as const;

export interface DataflowNodePaletteEntry extends AuthoringPaletteEntryBase {
  definition: FlowNodeDefinition;
}

export function buildDataflowNodePaletteGroups(
  nodeDefinitions: readonly FlowNodeDefinition[],
) {
  return buildAuthoringPaletteGroups(
    buildDataflowNodePaletteEntries(nodeDefinitions),
  );
}

export function buildDataflowNodePaletteEntries(
  nodeDefinitions: readonly FlowNodeDefinition[],
): DataflowNodePaletteEntry[] {
  return FLOW_NODE_PALETTE_GROUP_ORDER.flatMap((nodeClass) =>
    nodeDefinitions
      .filter((definition) => definition.kind === nodeClass)
      .map(buildDataflowNodePaletteEntry),
  );
}

export function buildDataflowNodePaletteEntry(
  definition: FlowNodeDefinition,
): DataflowNodePaletteEntry {
  return {
    badges: [
      {
        classifier: definition.kind,
        label: getNodeClassLabel(definition.kind),
      },
      ...(definitionHasControllers(definition)
        ? [
            {
              classifier: "controller" as const,
              icon: "window",
              label: "Controller",
              shape: "icon" as const,
              title: "Controller",
            },
          ]
        : []),
    ],
    definition,
    description: definition.short_description,
    groupId: definition.kind,
    groupLabel: getNodeClassLabel(definition.kind),
    title: formatFlowDefinitionTitle(definition.class_name),
    variant: definition.class_name,
  };
}
