/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises dataflow structural snapshot comparison and restore behaviour
 * backed by react-amnesia.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { type Amnesia, createAmnesiaStore } from "react-amnesia";

import type { FlowStudioEdge, FlowStudioNode } from "@battersea/editor/core/dataflow-editor-state";
import {
  resolveDataflowEdgeBridgeInsertion,
  resolveDataflowEdgeWaypointInsertion
} from "@battersea/editor/core/dataflow-edge-interactions";
import {
  type DataflowStructuralSnapshot,
  type DataflowUndoHistoryState,
  areDataflowStructuralSnapshotsExactlyEqual,
  areDataflowStructuralSnapshotsEqual,
  buildDataflowStructuralSnapshot,
  buildDataflowUndoHistoryState,
  commitDataflowSnapshotChange,
  pushDataflowPresentSnapshotChange,
  restoreDataflowStructuralWorkspace
} from "@battersea/editor/core/dataflow-undo";
import { buildDefaultFlowWorkspace } from "@battersea/editor/core/flow-persistence";
import type { FlowStudioWorkspaceState } from "@battersea/editor/core/flow-persistence";

function buildNode(overrides: Omit<Partial<FlowStudioNode>, "data"> & {
  data?: Partial<FlowStudioNode["data"]>;
} = {}): FlowStudioNode {
  const {
    data: dataOverrides,
    ...nodeOverrides
  } = overrides;

  return {
    className: "flow-studio-node flow-studio-node--inline",
    data: {
      actionPorts: [],
      automationPorts: [],
      controllerPortPlacement: "default",
      definitionName: "ChatAPI",
      hasController: false,
      inputPorts: ["input"],
      instanceName: "Chat API-1",
      longDescription: "Talks to the model.",
      nodeClass: "inline",
      outputPorts: ["output"],
      parameterValues: {
        prompt: "hello"
      },
      portOrder: undefined,
      portNames: undefined,
      signalPorts: [],
      shortDescription: "Talk to the model",
      ...dataOverrides
    },
    id: "chat-api-1",
    position: {
      x: 120,
      y: 240
    },
    type: "flowStudio",
    ...nodeOverrides
  };
}

function buildEdge(overrides: Omit<Partial<FlowStudioEdge>, "data"> & {
  data?: Partial<NonNullable<FlowStudioEdge["data"]>>;
} = {}): FlowStudioEdge {
  const {
    data: dataOverrides,
    ...edgeOverrides
  } = overrides;

  return {
    data: {
      bridges: undefined,
      kind: "token",
      order: 1,
      waypoints: undefined,
      ...dataOverrides
    },
    id: "chat-api-1-output-0-render-1-input-0-1",
    source: "chat-api-1",
    sourceHandle: "output-0",
    target: "render-1",
    targetHandle: "input-0",
    ...edgeOverrides
  };
}

interface AuthoringHarness {
  amnesia: Amnesia;
  getLastCaptured: () => DataflowUndoHistoryState;
  getWorkspace: () => FlowStudioWorkspaceState;
  recordPush: (next: FlowStudioWorkspaceState, restoreLayout: boolean) => void;
  restoreState: (state: DataflowUndoHistoryState) => void;
  setLiveWorkspace: (next: FlowStudioWorkspaceState) => void;
}

function createHarness(initial: FlowStudioWorkspaceState): AuthoringHarness {
  const amnesia = createAmnesiaStore({ capacity: 50 });
  let workspace = initial;
  let lastCaptured: DataflowUndoHistoryState = buildDataflowUndoHistoryState(
    buildDataflowStructuralSnapshot(initial),
    false
  );
  // Updated after a recorded push: workspace and amnesia are in sync.
  const recordPush = (next: FlowStudioWorkspaceState, restoreLayout: boolean) => {
    workspace = next;
    lastCaptured = buildDataflowUndoHistoryState(
      buildDataflowStructuralSnapshot(next),
      restoreLayout
    );
  };
  // Updated for live mutations that bypass the undo path (e.g. node drag).
  // The lastCaptured marker stays put so the next commit absorbs the
  // mutation into the new entry's undo target.
  const setLiveWorkspace = (next: FlowStudioWorkspaceState) => {
    workspace = next;
  };
  const restoreState = (historyState: DataflowUndoHistoryState) => {
    workspace = restoreDataflowStructuralWorkspace({
      currentWorkspace: workspace,
      preserveLayout: !historyState.restoreLayout,
      snapshot: historyState.snapshot
    });
    lastCaptured = historyState;
  };
  return {
    amnesia,
    getLastCaptured: () => lastCaptured,
    getWorkspace: () => workspace,
    recordPush,
    restoreState,
    setLiveWorkspace
  };
}

