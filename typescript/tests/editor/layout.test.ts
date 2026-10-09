/**
 * Copyright (c) Scott A Dixon
 *
 * Exercises the shared auto-layout registry, the pure ELK graph mapping, the
 * World graph adapter, and end-to-end ELK and gravity runs.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { buildElkGraph, readElkPositions } from "@battersea/editor/graph/layout/elk-graph";
import { runCompactGeographicLayout, runGeographicLayout } from "@battersea/editor/graph/layout/geo-engine";
import { runGravityLayout } from "@battersea/editor/graph/layout/gravity-engine";
import {
  ELK_LAYERED_ENGINE_ID,
  ELK_TREE_ENGINE_ID,
  GEOGRAPHIC_COMPACT_ENGINE_ID,
  GEOGRAPHIC_ENGINE_ID,
  GRAVITY_ENGINE_ID,
  resolveLayoutEngineId
} from "@battersea/editor/graph/layout/registry";
import type { LayoutGraph } from "@battersea/editor/graph";
import { loadLayoutEngine, LAYOUT_ENGINE_DESCRIPTORS } from "../../packages/editor/src/graph/layout/registry.js";
const SAMPLE_GRAPH: LayoutGraph = {
  edges: [
    { id: "a-b", kind: "containment", source: "a", target: "b" },
    { id: "a-c", kind: "containment", source: "a", target: "c" },
    { id: "b-c", kind: "link", source: "b", target: "c" }
  ],
  nodes: [
    { height: 72, id: "a", width: 200 },
    { height: 72, id: "b", width: 200 },
    { height: 72, id: "c", width: 200 }
  ],
  roots: ["a"]
};

test("buildElkGraph maps nodes, edges, and algorithm options", () => {
  const elkGraph = buildElkGraph(SAMPLE_GRAPH, { algorithm: "mrtree" });

  assert.equal(elkGraph.layoutOptions["elk.algorithm"], "mrtree");
  assert.equal(elkGraph.children.length, 3);
  assert.equal(elkGraph.edges.length, 3);
  assert.deepEqual(elkGraph.edges[0], { id: "a-b", sources: ["a"], targets: ["b"] });
});

test("buildElkGraph defaults layered to RIGHT and tree to DOWN", () => {
  assert.equal(buildElkGraph(SAMPLE_GRAPH, { algorithm: "layered" }).layoutOptions["elk.direction"], "RIGHT");
  assert.equal(buildElkGraph(SAMPLE_GRAPH, { algorithm: "mrtree" }).layoutOptions["elk.direction"], "DOWN");
  assert.equal(
    buildElkGraph(SAMPLE_GRAPH, { algorithm: "mrtree", direction: "right" }).layoutOptions["elk.direction"],
    "RIGHT"
  );
});

test("buildElkGraph drops self-loops and edges to missing nodes", () => {
  const graph: LayoutGraph = {
    edges: [
      { id: "self", kind: "link", source: "a", target: "a" },
      { id: "dangling", kind: "link", source: "a", target: "ghost" }
    ],
    nodes: [{ height: 72, id: "a", width: 200 }],
    roots: []
  };

  assert.equal(buildElkGraph(graph, { algorithm: "layered" }).edges.length, 0);
});

test("readElkPositions rounds coordinates and skips nodes without geometry", () => {
  const positions = readElkPositions({
    children: [
      { id: "a", x: 10.4, y: 20.6 },
      { id: "b" }
    ]
  });

  assert.deepEqual(positions, { a: { x: 10, y: 21 } });
});

test("the gravity engine loads from the registry and positions every node", async () => {
  const engine = await loadLayoutEngine(GRAVITY_ENGINE_ID);
  assert.equal(engine.id, GRAVITY_ENGINE_ID);

  const positions = await engine.run(SAMPLE_GRAPH);
  for (const node of SAMPLE_GRAPH.nodes) {
    const position = positions[node.id];
    assert.ok(position, `expected a position for ${node.id}`);
    assert.equal(Number.isFinite(position.x), true);
    assert.equal(Number.isFinite(position.y), true);
  }
});

test("the gravity layout is deterministic for a given graph", () => {
  assert.deepEqual(runGravityLayout(SAMPLE_GRAPH), runGravityLayout(SAMPLE_GRAPH));
});

test("the gravity layout separates nodes by at least their collision size", () => {
  const positions = runGravityLayout(SAMPLE_GRAPH);
  const ids = SAMPLE_GRAPH.nodes.map((node) => node.id);
  for (let i = 0; i < ids.length; i += 1) {
    for (let j = i + 1; j < ids.length; j += 1) {
      const a = positions[ids[i]!]!;
      const b = positions[ids[j]!]!;
      assert.ok(Math.hypot(a.x - b.x, a.y - b.y) > 100, `${ids[i]} and ${ids[j]} overlap`);
    }
  }
});

test("the gravity engine returns no positions for an empty graph", () => {
  assert.deepEqual(runGravityLayout({ edges: [], nodes: [], roots: [] }), {});
});

/** A real-world slice covering the geographic relationships the engine must respect. */
function geoNode(id: string, latitude: number, longitude: number) {
  return { geo: { latitude, longitude }, height: 76, id, width: 220 };
}

