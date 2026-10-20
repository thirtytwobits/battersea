import type { Tool } from "./tools-contract.js";
import { TOOL_PROFILES_META_KEY } from "./tools-defaults.js";

/**
 * The agent profiles permitted to call `tool`, or `undefined` when its profiles entry is not a
 * list of names. A tool that lists no profiles is permitted to none.
 */
export function toolProfiles(tool: Pick<Tool, "_meta">): string[] | undefined {
  const profiles = tool._meta?.[TOOL_PROFILES_META_KEY];
  if (profiles === undefined) return [];
  if (!Array.isArray(profiles)) return undefined;
  // A copy turns a sparse array's holes into undefined entries, which `every` would otherwise skip.
  const names: unknown[] = Array.from(profiles);
  return names.every(profile => typeof profile === "string") ? names as string[] : undefined;
}
