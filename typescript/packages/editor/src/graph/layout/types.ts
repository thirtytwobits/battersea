/**
 * Copyright (c) Scott A Dixon
 *
 * Declares the tab-agnostic graph contract that authoring-graph auto-layout
 * engines operate on. Feature tabs (World, Dataflow) adapt their domain graphs
 * into a `LayoutGraph`; engines return absolute node positions that the tab
 * commits into its own persisted layout.
 */

/** EPSG:4326 (WGS 84) coordinate in decimal degrees. */
export interface LayoutGeoCoordinate {
  latitude: number;
  longitude: number;
}

/**
 * A single layout node. Sizes are in canvas pixels and drive collision/spacing.
 *
 * `geo` is the one piece of engine-specific *input* the neutral graph carries:
 * the adapter populates it for geolocated nodes, the geographic engine projects
 * it, and every other engine ignores it. Optional per-engine inputs ride here so
 * a single adapter can feed every engine.
 */
export interface LayoutNode {
  id: string;
  width: number;
  height: number;
  geo?: LayoutGeoCoordinate;
}

/**
 * Edge semantics. `containment` is the hierarchy spine (parent → child),
 * `link` is a cross-reference, and `flow` is a directed dataflow connection.
 * Engines may weight these differently (e.g. gravity springs harder on links).
 */
export type LayoutEdgeKind = "containment" | "link" | "flow";

export interface LayoutEdge {
  id: string;
  source: string;
  target: string;
  kind: LayoutEdgeKind;
}

/**
 * The neutral graph handed to an engine. `roots` names the nodes an engine
 * should treat as hierarchy entry points (e.g. the World root place); engines
 * that ignore hierarchy may disregard it.
 */
export interface LayoutGraph {
  nodes: readonly LayoutNode[];
  edges: readonly LayoutEdge[];
  roots: readonly string[];
}

/** Top-left canvas coordinate for a laid-out node, matching React Flow positions. */
export interface LayoutPosition {
  x: number;
  y: number;
}

export type LayoutPositions = Record<string, LayoutPosition>;

export interface LayoutRunOptions {
  /** Primary flow direction for directional engines. Defaults per engine. */
  direction?: "right" | "down";
  /**
   * Engine parameter values, keyed by parameter id (see
   * `LayoutEngineDescriptor.parameters`). Absent values fall back to each
   * parameter's `defaultValue`; engines must tolerate a missing map.
   */
  parameters?: Readonly<Record<string, number>>;
  /** Aborts an in-flight run; engines should reject when signalled. */
  signal?: AbortSignal;
}

/**
 * A continuously-adjustable numeric engine parameter — rendered as a slider in
 * the auto-layout control and threaded into `run` via `LayoutRunOptions`. The
 * `kind` discriminant leaves room for boolean/enum parameters later.
 */
export interface LayoutEngineNumberParameter {
  kind: "number";
  id: string;
  label: string;
  description?: string;
  min: number;
  max: number;
  step: number;
  defaultValue: number;
}

export type LayoutEngineParameter = LayoutEngineNumberParameter;

/** A computed layout engine, loaded on demand from the registry. */
export interface LayoutEngine {
  id: string;
  run(graph: LayoutGraph, options?: LayoutRunOptions): Promise<LayoutPositions>;
}

/**
 * UI-facing engine metadata. Carried by the canvas auto-layout controller so the
 * picker can render an engine list without importing any engine implementation
 * (and therefore without pulling heavy layout libraries into the main bundle).
 */
export interface LayoutEngineDescriptor {
  id: string;
  label: string;
  description: string;
  /** Codicon id rendered in the picker and controls cluster. */
  icon: string;
  /** Tunable parameters this engine exposes as sliders; omitted when it has none. */
  parameters?: readonly LayoutEngineParameter[];
}