const WORLD_GEO_GRAPH: LayoutGraph = {
  edges: [
    // A flat inside an Archway building (the flat itself is abstract — no coordinates).
    { id: "bld-flat", kind: "containment", source: "archway-building", target: "archway-flat" }
  ],
  nodes: [
    geoNode("greater-london", 51.5074, -0.1278),
    geoNode("bloomsbury", 51.522, -0.1244),
    geoNode("primrose-hill", 51.5388, -0.161),
    geoNode("archway", 51.5654, -0.1353),
    geoNode("archway-building", 51.5655, -0.1352),
    geoNode("edmonton", 51.625, -0.064),
    geoNode("shere", 51.2167, -0.4667),
    geoNode("birmingham", 52.4862, -1.8904),
    geoNode("edinburgh", 55.9533, -3.1883),
    geoNode("cupertino", 37.323, -122.0322),
    geoNode("yokohama", 35.4437, 139.638),
    { height: 76, id: "archway-flat", width: 220 }
  ],
  roots: ["greater-london"]
};

function distance(a: { x: number; y: number }, b: { x: number; y: number }): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

test("the geographic engine loads from the registry and positions every node", async () => {
  const engine = await loadLayoutEngine(GEOGRAPHIC_ENGINE_ID);
  assert.equal(engine.id, GEOGRAPHIC_ENGINE_ID);

  const positions = await engine.run(WORLD_GEO_GRAPH);
  for (const node of WORLD_GEO_GRAPH.nodes) {
    const position = positions[node.id];
    assert.ok(position, `expected a position for ${node.id}`);
    assert.equal(Number.isFinite(position.x), true);
    assert.equal(Number.isFinite(position.y), true);
  }
});

test("the geographic engine keeps north up and south down", () => {
  const positions = runGeographicLayout(WORLD_GEO_GRAPH);
  // Archway / Edmonton are north of central London; Shere (Surrey) is south.
  assert.ok(positions.archway!.y < positions["greater-london"]!.y, "Archway should be north of Greater London");
  assert.ok(positions.edmonton!.y < positions["greater-london"]!.y, "Edmonton should be north of Greater London");
  assert.ok(positions.shere!.y > positions["greater-london"]!.y, "Shere should be south of Greater London");
});

test("the geographic engine keeps order inside a crowded cluster", () => {
  const positions = runGeographicLayout(WORLD_GEO_GRAPH);
  // Bloomsbury (central London) must stay north of Shere, which is far south in Surrey.
  assert.ok(positions.bloomsbury!.y < positions.shere!.y, "Bloomsbury should be north of Shere");
  // Edmonton is east of Primrose Hill, even though both are crowded into north London.
  assert.ok(positions.edmonton!.x > positions["primrose-hill"]!.x, "Edmonton should be east of Primrose Hill");
});

