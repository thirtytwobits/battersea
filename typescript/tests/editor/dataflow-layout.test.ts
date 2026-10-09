import assert from "node:assert/strict";
import test from "node:test";
import { buildDefaultFlowWorkspace, buildFlowDocumentFromWorkspace, buildFlowWorkspaceFromDocument } from "@battersea/editor/core/flow-persistence";
import { buildDataflowLayoutGraph, applyDataflowLayout } from "@battersea/editor/core/flow-layout";
import { loadLayoutEngine } from "@battersea/editor/graph/layout/registry";
import { createDroppedFlowNode, createFlowNodeDragPayload } from "@battersea/editor/core/flow-drag";
import { createFlowDefinitionLookup } from "@battersea/editor/core/flow-node-definitions";
import type { FlowNodeDefinition } from "@battersea/flow";
const definition: FlowNodeDefinition = { class_name: "External", handler_id: "external", kind: "inline", short_description: "", long_description: "", interfaces: [], activation_parameters: [], parameters: [],
 action_ports: [], signal_ports: [], input_ports: [{
mode: "final_value",
phase: "execution", name: "input", kind: "input", token_type: "text" }], output_ports: [{
mode: "final_value",
phase: "execution", name: "output", kind: "output", token_type: "text" }], dynamic_input_ports: [], dynamic_output_ports: [] };
test("dataflow layout retains disconnected nodes, edge direction and every authored parameter", async () => {
 const workspace = buildDefaultFlowWorkspace(); workspace.title = "Layout"; workspace.draftFlowKey = "layout";
 workspace.nodes = [1, 2, 3].map(nextIndex => createDroppedFlowNode({ payload: createFlowNodeDragPayload(definition), nextIndex, position: { x: 0, y: 0 } }));
 const [source, target] = workspace.nodes; const values = { tool_execution: ["custom"], properties: { nested: { preserved: true } } }; source!.data.parameterValues = values;
 workspace.edges = [{ id: "edge", source: source!.id, target: target!.id, sourceHandle: "output-0", targetHandle: "input-0", data: { kind: "token", order: 0 } }];
 const graph = buildDataflowLayoutGraph(workspace); assert.deepEqual(graph.nodes.map(n => n.id), workspace.nodes.map(n => n.id));
 const before = structuredClone(workspace); const engine = await loadLayoutEngine("dagre"); const positions = await engine.run(graph, { direction: "right" });
 assert.ok(positions[target!.id]!.x > positions[source!.id]!.x);
 const placed = applyDataflowLayout(workspace, positions); assert.deepEqual(workspace, before); assert.strictEqual(placed.edges, workspace.edges);
 assert.deepEqual(placed.nodes[0]!.data.parameterValues, values);
 const document = buildFlowDocumentFromWorkspace(placed);
 const restored = buildFlowWorkspaceFromDocument({ document, definitions: [definition] });
 assert.deepEqual(restored.nodes[0]!.data.parameterValues, values);
 assert.deepEqual(restored.nodes.map(n => n.position), placed.nodes.map(n => n.position));
});
test("an aborted layout does not produce positions", async () => { const signal = AbortSignal.abort(); const engine = await loadLayoutEngine("dagre"); await assert.rejects(engine.run({ nodes: [], edges: [], roots: [] }, { signal }), { name: "AbortError" }); });
