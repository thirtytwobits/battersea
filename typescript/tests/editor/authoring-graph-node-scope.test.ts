import assert from "node:assert/strict";
import test from "node:test";
import { findReactFlowNodeElement } from "@battersea/editor/graph";

test("graph geometry resolves reused node IDs only inside its owning canvas", () => {
  const nodeId = "shared-node";
  const firstNode = { getAttribute: (name: string) => name === "data-id" ? nodeId : null } as HTMLElement;
  const secondNode = { getAttribute: (name: string) => name === "data-id" ? nodeId : null } as HTMLElement;
  const canvas = (node: HTMLElement) => ({
    querySelectorAll: (selector: string) => selector === ".react-flow__node" ? [node] : []
  }) as unknown as HTMLElement;
  assert.equal(findReactFlowNodeElement(canvas(firstNode), nodeId), firstNode);
  assert.equal(findReactFlowNodeElement(canvas(secondNode), nodeId), secondNode);
  assert.equal(findReactFlowNodeElement(canvas(firstNode), "absent-node"), null);
  assert.equal(findReactFlowNodeElement(null, nodeId), null);
});