test("the geographic engine places Birmingham south-east of Edinburgh", () => {
  const positions = runGeographicLayout(WORLD_GEO_GRAPH);
  // Edinburgh (3.19W) is actually further west than Birmingham (1.89W), so
  // Birmingham is south-EAST of Edinburgh — east (greater x) and south (greater y).
  assert.ok(positions.birmingham!.x > positions.edinburgh!.x, "Birmingham should be east of Edinburgh");
  assert.ok(positions.birmingham!.y > positions.edinburgh!.y, "Birmingham should be south of Edinburgh");
});

test("the geographic engine keeps far-apart places far apart", () => {
  const positions = runGeographicLayout(WORLD_GEO_GRAPH);
  const edmontonToLondon = distance(positions.edmonton!, positions["greater-london"]!);
  const edmontonToCupertino = distance(positions.edmonton!, positions.cupertino!);
  // Cupertino (~8500km from London) must be vastly further from Edmonton than its London neighbour.
  assert.ok(
    edmontonToCupertino > edmontonToLondon * 20,
    `Cupertino should dwarf the London-local distance (local ${Math.round(edmontonToLondon)}, transatlantic ${Math.round(edmontonToCupertino)})`
  );
  // California is west of London; Japan is east.
  assert.ok(positions.cupertino!.x < positions["greater-london"]!.x, "Cupertino should be west of London");
  assert.ok(positions.yokohama!.x > positions["greater-london"]!.x, "Yokohama should be east of London");
});

test("a contained flat stays near its building, not flung across the globe", () => {
  const positions = runGeographicLayout(WORLD_GEO_GRAPH);
  const flatToBuilding = distance(positions["archway-flat"]!, positions["archway-building"]!);
  const londonToYokohama = distance(positions["greater-london"]!, positions.yokohama!);
  assert.ok(
    flatToBuilding < londonToYokohama,
    `the flat (${Math.round(flatToBuilding)}) should be far closer to its building than London is to Yokohama (${Math.round(londonToYokohama)})`
  );
});

test("dense geographic clusters do not fling abstract hierarchies away from linked places", () => {
  const graph: LayoutGraph = {
    edges: [
      { id: "root-a", kind: "containment", source: "root", target: "geo-a" },
      { id: "root-b", kind: "containment", source: "root", target: "geo-b" },
      { id: "root-region", kind: "containment", source: "root", target: "region" },
      { id: "region-place", kind: "containment", source: "region", target: "place" }
    ],
    nodes: [
      { height: 72, id: "root", width: 200 },
      geoNode("geo-a", 51.5, -0.12),
      geoNode("geo-b", 51.50001, -0.11999),
      { height: 72, id: "region", width: 200 },
      { height: 72, id: "place", width: 200 }
    ],
    roots: ["root"]
  };
  const maximumHierarchySpan = Math.max(...graph.nodes.map((node) => Math.max(node.width, node.height)))
    * graph.nodes.length
    * 2;

  for (const [engineName, run] of [
    ["geographic", runGeographicLayout],
    ["compact geographic", runCompactGeographicLayout]
  ] as const) {
    const positions = run(graph);
    const maximumLinkedDistance = Math.max(...graph.edges.map((edge) => (
      distance(positions[edge.source]!, positions[edge.target]!)
    )));

    assert.ok(
      maximumLinkedDistance < maximumHierarchySpan,
      `${engineName} abstract containment should remain near its geographic cluster (maximum link ${Math.round(maximumLinkedDistance)})`
    );
  }
});

test("the geographic layout is deterministic for a given graph", () => {
  assert.deepEqual(runGeographicLayout(WORLD_GEO_GRAPH), runGeographicLayout(WORLD_GEO_GRAPH));
});

test("the geographic engine falls back to gravity when nothing is geolocated", () => {
  assert.deepEqual(runGeographicLayout(SAMPLE_GRAPH), runGravityLayout(SAMPLE_GRAPH));
});

