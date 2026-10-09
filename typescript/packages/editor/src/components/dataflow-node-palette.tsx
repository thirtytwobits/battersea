/** Copyright (c) Scott A Dixon */
import React from "react";
import type { FlowNodeDefinition } from "@battersea/flow";
import {
  buildDataflowNodePaletteGroups,
  type DataflowNodePaletteEntry,
} from "../core/dataflow-node-palette-state.js";
import {
  buildFlowNodePaletteDragId,
  createFlowNodeDragPayload,
  serialiseFlowNodeDragPayload,
} from "../core/flow-drag.js";
export interface FlowPaletteModel {
  groups: ReturnType<typeof buildDataflowNodePaletteGroups>;
  dragId: (entry: DataflowNodePaletteEntry) => string;
  dragData: (entry: DataflowNodePaletteEntry) => {
    flowNodeDragPayload: string;
    nodeDefinition: FlowNodeDefinition;
  };
}
export function DataflowNodePalette({
  nodeDefinitions,
  render,
  onAdd,
}: {
  nodeDefinitions: readonly FlowNodeDefinition[];
  render?: (model: FlowPaletteModel) => React.ReactNode;
  onAdd?: (definition: FlowNodeDefinition) => void;
}): React.JSX.Element {
  const groups = React.useMemo(
    () => buildDataflowNodePaletteGroups(nodeDefinitions),
    [nodeDefinitions],
  );
  const model: FlowPaletteModel = {
    groups,
    dragId: (entry) => buildFlowNodePaletteDragId(entry.definition.class_name),
    dragData: (entry) => ({
      flowNodeDragPayload: serialiseFlowNodeDragPayload(
        createFlowNodeDragPayload(entry.definition),
      ),
      nodeDefinition: entry.definition,
    }),
  };
  return (
    <>
      {render ? (
        render(model)
      ) : (
        <nav aria-label="Node types">
          {groups.map((group) => (
            <section key={group.id}>
              <h3>{group.label}</h3>
              {group.items.map((entry) => (
                <button
                  type="button"
                  data-node-kind={entry.variant}
                  key={entry.variant}
                  onClick={() => onAdd?.(entry.definition)}
                  title={entry.description}
                >
                  {entry.title}
                </button>
              ))}
            </section>
          ))}
        </nav>
      )}
    </>
  );
}
