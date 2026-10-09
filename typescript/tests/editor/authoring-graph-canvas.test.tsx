/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises the shared authoring graph canvas substrate used by editor workspaces.
 */
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { DndContext } from "@dnd-kit/core";
import { renderToStaticMarkup } from "react-dom/server";

import { AuthoringGraphCanvas } from "@battersea/editor/graph";

test("AuthoringGraphCanvas renders the supplied pane, canvas, controls, and overlay", () => {
  const html = renderToStaticMarkup(
    <DndContext>
      <AuthoringGraphCanvas
        canvasClassName="test-canvas"
        controlsProps={{ showZoom: false }}
        dropzoneId="test-dropzone"
        edgeTypes={{}}
        edges={[]}
        flowInstanceRef={{ current: null }}
        nodeTypes={{}}
        nodes={[]}
        overlay={<div className="test-overlay">Overlay</div>}
        paneClassName="test-pane"
      />
    </DndContext>
  );

  assert.match(html, /class="authoring-graph-canvas-pane test-pane"/);
  assert.match(html, /class="react-flow authoring-graph-canvas test-canvas light"/);
  assert.match(html, /react-flow__controls/);
  assert.match(html, /test-overlay/);
  assert.match(html, /react-flow__controls[\s\S]*<\/div><div class="test-overlay">Overlay<\/div>/);
  assert.doesNotMatch(html, /zoom in/i);
  assert.doesNotMatch(html, /zoom out/i);
});

test("AuthoringGraphCanvas omits controls when disabled", () => {
  const html = renderToStaticMarkup(
    <DndContext>
      <AuthoringGraphCanvas
        canvasClassName="test-canvas"
        controlsProps={false}
        dropzoneId="test-dropzone"
        edgeTypes={{}}
        edges={[]}
        flowInstanceRef={{ current: null }}
        nodeTypes={{}}
        nodes={[]}
        paneClassName="test-pane"
      />
    </DndContext>
  );

  assert.doesNotMatch(html, /react-flow__controls/);
});

const TEST_LAYOUT_ENGINES = [
  { description: "Tidy tree", icon: "type-hierarchy-sub", id: "elk-tree", label: "ELK · tree" },
  { description: "Layered flow", icon: "git-merge", id: "elk-layered", label: "ELK · layered" }
] as const;

test("AuthoringGraphCanvas renders the auto-layout apply action and engine picker when a controller is visible", () => {
  const html = renderToStaticMarkup(
    <DndContext>
      <AuthoringGraphCanvas
        autoLayoutController={{
          activeEngineId: "elk-tree",
          engines: TEST_LAYOUT_ENGINES,
          onApply: () => undefined,
          onSelectEngine: () => undefined,
          visible: true
        }}
        canvasClassName="test-canvas"
        controlsProps={{ showZoom: false }}
        dropzoneId="test-dropzone"
        edgeTypes={{}}
        edges={[]}
        flowInstanceRef={{ current: null }}
        nodeTypes={{}}
        nodes={[]}
        paneClassName="test-pane"
      />
    </DndContext>
  );

  assert.match(html, /authoring-graph-canvas__auto-layout-apply/);
  assert.match(html, /authoring-graph-canvas__auto-layout-menu/);
  assert.match(html, /aria-label="Auto layout: apply ELK · tree"/);
  assert.match(html, /aria-haspopup="menu"/);
});

test("AuthoringGraphCanvas omits the engine picker when only one engine is offered", () => {
  const html = renderToStaticMarkup(
    <DndContext>
      <AuthoringGraphCanvas
        autoLayoutController={{
          activeEngineId: "elk-tree",
          engines: [TEST_LAYOUT_ENGINES[0]],
          onApply: () => undefined,
          onSelectEngine: () => undefined,
          visible: true
        }}
        canvasClassName="test-canvas"
        controlsProps={{ showZoom: false }}
        dropzoneId="test-dropzone"
        edgeTypes={{}}
        edges={[]}
        flowInstanceRef={{ current: null }}
        nodeTypes={{}}
        nodes={[]}
        paneClassName="test-pane"
      />
    </DndContext>
  );

  assert.match(html, /authoring-graph-canvas__auto-layout-apply/);
  assert.doesNotMatch(html, /authoring-graph-canvas__auto-layout-menu/);
});

test("AuthoringGraphCanvas hides the auto-layout control when the controller is not visible", () => {
  const html = renderToStaticMarkup(
    <DndContext>
      <AuthoringGraphCanvas
        autoLayoutController={{
          activeEngineId: "elk-tree",
          engines: TEST_LAYOUT_ENGINES,
          onApply: () => undefined,
          onSelectEngine: () => undefined,
          visible: false
        }}
        canvasClassName="test-canvas"
        controlsProps={{ showZoom: false }}
        dropzoneId="test-dropzone"
        edgeTypes={{}}
        edges={[]}
        flowInstanceRef={{ current: null }}
        nodeTypes={{}}
        nodes={[]}
        paneClassName="test-pane"
      />
    </DndContext>
  );

  assert.doesNotMatch(html, /authoring-graph-canvas__auto-layout-apply/);
});