test("the compact geographic engine collapses the gulfs between clusters", () => {
  const faithful = runGeographicLayout(WORLD_GEO_GRAPH);
  const compact = runCompactGeographicLayout(WORLD_GEO_GRAPH, { parameters: { compaction: 1 } });

  // California is its own cluster an ocean away. Compact pulls it far closer to
  // London than the faithful map does.
  const faithfulGap = distance(faithful["greater-london"]!, faithful.cupertino!);
  const compactGap = distance(compact["greater-london"]!, compact.cupertino!);
  assert.ok(
    compactGap < faithfulGap / 5,
    `compact should collapse the transatlantic gulf (faithful ${Math.round(faithfulGap)}, compact ${Math.round(compactGap)})`
  );
});

test("the compact geographic engine keeps clusters distinct, ordered, and locally faithful", () => {
  const positions = runCompactGeographicLayout(WORLD_GEO_GRAPH);

  // Distinct: California is still its own cluster — farther from London than a
  // London-local neighbour, not merged into it.
  const londonLocal = distance(positions["greater-london"]!, positions.edmonton!);
  const toCupertino = distance(positions["greater-london"]!, positions.cupertino!);
  assert.ok(toCupertino > londonLocal, "Cupertino should stay a distinct cluster, not collapse into London");

  // Ordered: west stays west, east stays east, north stays up.
  assert.ok(positions.cupertino!.x < positions["greater-london"]!.x, "Cupertino should stay west of London");
  assert.ok(positions.yokohama!.x > positions["greater-london"]!.x, "Yokohama should stay east of London");
  assert.ok(positions.edmonton!.y < positions["greater-london"]!.y, "Edmonton should stay north of Greater London");

  // Locally faithful: within the London cluster the real arrangement survives.
  assert.ok(positions.bloomsbury!.y < positions.shere!.y, "Bloomsbury should stay north of Shere");
  assert.ok(positions.edmonton!.x > positions["primrose-hill"]!.x, "Edmonton should stay east of Primrose Hill");
});

test("the compact geographic engine loads from the registry", async () => {
  const engine = await loadLayoutEngine(GEOGRAPHIC_COMPACT_ENGINE_ID);
  assert.equal(engine.id, GEOGRAPHIC_COMPACT_ENGINE_ID);
  const positions = await engine.run(WORLD_GEO_GRAPH);
  for (const node of WORLD_GEO_GRAPH.nodes) {
    assert.ok(positions[node.id], `expected a position for ${node.id}`);
  }
});

test("the compact geographic descriptor declares a tunable compaction parameter", () => {
  const descriptor = LAYOUT_ENGINE_DESCRIPTORS.find((d) => d.id === GEOGRAPHIC_COMPACT_ENGINE_ID);
  const parameter = descriptor?.parameters?.find((p) => p.id === "compaction");
  assert.ok(parameter, "compact engine should declare a compaction parameter");
  assert.equal(parameter!.kind, "number");
  assert.equal(parameter!.min, 0);
  assert.equal(parameter!.max, 2);
});

test("the compaction parameter tightens the layout as it rises", () => {
  const extentOf = (positions: Record<string, { x: number; y: number }>): number => {
    const xs = Object.values(positions).map((p) => p.x);
    const ys = Object.values(positions).map((p) => p.y);
    return Math.max(Math.max(...xs) - Math.min(...xs), Math.max(...ys) - Math.min(...ys));
  };
  const loose = extentOf(runCompactGeographicLayout(WORLD_GEO_GRAPH, { parameters: { compaction: 0 } }));
  const tight = extentOf(runCompactGeographicLayout(WORLD_GEO_GRAPH, { parameters: { compaction: 1 } }));
  // Past 100%: the range extends to 200%, which keeps tightening rather than
  // inverting the (geometric) gap into a negative value.
  const tighter = extentOf(runCompactGeographicLayout(WORLD_GEO_GRAPH, { parameters: { compaction: 2 } }));
  assert.ok(
    tighter < tight && tight < loose,
    `higher compaction should keep shrinking (loose ${Math.round(loose)}, 100% ${Math.round(tight)}, 200% ${Math.round(tighter)})`
  );
});

