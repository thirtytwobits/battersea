/** Copyright (c) Scott A Dixon */
import React from "react";
import type { FlowNodeDefinition } from "@battersea/flow";
import {
  DataflowNodePalette,
  FlowInspector,
  AuthoringGraphCanvas,
  FlowStudioCanvasNode,
  AuthoringEditableEdgeView,
} from "@battersea/editor";
import {
  createDroppedFlowNode,
  createFlowNodeDragPayload,
} from "@battersea/editor/core/flow-drag";
import type { ReactFlowInstance } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
const definition: FlowNodeDefinition = {
  class_name: "Example",
  handler_id: "example.echo",
  kind: "inline",
  short_description: "A custom node",
  long_description: "A node registered by an application.",
  input_ports: [{ kind: "input", name: "input", token_type: "text" }],
  output_ports: [{ kind: "output", name: "output", token_type: "text" }],
  action_ports: [],
  signal_ports: [],
  interfaces: [],
  activation_parameters: [],
  dynamic_input_ports: [],
  dynamic_output_ports: [],
  parameters: [
    {
      name: "message",
      datatype: { kind: "string" },
      editor: { kind: "string", default_value: "Authored text" },
    },
    {
      name: "enabled",
      datatype: { kind: "boolean" },
      editor: { kind: "boolean", default_value: true },
    },
  ],
};
export default { title: "Battersea/Editor" };
export const Palette = {
  render: () => <DataflowNodePalette nodeDefinitions={[definition]} />,
};
export const Inspector = {
  render: () => <FlowInspector parameters={definition.parameters} />,
};
function Graph({ connected = false }: { connected?: boolean }) {
  const ref = React.useRef<ReactFlowInstance | null>(null);
  const first = createDroppedFlowNode({
    payload: createFlowNodeDragPayload(definition),
    nextIndex: 1,
    position: { x: 60, y: 100 },
  });
  const second = createDroppedFlowNode({
    payload: createFlowNodeDragPayload(definition),
    nextIndex: 2,
    position: { x: 470, y: 100 },
  });
  return (
    <div style={{ height: 430 }}>
      <AuthoringGraphCanvas
        nodes={connected ? [first, second] : [first]}
        nodeTypes={{
          flowStudio: FlowStudioCanvasNode,
          AuthoringEditableEdgeView,
        }}
        edgeTypes={{ editable: AuthoringEditableEdgeView }}
        edges={
          connected
            ? [
                {
                  type: "editable",
                  data: { directionMarker: true },
                  id: "delivery",
                  source: first.id,
                  target: second.id,
                  sourceHandle: "output-0",
                  targetHandle: "input-0",
                },
              ]
            : []
        }
        flowInstanceRef={ref}
        reactFlowProps={{ fitView: true }}
      />
    </div>
  );
}
export const Node = { render: () => <Graph /> };
export const Edge = { render: () => <Graph connected /> };
