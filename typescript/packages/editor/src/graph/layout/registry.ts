/**
 * Copyright (c) Scott A Dixon
 *
 * The auto-layout engine registry. Holds UI-facing descriptors (safe to import
 * anywhere, including Node tests and the main bundle) and a lazy loader that
 * dynamically imports the heavy engine implementation only when a layout is
 * actually applied.
 */
import type { LayoutEngine, LayoutEngineDescriptor } from "./types.js";

export const DAGRE_ENGINE_ID = "dagre";
export const DAGRE_ENGINE_DESCRIPTOR: LayoutEngineDescriptor = {
  id: DAGRE_ENGINE_ID,
  label: "Dagre",
  description: "Directed graph layout.",
  icon: "git-merge",
};

export const ELK_TREE_ENGINE_ID = "elk-tree";
export const ELK_LAYERED_ENGINE_ID = "elk-layered";
export const GRAVITY_ENGINE_ID = "gravity";
export const GEOGRAPHIC_ENGINE_ID = "geographic";
export const GEOGRAPHIC_COMPACT_ENGINE_ID = "geographic-compact";

export const ELK_TREE_ENGINE_DESCRIPTOR: LayoutEngineDescriptor = {
  description: "Tidy containment hierarchy, ranked by depth.",
  icon: "type-hierarchy-sub",
  id: ELK_TREE_ENGINE_ID,
  label: "ELK · tree",
};

export const ELK_LAYERED_ENGINE_DESCRIPTOR: LayoutEngineDescriptor = {
  description: "Directional layered flow, crossings minimised.",
  icon: "git-merge",
  id: ELK_LAYERED_ENGINE_ID,
  label: "ELK · layered",
};

export const GRAVITY_ENGINE_DESCRIPTOR: LayoutEngineDescriptor = {
  description: "Force-directed; clusters linked nodes.",
  icon: "graph-scatter",
  id: GRAVITY_ENGINE_ID,
  label: "Gravity",
};

export const GEOGRAPHIC_ENGINE_DESCRIPTOR: LayoutEngineDescriptor = {
  description: "Projects latitude and longitude onto a map.",
  icon: "globe",
  id: GEOGRAPHIC_ENGINE_ID,
  label: "Geographic",
};

export const GEOGRAPHIC_COMPACT_ENGINE_DESCRIPTOR: LayoutEngineDescriptor = {
  description: "Real local scale; oceans between clusters collapsed.",
  icon: "combine",
  id: GEOGRAPHIC_COMPACT_ENGINE_ID,
  label: "Geographic · compact",
  parameters: [
    {
      // Keep the default/max in sync with COMPACTION_DEFAULT/COMPACTION_MAX in
      // geo-engine.ts. Goes past 100% to pack clusters nearly touching.
      defaultValue: 0.5,
      description: "How tightly the gulfs between clusters collapse.",
      id: "compaction",
      kind: "number",
      label: "Compaction",
      max: 2,
      min: 0,
      step: 0.05,
    },
  ],
};

/** All engines known to the registry, in display order. */
export const LAYOUT_ENGINE_DESCRIPTORS: readonly LayoutEngineDescriptor[] = [
  DAGRE_ENGINE_DESCRIPTOR,
  GRAVITY_ENGINE_DESCRIPTOR,
  GEOGRAPHIC_ENGINE_DESCRIPTOR,
  GEOGRAPHIC_COMPACT_ENGINE_DESCRIPTOR,
];

const DESCRIPTORS_BY_ID = new Map(
  LAYOUT_ENGINE_DESCRIPTORS.map((descriptor) => [descriptor.id, descriptor]),
);

export function isLayoutEngineId(id: string): boolean {
  return DESCRIPTORS_BY_ID.has(id);
}

export function resolveLayoutEngineDescriptor(
  id: string,
): LayoutEngineDescriptor | undefined {
  return DESCRIPTORS_BY_ID.get(id);
}

/**
 * Picks the supplied engine id when known, otherwise falls back to the first
 * registered engine. Keeps persisted/unknown ids from breaking the picker.
 */
export function resolveLayoutEngineId(
  id: string | undefined,
  available: readonly LayoutEngineDescriptor[],
): string {
  if (id && available.some((descriptor) => descriptor.id === id)) {
    return id;
  }
  if (!available.length)
    throw new RangeError("At least one layout engine is required");
  return available[0].id;
}

/**
 * Loads a layout engine on demand.
 */
export async function loadLayoutEngine(id: string): Promise<LayoutEngine> {
  switch (id) {
    case DAGRE_ENGINE_ID: {
      const { createDagreEngine } = await import("./dagre-engine.js");
      return createDagreEngine();
    }
    case GEOGRAPHIC_ENGINE_ID: {
      const { createGeographicEngine } = await import("./geo-engine.js");
      return createGeographicEngine(GEOGRAPHIC_ENGINE_ID);
    }
    case GEOGRAPHIC_COMPACT_ENGINE_ID: {
      const { createCompactGeographicEngine } = await import("./geo-engine.js");
      return createCompactGeographicEngine(GEOGRAPHIC_COMPACT_ENGINE_ID);
    }
    case GRAVITY_ENGINE_ID: {
      const { createGravityEngine } = await import("./gravity-engine.js");
      return createGravityEngine(GRAVITY_ENGINE_ID);
    }
    default:
      throw new RangeError(`Unknown layout engine: ${id}`);
  }
}
