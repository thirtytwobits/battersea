/**
 * Copyright (c) Scott A Dixon
 *
 * Implements flow enum switch for the editor's dataflow workspace.
 */
export interface FlowEnumOption {
  label: string;
  value: string;
}

const BINARY_OPTION_PAIRS = [
  ["false", "true"],
  ["no", "yes"],
  ["off", "on"],
  ["disabled", "enabled"],
] as const;

export function flowEnumUsesSwitch(options: FlowEnumOption[]): boolean {
  if (options.length !== 2) {
    return false;
  }

  return (
    binaryOptionTextsMatch(options.map((option) => option.value)) ||
    binaryOptionTextsMatch(options.map((option) => option.label))
  );
}

function binaryOptionTextsMatch(values: readonly string[]): boolean {
  const normalisedValues = values.map(normalizeBinaryOptionText).sort();
  return BINARY_OPTION_PAIRS.some(
    ([firstValue, secondValue]) =>
      normalisedValues[0] === firstValue && normalisedValues[1] === secondValue,
  );
}

function normalizeBinaryOptionText(value: string): string {
  return value
    .trim()
    .toLowerCase()
    .replace(/[\s_-]+/g, "-");
}
