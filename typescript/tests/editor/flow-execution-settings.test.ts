/** Copyright (c) Scott A Dixon */
import assert from "node:assert/strict";
import test from "node:test";
import {
  createFlowExecutionPolicy,
  requireFlowFailurePolicy,
  canEmitNodeFailure,
  type FlowDocument,
  type FlowNodeDefinition,
} from "@battersea/flow";
import {
  validateFlowExecutionSettings,
  moveFlowSource,
} from "@battersea/editor/core/flow-execution-settings";
import {
  buildFlowWorkspaceFromDocument,
  buildFlowSaveDocument,
  buildFlowValidationDocument,
  normalizeFlowWorkspaceState,
  removeNodeFromWorkspace,
  buildFlowDocumentFromWorkspace,
} from "@battersea/editor/core/flow-persistence";
import {
  buildDataflowStructuralSnapshot,
  areDataflowStructuralSnapshotsEqual,
  restoreDataflowStructuralWorkspace,
} from "@battersea/editor/core/dataflow-undo";
import { reconcileFlowWorkspaceDefinitions } from "@battersea/editor/core/flow-persistence";

const definitions: FlowNodeDefinition[] = [
  {
    class_name: "Source",
    handler_id: "test.source",
    kind: "source",
    interfaces: [],
    short_description: "",
    long_description: "",
    parameters: [],
    activation_parameters: [],
    input_ports: [],
    output_ports: [
      {
        name: "out",
        kind: "output",
        token_type: "text",
        mode: "stream",
        phase: "execution",
      },
    ],
    action_ports: [],
    signal_ports: [],
    dynamic_input_ports: [],
    dynamic_output_ports: [],
  },
  {
    class_name: "Sink",
    handler_id: "test.sink",
    kind: "sink",
    interfaces: [],
    short_description: "",
    long_description: "",
    parameters: [],
    activation_parameters: [],
    input_ports: [
      {
        name: "in",
        kind: "input",
        token_type: "text",
        mode: "stream",
        phase: "execution",
      },
    ],
    output_ports: [],
    action_ports: [],
    signal_ports: [],
    dynamic_input_ports: [],
    dynamic_output_ports: [],
  },
];
function fixture(): FlowDocument {
  const execution = createFlowExecutionPolicy(["z", "a"]);
  return {
    version: 3,
    flow_key: "policy",
    title: "Policy",
    execution,
    nodes: ["a", "z", "sink"].map((id) => ({
      id,
      instance_name: "Duplicate title",
      definition_name: id === "sink" ? "Sink" : "Source",
      parameter_values: {},
    })),
    edges: [
      {
        id: "stream",
        kind: "token",
        source_node_id: "z",
        source_port: "out",
        target_node_id: "sink",
        target_port: "in",
        order: 0,
        queue: { ...execution.limits.provider_queue, policy: "drop_oldest" },
      },
    ],
  };
}

test("node failure policy survives draft, undo, redo, catalogue reconciliation and save/reload", () => {
  const document = fixture();
  document.nodes[0].failure_policy = "skip";
  const workspace = buildFlowWorkspaceFromDocument({ document, definitions });
  const original = buildDataflowStructuralSnapshot(workspace);
  workspace.nodes[0].data.failurePolicy = "emit";
  const edited = buildDataflowStructuralSnapshot(workspace);
  assert.equal(areDataflowStructuralSnapshotsEqual(original, edited), false);
  const undone = restoreDataflowStructuralWorkspace({ currentWorkspace: workspace, snapshot: original });
  assert.equal(undone.nodes[0].data.failurePolicy, "skip");
  const redone = restoreDataflowStructuralWorkspace({ currentWorkspace: undone, snapshot: edited });
  const draft = normalizeFlowWorkspaceState(JSON.parse(JSON.stringify(redone)));
  const reconciled = reconcileFlowWorkspaceDefinitions(draft, Object.fromEntries(definitions.map(d => [d.class_name,d])));
  const saved = buildFlowSaveDocument({workspace: reconciled})!;
  assert.equal(saved.nodes.find(n => n.id === document.nodes[0].id)?.failure_policy,"emit");
  const loaded = buildFlowWorkspaceFromDocument({document:saved,definitions});
  assert.equal(loaded.nodes.find(n => n.id === document.nodes[0].id)?.data.failurePolicy,"emit");
  assert.equal(buildFlowValidationDocument({workspace:loaded}).nodes.find(n=>n.id===document.nodes[0].id)?.failure_policy,"emit");
});

