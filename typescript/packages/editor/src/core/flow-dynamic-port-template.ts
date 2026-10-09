/**
 * Copyright (c) Scott A Dixon
 *
 * Shared helpers for dynamic-port name templates ("input-{index}") and for
 * computing the old→new id rename map used when the editor renumbers a
 * dynamic-port group. Both port delete and port reorder rely on this so the
 * saved flow's port ids encode the user's intended order (the engine sorts
 * dynamic input ports by numeric suffix, so visual order has to be expressed
 * in the id itself, not in a parallel `port_order` field the runtime ignores).
 */
import type {
  FlowDynamicPortGroup as WireFlowDynamicPortGroup,
  FlowNodeDefinition as WireFlowNodeDefinition,
} from "@battersea/flow";

import type { FlowPortSide } from "./flow-node-ports.js";

/**
 * Parses a port id against a `name_template` like `"input-{index}"`. Returns
 * the numeric index when the id matches the template, or null when it doesn't
 * (e.g. the id is a fixed-port name like `"prompt"`).
 */
export function dynamicPortTemplateIndex(
  template: string,
  portId: string,
): number | null {
  const placeholder = "{index}";
  const at = template.indexOf(placeholder);
  if (at < 0) {
    return null;
  }

  const prefix = template.slice(0, at);
  const suffix = template.slice(at + placeholder.length);
  if (
    portId.length < prefix.length + suffix.length ||
    !portId.startsWith(prefix) ||
    !portId.endsWith(suffix)
  ) {
    return null;
  }

  const middle = portId.slice(prefix.length, portId.length - suffix.length);
  return /^\d+$/.test(middle) ? Number.parseInt(middle, 10) : null;
}

/**
 * Renders a port id from a template and an index, e.g. `("input-{index}", 2)`
 * → `"input-2"`.
 */
export function dynamicPortTemplateId(template: string, index: number): string {
  return template.replace("{index}", String(index));
}

/**
 * Returns the dynamic port groups defined for the given side. Action,
 * signal and automation sides have no dynamic groups in the current
 * schema; only `input` and `output` carry variadic groups.
 */
export function dynamicPortGroupsForSide(
  definition: WireFlowNodeDefinition,
  side: FlowPortSide,
): readonly WireFlowDynamicPortGroup[] {
  if (side === "input") {
    return definition.dynamic_input_ports ?? [];
  }
  if (side === "output") {
    return definition.dynamic_output_ports ?? [];
  }
  return [];
}

/**
 * Builds a `oldPortId -> newPortId` rename map for a side, given the
 * desired visual order of ports (by their current ids). Fixed ports keep
 * their ids; each dynamic group's surviving members are renumbered into a
 * contiguous id space (`0..N-1`) in the order they appear in the visual
 * sequence.
 *
 * Passing the same id list for both the visual order and the existing
 * order is a no-op (identity map). The caller is responsible for then
 * applying the map to `port_names`, `port_parameter_values`, and any edges
 * whose handles reference renamed ids.
 */
export function buildDynamicPortRenameMap(options: {
  definition: WireFlowNodeDefinition;
  side: FlowPortSide;
  visualOrderOldIds: readonly string[];
}): Map<string, string> {
  const rename = new Map<string, string>();
  for (const id of options.visualOrderOldIds) {
    rename.set(id, id);
  }

  for (const group of dynamicPortGroupsForSide(
    options.definition,
    options.side,
  )) {
    let sequence = 0;
    for (const oldId of options.visualOrderOldIds) {
      if (dynamicPortTemplateIndex(group.name_template, oldId) !== null) {
        rename.set(oldId, dynamicPortTemplateId(group.name_template, sequence));
        sequence += 1;
      }
    }
  }

  return rename;
}