function readLastPastLabel(amnesia: Amnesia): string | undefined {
  const snapshot = amnesia.getSnapshot();
  return snapshot.past[snapshot.past.length - 1]?.label;
}

test("areDataflowStructuralSnapshotsEqual ignores layout-only node and edge geometry changes", () => {
  const baseline = buildDataflowStructuralSnapshot({
    edges: [buildEdge()],
    nodes: [buildNode()]
  });
  const layoutOnly = buildDataflowStructuralSnapshot({
    edges: [buildEdge({
      data: {
        bridges: [{
          gap: 24,
          segmentIndex: 0,
          t: 0.6
        }],
        waypoints: [{
          inHandle: { x: 20, y: 20 },
          outHandle: { x: 30, y: 30 },
          position: { x: 25, y: 25 }
        }]
      }
    })],
    nodes: [buildNode({
      data: {
        controllerPortPlacement: "swapped"
      },
      position: {
        x: 640,
        y: 128
      }
    })]
  });

  assert.equal(areDataflowStructuralSnapshotsEqual(baseline, layoutOnly), true);
});

test("restoreDataflowStructuralWorkspace preserves the current node position while undoing a rename", () => {
  const currentWorkspace = {
    ...buildDefaultFlowWorkspace(),
    nodes: [buildNode({
      data: {
        instanceName: "Chat API Renamed"
      },
      position: {
        x: 900,
        y: 420
      }
    })]
  };
  const targetSnapshot = buildDataflowStructuralSnapshot({
    edges: [],
    nodes: [buildNode({
      data: {
        instanceName: "Chat API-1"
      },
      position: {
        x: 120,
        y: 240
      }
    })]
  });

  const restored = restoreDataflowStructuralWorkspace({
    currentWorkspace,
    snapshot: targetSnapshot
  });

  assert.equal(restored.nodes[0]?.data.instanceName, "Chat API-1");
  assert.deepEqual(restored.nodes[0]?.position, {
    x: 900,
    y: 420
  });
});

test("restoreDataflowStructuralWorkspace restores deleted nodes at their captured position", () => {
  const restored = restoreDataflowStructuralWorkspace({
    currentWorkspace: buildDefaultFlowWorkspace(),
    snapshot: buildDataflowStructuralSnapshot({
      edges: [],
      nodes: [buildNode({
        position: {
          x: 512,
          y: 96
        }
      })]
    })
  });

  assert.equal(restored.nodes.length, 1);
  assert.deepEqual(restored.nodes[0]?.position, {
    x: 512,
    y: 96
  });
});

test("restoreDataflowStructuralWorkspace preserves current waypoints and bridges for unchanged edges", () => {
  const currentWorkspace = {
    ...buildDefaultFlowWorkspace(),
    edges: [buildEdge({
      data: {
        bridges: [{
          gap: 32,
          segmentIndex: 0,
          t: 0.7
        }],
        kind: "token",
        order: 2,
        waypoints: [{
          inHandle: { x: 15, y: 15 },
          outHandle: { x: 25, y: 25 },
          position: { x: 20, y: 20 }
        }]
      }
    })],
    nodes: [
      buildNode(),
      buildNode({
        data: {
          definitionName: "Render",
          inputPorts: ["input"],
          instanceName: "Render-1",
          outputPorts: [],
          shortDescription: "Render output"
        },
        id: "render-1"
      })
    ],
    selectedTarget: {
      kind: "node",
      nodeId: "chat-api-1"
    } as const
  };
  const targetSnapshot = buildDataflowStructuralSnapshot({
    edges: [buildEdge({
      data: {
        kind: "token",
        order: 1,
        waypoints: undefined,
        bridges: undefined
      }
    })],
    nodes: currentWorkspace.nodes
  });

  const restored = restoreDataflowStructuralWorkspace({
    currentWorkspace,
    snapshot: targetSnapshot
  });

  assert.deepEqual(restored.edges[0]?.data?.waypoints, currentWorkspace.edges[0]?.data?.waypoints);
  assert.deepEqual(restored.edges[0]?.data?.bridges, currentWorkspace.edges[0]?.data?.bridges);
  assert.deepEqual(restored.selectedTarget, { kind: "none" });
});

