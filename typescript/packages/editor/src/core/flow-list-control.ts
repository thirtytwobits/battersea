import type { FlowParameterDefinition } from "@battersea/flow";

export function flowListUsesSingleSelect(
  parameter: FlowParameterDefinition,
): boolean {
  return parameter.editor.kind === "list" && parameter.editor.max === 1;
}

export function resolveSingleSelectValue(value: unknown): string {
  if (!Array.isArray(value)) {
    return "";
  }

  const selectedValue = value.find(
    (entry): entry is string => typeof entry === "string",
  );
  return selectedValue ?? "";
}

export function normaliseSingleSelectValue(value: string): string[] {
  return value ? [value] : [];
}
