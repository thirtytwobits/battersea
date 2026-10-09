/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises the dataflow canvas shell wiring and stable React Flow registration.
 */
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { DataflowCanvas } from "@battersea/editor/components/dataflow-canvas";

const NO_OP_GESTURE_DRIVER = {
  beginInteractiveGesture: () => undefined,
  cancelInteractiveGesture: () => undefined,
  commitInteractiveGesture: () => undefined
} as const;

function renderDataflowCanvas(overrides: Partial<React.ComponentProps<typeof DataflowCanvas>> = {}): string {
  return renderToStaticMarkup(
    <DataflowCanvas
      edgeGestureDriver={NO_OP_GESTURE_DRIVER}
      edgeActivationState={{}}
      edges={[]}
      flowInstanceRef={{ current: null }}
      isConnectionValid={() => true}
      movedPortPulse={null}
      nodeContextMenu={null}
      nodes={[]}
      onConnect={() => undefined}
      onConnectEnd={() => undefined}
      onEdgesChange={() => undefined}
      onInsertEdgeBridge={() => undefined}
      onInsertEdgeWaypoint={() => undefined}
      onMovePort={() => undefined}
      onNodeClick={() => undefined}
      onNodeContextMenu={() => undefined}
      onNodeDragStart={() => undefined}
      onNodeDragStop={() => undefined}
      onNodesChange={() => undefined}
      onPaneClick={() => undefined}
      onRemoveNode={() => undefined}
      onSelectEdgeBridge={() => undefined}
      onSelectEdgeWaypoint={() => undefined}
      onUpdateEdgeBridgeGap={() => undefined}
      onUpdateEdgeBridgePosition={() => undefined}
      onUpdateEdgeWaypointHandle={() => undefined}
      onUpdateEdgeWaypointPosition={() => undefined}
      {...overrides}
    />
  );
}

test("DataflowCanvas preserves the existing pane, canvas, and controls shell", () => {
  const html = renderDataflowCanvas();

  assert.match(html, /class="authoring-graph-canvas-pane flow-studio-flow-pane"/);
  assert.match(html, /class="react-flow authoring-graph-canvas flow-studio-canvas light"/);
  assert.match(html, /react-flow__controls/);
  assert.doesNotMatch(html, /authoring-graph-canvas__auto-layout-toggle/);
  assert.doesNotMatch(html, /zoom in/i);
  assert.doesNotMatch(html, /zoom out/i);
});

test("DataflowCanvas still renders the node context menu overlay with the existing class names", () => {
  const html = renderDataflowCanvas({
    nodeContextMenu: {
      nodeId: "node-14",
      x: 120,
      y: 180
    }
  });

  assert.match(html, /flow-studio-node-context-menu/);
  assert.match(html, /flow-studio-node-context-menu__action/);
  assert.match(html, />Remove</);
});

test("DataflowCanvas does not emit the React Flow nodeTypes warning during repeated static renders", () => {
  const originalWarn = console.warn;
  const warnings: string[] = [];

  console.warn = (...args: unknown[]) => {
    warnings.push(args.map((arg) => String(arg)).join(" "));
  };

  try {
    renderDataflowCanvas();
    renderDataflowCanvas({
      movedPortPulse: {
        nodeId: "node-14",
        portId: "output",
        replay: "a",
        side: "output"
      },
      selectedPort: {
        kind: "port",
        nodeId: "node-14",
        portId: "output",
        side: "output"
      }
    });
  } finally {
    console.warn = originalWarn;
  }

  assert.deepEqual(
    warnings.filter((warning) => warning.includes("React Flow") && warning.includes("nodeTypes or edgeTypes")),
    []
  );
});