test("commitDataflowSnapshotChange can record exact layout-only changes when requested", () => {
  const harness = createHarness({
    ...buildDefaultFlowWorkspace(),
    nodes: [buildNode()]
  });

  const nextWorkspace = commitDataflowSnapshotChange({
    amnesia: harness.amnesia,
    compareSnapshots: areDataflowStructuralSnapshotsExactlyEqual,
    label: "Move node",
    previousState: harness.getLastCaptured(),
    restoreLayout: true,
    restoreState: harness.restoreState,
    transform: (currentWorkspace) => ({
      ...currentWorkspace,
      nodes: currentWorkspace.nodes.map((node) => ({
        ...node,
        position: {
          x: node.position.x + 200,
          y: node.position.y + 80
        }
      }))
    }),
    workspace: harness.getWorkspace()
  });
  harness.recordPush(nextWorkspace, true);

  assert.deepEqual(nextWorkspace.nodes[0]?.position, {
    x: 320,
    y: 320
  });
  assert.equal(harness.amnesia.getSnapshot().canUndo, true);
  assert.equal(readLastPastLabel(harness.amnesia), "Move node");
});

test("pushDataflowPresentSnapshotChange restores the pre-drag snapshot before pushing the completed layout step", async () => {
  const startWorkspace: FlowStudioWorkspaceState = {
    ...buildDefaultFlowWorkspace(),
    nodes: [buildNode()]
  };
  const draggedWorkspace: FlowStudioWorkspaceState = {
    ...startWorkspace,
    nodes: startWorkspace.nodes.map((node) => ({
      ...node,
      position: {
        x: 640,
        y: 128
      }
    }))
  };
  const harness = createHarness(startWorkspace);
  // Live drag updates the workspace without touching the history.
  harness.setLiveWorkspace(draggedWorkspace);

  assert.equal(pushDataflowPresentSnapshotChange({
    amnesia: harness.amnesia,
    label: "Move node",
    previousSnapshot: buildDataflowStructuralSnapshot(startWorkspace),
    restoreState: harness.restoreState,
    workspace: harness.getWorkspace()
  }), true);

  await harness.amnesia.undo();
  assert.deepEqual(harness.getWorkspace().nodes[0]?.position, {
    x: 120,
    y: 240
  });

  await harness.amnesia.redo();
  assert.deepEqual(harness.getWorkspace().nodes[0]?.position, {
    x: 640,
    y: 128
  });
});

test("a completed node drag can be undone back to its drag-start position", async () => {
  const startWorkspace: FlowStudioWorkspaceState = {
    ...buildDefaultFlowWorkspace(),
    nodes: [buildNode()]
  };
  const midDragWorkspace: FlowStudioWorkspaceState = {
    ...startWorkspace,
    nodes: startWorkspace.nodes.map((node) => ({
      ...node,
      position: {
        x: 420,
        y: 300
      }
    }))
  };
  const finalDragWorkspace: FlowStudioWorkspaceState = {
    ...startWorkspace,
    nodes: startWorkspace.nodes.map((node) => ({
      ...node,
      position: {
        x: 640,
        y: 128
      }
    }))
  };
  const harness = createHarness(startWorkspace);

  // Mid-drag and final-drag arrive as live workspace mutations (no push).
  harness.setLiveWorkspace(midDragWorkspace);
  harness.setLiveWorkspace(finalDragWorkspace);

  assert.equal(pushDataflowPresentSnapshotChange({
    amnesia: harness.amnesia,
    label: "Move node",
    previousSnapshot: buildDataflowStructuralSnapshot(startWorkspace),
    restoreState: harness.restoreState,
    workspace: harness.getWorkspace()
  }), true);

  await harness.amnesia.undo();
  assert.deepEqual(harness.getWorkspace().nodes[0]?.position, {
    x: 120,
    y: 240
  });
});

test("commitDataflowSnapshotChange records bridge insertions as undoable layout changes", async () => {
  const baseWorkspace: FlowStudioWorkspaceState = {
    ...buildDefaultFlowWorkspace(),
    edges: [buildEdge()],
    nodes: [
      buildNode(),
      buildNode({
        data: {
          definitionName: "Render",
          inputPorts: ["input"],
          instanceName: "Render-1",
          outputPorts: [],
          shortDescription: "Render output"
        },
        id: "render-1"
      })
    ]
  };
  const harness = createHarness(baseWorkspace);

  const nextWorkspace = commitDataflowSnapshotChange({
    amnesia: harness.amnesia,
    compareSnapshots: areDataflowStructuralSnapshotsExactlyEqual,
    label: "Add bridge",
    previousState: harness.getLastCaptured(),
    restoreLayout: true,
    restoreState: harness.restoreState,
    transform: (currentWorkspace) => (
      resolveDataflowEdgeBridgeInsertion(currentWorkspace, buildEdge().id, {
        gap: 24,
        segmentIndex: 0,
        t: 0.5
      }).workspace
    ),
    workspace: harness.getWorkspace()
  });
  harness.recordPush(nextWorkspace, true);

  assert.deepEqual(nextWorkspace.edges[0]?.data?.bridges, [{
    gap: 24,
    segmentIndex: 0,
    t: 0.5
  }]);
  assert.equal(harness.amnesia.getSnapshot().canUndo, true);
  assert.equal(readLastPastLabel(harness.amnesia), "Add bridge");

  await harness.amnesia.undo();
  assert.equal(harness.getWorkspace().edges[0]?.data?.bridges, undefined);
});