test("failure authoring validates policy values and both phases of a source", () => {
  for (const policy of ["fail","emit","skip"] as const) assert.equal(requireFlowFailurePolicy(policy),policy);
  assert.equal(requireFlowFailurePolicy(undefined),"fail");
  for (const invalid of [null,"retry",{},1]) assert.throws(()=>requireFlowFailurePolicy(invalid));
  const source = structuredClone(definitions[0]);
  assert.equal(canEmitNodeFailure(source),false);
  source.failure_ports = [{name:"snapshot_error",phase:"snapshot"}];
  assert.equal(canEmitNodeFailure(source),false);
  source.failure_ports.push({name:"error",phase:"execution"});
  assert.equal(canEmitNodeFailure(source),true);
  const workspace = buildFlowWorkspaceFromDocument({document:fixture(),definitions:[source,definitions[1]]});
  assert.ok(workspace.nodes[0].data.outputPorts.some(port=>typeof port!=="string" && port.id==="error"));
  const corrupt = JSON.parse(JSON.stringify(workspace));
  corrupt.nodes[0].data.failurePolicy = "retry";
  assert.throws(()=>normalizeFlowWorkspaceState(corrupt));
});

test("source priority moves by identity independently of node array order and titles", () => {
  const document = fixture();
  const moved = moveFlowSource(document.execution, "a", -1);
  assert.deepEqual(
    moved.source_order,
    [...document.execution.source_order].reverse(),
  );
  assert.equal(moveFlowSource(moved, "a", -1), moved);
  assert.equal(moveFlowSource(moved, "missing", 1), moved);
});

test("execution limits enforce finite integer capacities and cross-budget constraints", () => {
  const document = fixture();
  assert.deepEqual(
    validateFlowExecutionSettings(document.execution, document.edges),
    [],
  );
  for (const value of [0, -1, 1.5, NaN, Infinity, 2 ** 32]) {
    for (const key of [
      "pending_events",
      "retained_bytes",
      "node_retained_bytes",
    ] as const) {
      const policy = structuredClone(document.execution);
      policy.limits[key] = value;
      assert.ok(
        validateFlowExecutionSettings(policy, document.edges).length,
        `${key}: ${value}`,
      );
    }
    for (const key of ["items", "bytes", "max_event_bytes"] as const) {
      const policy = structuredClone(document.execution);
      policy.limits.provider_queue[key] = value;
      assert.ok(
        validateFlowExecutionSettings(policy, document.edges).length,
        `${key}: ${value}`,
      );
      const edges = structuredClone(document.edges);
      edges[0].queue![key] = value;
      assert.ok(
        validateFlowExecutionSettings(document.execution, edges).length,
        `edge ${key}: ${value}`,
      );
    }
  }
  for (const change of [
    (p: typeof document.execution) => {
      p.limits.node_retained_bytes = p.limits.retained_bytes + 1;
    },
    (p: typeof document.execution) => {
      p.limits.provider_queue.bytes = p.limits.retained_bytes + 1;
    },
    (p: typeof document.execution) => {
      p.limits.provider_queue.items = p.limits.pending_events + 1;
    },
    (p: typeof document.execution) => {
      p.limits.provider_queue.max_event_bytes =
        p.limits.provider_queue.bytes + 1;
    },
    (p: typeof document.execution) => {
      p.limits.provider_queue.policy = "drop_oldest";
    },
  ]) {
    const policy = structuredClone(document.execution);
    change(policy);
    assert.ok(validateFlowExecutionSettings(policy, document.edges).length);
  }
  for (const key of ["items", "bytes", "max_event_bytes"] as const) {
    const edges = structuredClone(document.edges);
    edges[0].queue![key] = document.execution.limits.retained_bytes + 1;
    assert.ok(validateFlowExecutionSettings(document.execution, edges).length);
  }
});

