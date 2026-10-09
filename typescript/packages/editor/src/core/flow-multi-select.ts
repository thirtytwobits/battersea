/**
 * Copyright (c) Scott A Dixon
 *
 * Implements flow multi select for the editor's dataflow workspace.
 */
export interface FlowMultiSelectOption {
  label: string;
  value: string;
}

export function selectAllFlowMultiSelectValues(
  options: FlowMultiSelectOption[],
): string[] {
  return Array.from(new Set(options.map((option) => option.value)));
}

export function selectNoFlowMultiSelectValues(): string[] {
  return [];
}

export function flowMultiSelectHasAllValues(
  options: FlowMultiSelectOption[],
  selectedValues: string[],
): boolean {
  const allValues = selectAllFlowMultiSelectValues(options);
  if (allValues.length === 0) {
    return false;
  }

  const selected = new Set(selectedValues);
  return allValues.every((value) => selected.has(value));
}
