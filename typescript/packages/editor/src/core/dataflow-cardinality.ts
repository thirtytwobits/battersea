/**
 * Copyright (c) Scott A Dixon
 *
 * Implements dataflow cardinality for the editor's dataflow workspace.
 */
export function formatCardinalityRange(
  min: number,
  max: number | null,
): string {
  if (max === null) {
    return `(${min}...*)`;
  }

  if (min === max) {
    return `(${min})`;
  }

  return `(${min}...${max})`;
}

export function isFixedCardinality(min: number, max: number | null): boolean {
  return max !== null && min === max;
}

export function buildCardinalityOptions(min: number, max: number): number[] {
  const options: number[] = [];
  for (let value = min; value <= max; value += 1) {
    options.push(value);
  }

  return options;
}

export function resolveCardinalityValue(value: unknown, min: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : min;
}