test("commitDataflowSnapshotChange records waypoint insertions as undoable layout changes", async () => {
  const baseWorkspace: FlowStudioWorkspaceState = {
    ...buildDefaultFlowWorkspace(),
    edges: [buildEdge()],
    nodes: [
      buildNode(),
      buildNode({
        data: {
          definitionName: "Render",
          inputPorts: ["input"],
          instanceName: "Render-1",
          outputPorts: [],
          shortDescription: "Render output"
        },
        id: "render-1"
      })
    ]
  };
  const harness = createHarness(baseWorkspace);

  const nextWorkspace = commitDataflowSnapshotChange({
    amnesia: harness.amnesia,
    compareSnapshots: areDataflowStructuralSnapshotsExactlyEqual,
    label: "Add waypoint",
    previousState: harness.getLastCaptured(),
    restoreLayout: true,
    restoreState: harness.restoreState,
    transform: (currentWorkspace) => (
      resolveDataflowEdgeWaypointInsertion(currentWorkspace, buildEdge().id, 0, 0.5, {
        inHandle: { x: 220, y: 140 },
        outHandle: { x: 320, y: 180 },
        position: { x: 270, y: 160 }
      }).workspace
    ),
    workspace: harness.getWorkspace()
  });
  harness.recordPush(nextWorkspace, true);

  assert.deepEqual(nextWorkspace.edges[0]?.data?.waypoints, [{
    inHandle: { x: 220, y: 140 },
    outHandle: { x: 320, y: 180 },
    position: { x: 270, y: 160 }
  }]);
  assert.equal(harness.amnesia.getSnapshot().canUndo, true);
  assert.equal(readLastPastLabel(harness.amnesia), "Add waypoint");

  await harness.amnesia.undo();
  assert.equal(harness.getWorkspace().edges[0]?.data?.waypoints, undefined);
});

test("restoreDataflowStructuralWorkspace restores exact layout when requested", () => {
  const restored = restoreDataflowStructuralWorkspace({
    currentWorkspace: {
      ...buildDefaultFlowWorkspace(),
      nodes: [buildNode({
        position: {
          x: 900,
          y: 420
        }
      })]
    },
    preserveLayout: false,
    snapshot: buildDataflowStructuralSnapshot({
      edges: [],
      nodes: [buildNode({
        position: {
          x: 120,
          y: 240
        }
      })]
    })
  });

  assert.deepEqual(restored.nodes[0]?.position, {
    x: 120,
    y: 240
  });
});

test("commitDataflowSnapshotChange undo of a structural change preserves the current live layout", async () => {
  const startWorkspace: FlowStudioWorkspaceState = {
    ...buildDefaultFlowWorkspace(),
    nodes: [buildNode()]
  };
  const harness = createHarness(startWorkspace);

  // First a structural change (e.g. add an edge) — undo of this should
  // restore the structure but preserve any current live layout.
  const renamedWorkspace = commitDataflowSnapshotChange({
    amnesia: harness.amnesia,
    compareSnapshots: areDataflowStructuralSnapshotsEqual,
    label: "Rename node",
    previousState: harness.getLastCaptured(),
    restoreLayout: false,
    restoreState: harness.restoreState,
    transform: (currentWorkspace) => ({
      ...currentWorkspace,
      nodes: currentWorkspace.nodes.map((node) => ({
        ...node,
        data: {
          ...node.data,
          instanceName: "Renamed"
        }
      }))
    }),
    workspace: harness.getWorkspace()
  });
  harness.recordPush(renamedWorkspace, false);

  // Now the user moves the node live (no push).
  const liveLayoutWorkspace: FlowStudioWorkspaceState = {
    ...renamedWorkspace,
    nodes: renamedWorkspace.nodes.map((node) => ({
      ...node,
      position: { x: 800, y: 500 }
    }))
  };
  harness.setLiveWorkspace(liveLayoutWorkspace);

  // Undo the rename. The structure should revert; the live layout should
  // stay because the structural undo target had `restoreLayout: false`.
  await harness.amnesia.undo();
  assert.equal(harness.getWorkspace().nodes[0]?.data.instanceName, "Chat API-1");
  assert.deepEqual(harness.getWorkspace().nodes[0]?.position, {
    x: 800,
    y: 500
  });
});
