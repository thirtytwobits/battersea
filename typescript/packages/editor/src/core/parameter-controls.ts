/** Copyright (c) Scott A Dixon */
export function resolveNumberControlValue(
  value: unknown,
  fallback: number,
): number {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : typeof value === "string" && value.trim()
      ? Number.parseInt(value, 10) || fallback
      : fallback;
}

export function resolveTextControlValue(value: unknown): string {
  return typeof value === "string" ? value : "";
}

export function resolveMultiSelectValue(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((entry): entry is string => typeof entry === "string")
    : [];
}
