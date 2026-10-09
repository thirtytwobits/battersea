import type { FlowParameterDefinition as WireFlowParameterDefinition } from "@battersea/flow";

export interface FlowControlLayoutGroup {
  parameters: WireFlowParameterDefinition[];
  layout: "row" | "stack";
}

const FORMATTING_PARAMETER_ORDER = [
  "output_encoding",
  "whitespace_mode",
  "plain_fragment_delimiter",
] as const;

const FORMATTING_PARAMETER_NAMES = new Set<string>(FORMATTING_PARAMETER_ORDER);

export function groupControlsForDetailPane(
  parameters: WireFlowParameterDefinition[],
): FlowControlLayoutGroup[] {
  const groups: FlowControlLayoutGroup[] = [];
  const remaining = [...parameters];

  while (remaining.length > 0) {
    const parameter = remaining.shift();
    if (!parameter) {
      continue;
    }

    if (FORMATTING_PARAMETER_NAMES.has(parameter.name)) {
      const candidates = [parameter, ...remaining];
      const hasFormattingRoot = candidates.some(
        (entry) => entry.name === "output_encoding",
      );
      if (hasFormattingRoot) {
        const formattingParameters = FORMATTING_PARAMETER_ORDER.flatMap(
          (name) => {
            const match = candidates.find((entry) => entry.name === name);
            return match ? [match] : [];
          },
        );
        for (const formattingParameter of formattingParameters) {
          const index = remaining.findIndex(
            (entry) => entry.name === formattingParameter.name,
          );
          if (index >= 0) {
            remaining.splice(index, 1);
          }
        }
        groups.push({
          layout: "stack",
          parameters: formattingParameters,
        });
        continue;
      }
    }

    if (parameter.editor.kind === "input_port_count") {
      const outputIndex = remaining.findIndex(
        (entry) => entry.editor.kind === "output_port_count",
      );
      if (outputIndex >= 0) {
        const [outputParameter] = remaining.splice(outputIndex, 1);
        groups.push({
          layout: "row",
          parameters: [parameter, outputParameter],
        });
        continue;
      }
    }

    if (parameter.editor.kind === "output_port_count") {
      const inputIndex = remaining.findIndex(
        (entry) => entry.editor.kind === "input_port_count",
      );
      if (inputIndex >= 0) {
        const [inputParameter] = remaining.splice(inputIndex, 1);
        groups.push({
          layout: "row",
          parameters: [inputParameter, parameter],
        });
        continue;
      }
    }

    groups.push({
      layout: "stack",
      parameters: [parameter],
    });
  }

  return groups;
}