test("edited execution and stream queues survive draft normalisation, validation and save", () => {
  const document = fixture();
  const workspace = buildFlowWorkspaceFromDocument({ document, definitions });
  workspace.execution = moveFlowSource(workspace.execution, "a", -1);
  workspace.execution.limits.node_retained_bytes /= 2;
  workspace.edges[0].data!.queue!.items /= 2;
  const restored = normalizeFlowWorkspaceState(
    JSON.parse(JSON.stringify(workspace)),
  );
  const saved = buildFlowSaveDocument({ workspace: restored })!;
  assert.deepEqual(saved.execution, workspace.execution);
  assert.deepEqual(
    buildFlowValidationDocument({ workspace: restored }).execution,
    workspace.execution,
  );
  assert.deepEqual(saved.edges[0].queue, workspace.edges[0].data!.queue);
  assert.deepEqual(workspace.baselineFlow, document);
  const loaded = buildFlowWorkspaceFromDocument({
    document: saved,
    definitions,
  });
  assert.deepEqual(loaded.execution, saved.execution);
  assert.notEqual(loaded.execution, saved.execution);
});

test("undo includes policy and queue edits while preserving the saved baseline and live layout", () => {
  const workspace = buildFlowWorkspaceFromDocument({
    document: fixture(),
    definitions,
  });
  const before = buildDataflowStructuralSnapshot(workspace);
  const edited = structuredClone(workspace);
  edited.execution.limits.pending_events *= 2;
  assert.equal(
    areDataflowStructuralSnapshotsEqual(
      before,
      buildDataflowStructuralSnapshot(edited),
    ),
    false,
  );
  edited.execution = structuredClone(workspace.execution);
  edited.edges[0].data!.queue!.policy = "backpressure";
  assert.equal(
    areDataflowStructuralSnapshotsEqual(
      before,
      buildDataflowStructuralSnapshot(edited),
    ),
    false,
  );
  edited.nodes[0].position.x += 200;
  const undone = restoreDataflowStructuralWorkspace({
    currentWorkspace: edited,
    snapshot: before,
  });
  assert.deepEqual(undone.execution, workspace.execution);
  assert.deepEqual(undone.edges[0].data!.queue, workspace.edges[0].data!.queue);
  assert.deepEqual(undone.nodes[0].position, edited.nodes[0].position);
  assert.equal(undone.baselineFlow, edited.baselineFlow);
});

test("capacity boundaries are inclusive and connection order allows zero but not fractions", () => {
  const document = fixture();
  for (const capacity of [1, 0xffffffff]) {
    const policy = createFlowExecutionPolicy();
    policy.limits = {
      pending_events: capacity,
      retained_bytes: capacity,
      node_retained_bytes: capacity,
      provider_queue: {
        items: capacity,
        bytes: capacity,
        max_event_bytes: capacity,
        policy: "backpressure",
      },
    };
    assert.deepEqual(
      validateFlowExecutionSettings(policy, [
        {
          id: "boundary",
          order: 0,
          queue: { ...policy.limits.provider_queue, policy: "drop_oldest" },
        },
      ]),
      [],
    );
  }
  for (const order of [-1, 0.5, Infinity, 2 ** 32]) {
    assert.ok(
      validateFlowExecutionSettings(document.execution, [
        { ...document.edges[0], order },
      ]).length,
    );
  }
});

test("cloning a draft preserves authored policy without aliasing the original document", () => {
  const original = fixture();
  const workspace = buildFlowWorkspaceFromDocument({
    document: original,
    definitions,
  });
  workspace.execution = moveFlowSource(workspace.execution, "a", -1);
  workspace.draftFlowKey = "policy-clone";
  const clone = buildFlowSaveDocument({
    workspace,
    titleOverride: "Policy clone",
  })!;
  assert.deepEqual(clone.execution, workspace.execution);
  assert.deepEqual(clone.edges[0].queue, original.edges[0].queue);
  assert.notEqual(clone.execution, original.execution);
  assert.notEqual(clone.edges[0].queue, original.edges[0].queue);
  assert.equal(clone.flow_key, workspace.draftFlowKey);
});

test("deleting a source removes its priority so a new node reusing its identity is appended", () => {
  const workspace = buildFlowWorkspaceFromDocument({
    document: fixture(),
    definitions,
  });
  const removed = removeNodeFromWorkspace(workspace, "z");
  assert.ok(!removed.execution.source_order.includes("z"));
  const recreated = {
    ...removed,
    nodes: [...removed.nodes, workspace.nodes.find((node) => node.id === "z")!],
  };
  const saved = buildFlowDocumentFromWorkspace(recreated);
  assert.equal(saved.execution.source_order.at(-1), "z");
  assert.deepEqual(
    workspace.execution.source_order,
    fixture().execution.source_order,
  );
});
