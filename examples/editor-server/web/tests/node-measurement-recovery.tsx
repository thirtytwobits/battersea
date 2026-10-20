/** Copyright (c) Scott A Dixon */
import React from "react";
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { ReactFlow, addEdge, applyNodeChanges, type Edge, type Node } from "@xyflow/react";
import {
  FlowStudioCanvasNode,
  type FlowStudioCanvasNodeData,
} from "@battersea/editor/core/flow-canvas-node";
import "@xyflow/react/dist/style.css";
import "../style.css";

declare global {
  interface Window {
    holdNodeResizeNotifications?: boolean;
  }
}

const nodeTypes = { flowStudio: FlowStudioCanvasNode };
type CanvasNode = Node<FlowStudioCanvasNodeData, "flowStudio">;
const data: FlowStudioCanvasNodeData = {
  actionPorts: [],
  automationPorts: [],
  controllerPortPlacement: "default",
  definitionName: "External",
  hasController: false,
  inputPorts: ["input"],
  instanceName: "Node",
  longDescription: "",
  nodeClass: "inline",
  outputPorts: ["output"],
  parameterValues: {},
  signalPorts: [],
  shortDescription: "",
};
const initialNodes: CanvasNode[] = [
  { id: "source", type: "flowStudio", position: { x: 0, y: 0 }, data },
  { id: "target", type: "flowStudio", position: { x: 350, y: 0 }, data },
  { id: "branch", type: "flowStudio", position: { x: 350, y: 230 }, data },
];

function MeasurementRecovery() {
  const [nodes, setNodes] = React.useState(initialNodes);
  const [moveBranchOnConnect, setMoveBranchOnConnect] = React.useState(false);
  const [edges, setEdges] = React.useState<Edge[]>([{
    id: "existing",
    source: "source",
    target: "target",
    sourceHandle: "output-0",
    targetHandle: "input-0",
  }]);
  const restoreMeasurements = () => {
    const measured = new Map(nodes.map(node => [node.id, node.measured]));
    window.holdNodeResizeNotifications = true;
    // Reinitialise controlled nodes, then restore dimensions before another resize delivery.
    flushSync(() => setNodes(current => current.map(({ measured, ...node }) => node)));
    flushSync(() => setNodes(current => current.map(node => ({
      ...node,
      measured: measured.get(node.id),
    }))));
  };
  return (
    <>
      <button onClick={restoreMeasurements}>Restore measured nodes</button>
      <label>
        <input
          type="checkbox"
          checked={moveBranchOnConnect}
          onChange={event => setMoveBranchOnConnect(event.target.checked)}
        />
        Move branch when connecting
      </label>
      <div style={{ width: 900, height: 600 }}>
        <ReactFlow<CanvasNode>
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          onNodesChange={changes => setNodes(current => applyNodeChanges(changes, current))}
          onConnectStart={() => {
            if (moveBranchOnConnect) {
              flushSync(() => setNodes(current => current.map(node =>
                node.id === "branch" ? { ...node, position: { x: 500, y: 380 } } : node)));
            }
          }}
          onConnect={connection => setEdges(current => addEdge(connection, current))}
        />
      </div>
    </>
  );
}

createRoot(document.getElementById("root")!).render(<MeasurementRecovery />);
