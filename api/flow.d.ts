// contract.d.ts
export type FlowActionPortDefinition = {
    "display_class"?: FlowPortDisplayClass;
    "long_description"?: string;
    "name": string;
    "short_description"?: string;
};
export type FlowAutomationPortDefinition = {
    "accepted_token_types"?: Array<string>;
    "display_class"?: FlowPortDisplayClass;
    "long_description"?: string;
    "name": string;
    "parameter_name": string;
    "short_description"?: string;
    "token_type": string;
};
export type FlowControllerActionDefinition = {
    "kind": string;
    "long_description"?: string;
    "name": string;
    "short_description"?: string;
};
export type FlowControllerOutputDefinition = {
    "kind": string;
    "long_description"?: string;
    "name": string;
    "short_description"?: string;
};
export type FlowDocument = {
    "description"?: string;
    "edges": Array<FlowEdge>;
    "execution": FlowExecutionPolicy;
    "flow_key": string;
    "layout"?: {
        [key: string]: unknown;
    };
    "metadata"?: unknown;
    "nodes": Array<FlowNode>;
    "output_encoding": string;
    "plain_fragment_delimiter": string;
    "title": string;
    "version": number;
    "whitespace_mode": string;
};
export type FlowDynamicPortGroup = {
    "accepted_token_types"?: Array<string>;
    "count_parameter": string;
    "display_class"?: FlowPortDisplayClass;
    "long_description"?: string;
    "mode": FlowPortMode;
    "name_template": string;
    "parameters"?: Array<FlowParameterDefinition>;
    "phase": FlowPortPhase;
    "short_description"?: string;
    "token_type": string;
};
export type FlowDynamicSignalPortGroup = {
    "count_parameter": string;
    "display_class"?: FlowPortDisplayClass;
    "long_description"?: string;
    "name_template": string;
    "short_description"?: string;
};
export type FlowEdge = {
    "id": string;
    "kind": FlowEdgeKind;
    "order": number;
    "queue"?: FlowQueueLimits;
    "source_node_id": string;
    "source_port": string;
    "target_node_id": string;
    "target_port": string;
};
export type FlowEdgeKind = "token" | "signal";
export type FlowExecutionLimits = {
    "node_retained_bytes": number;
    "pending_events": number;
    "provider_queue": FlowQueueLimits;
    "retained_bytes": number;
};
export type FlowExecutionPolicy = {
    "limits": FlowExecutionLimits;
    "source_order": Array<string>;
};
export type FlowNode = {
    "definition_name": string;
    "id": string;
    "instance_name": string;
    "parameter_values": {
        [key: string]: unknown;
    };
    "port_names"?: FlowNodePortNames;
    "port_order"?: FlowNodePortOrder;
    "port_parameter_values"?: FlowNodePortParameterValues;
};
export type FlowNodeClass = "source" | "control" | "hybrid" | "instrument" | "inline" | "logic" | "sink";
export type FlowNodeDefinition = {
    "action_ports": Array<FlowActionPortDefinition>;
    "activation_parameters": Array<string>;
    "automation_ports"?: Array<FlowAutomationPortDefinition>;
    "class_name": string;
    "controller_actions"?: Array<FlowControllerActionDefinition>;
    "controller_outputs"?: Array<FlowControllerOutputDefinition>;
    "dynamic_action_ports"?: Array<FlowDynamicSignalPortGroup>;
    "dynamic_input_ports": Array<FlowDynamicPortGroup>;
    "dynamic_output_ports": Array<FlowDynamicPortGroup>;
    "dynamic_signal_ports"?: Array<FlowDynamicSignalPortGroup>;
    "handler_id": string;
    "input_ports": Array<FlowPort>;
    "interfaces": Array<string>;
    "kind": FlowNodeClass;
    "long_description": string;
    "output_ports": Array<FlowPort>;
    "parameters": Array<FlowParameterDefinition>;
    "short_description": string;
    "signal_ports": Array<FlowSignalPortDefinition>;
};
export type FlowNodePortNames = {
    "action"?: {
        [key: string]: string;
    };
    "automation"?: {
        [key: string]: string;
    };
    "input"?: {
        [key: string]: string;
    };
    "output"?: {
        [key: string]: string;
    };
    "signal"?: {
        [key: string]: string;
    };
};
export type FlowNodePortOrder = {
    "action"?: Array<string>;
    "automation"?: Array<string>;
    "input"?: Array<string>;
    "output"?: Array<string>;
    "signal"?: Array<string>;
};
export type FlowNodePortParameterValues = {
    "input"?: {
        [key: string]: {
            [key: string]: unknown;
        };
    };
    "output"?: {
        [key: string]: {
            [key: string]: unknown;
        };
    };
};
export type FlowParameterDataType = {
    "item_type"?: FlowParameterDataType;
    "kind": string;
    "value_type"?: FlowParameterDataType;
};
export type FlowParameterDefinition = {
    "controller"?: FlowParameterEditor;
    "datatype": FlowParameterDataType;
    "editor": FlowParameterEditor;
    "long_description"?: string;
    "name": string;
    "short_description"?: string;
};
export type FlowParameterEditor = {
    "default_value"?: unknown;
    "kind": FlowParameterEditorKind;
    "max"?: number;
    "min"?: number;
    "source"?: string;
    "values"?: Array<string>;
};
export type FlowParameterEditorKind = "boolean" | "unsigned" | "input_port_count" | "output_port_count" | "enum" | "list" | "media_preview" | "string" | "text" | "text_input" | "properties" | "query";
export type FlowPort = {
    "accepted_token_types"?: Array<string>;
    "display_class"?: FlowPortDisplayClass;
    "formatter"?: FlowPortFormatter;
    "kind": FlowPortKind;
    "long_description"?: string;
    "mode": FlowPortMode;
    "name": string;
    "parameters"?: Array<FlowParameterDefinition>;
    "phase": FlowPortPhase;
    "short_description"?: string;
    "token_type": string;
};
export type FlowPortDisplayClass = "source" | "inline" | "sink";
export type FlowPortFormatter = {
    "default"?: FlowPromptTemplateDefinition;
    "kind": FlowPortFormatterKind;
    "variants"?: {
        [key: string]: FlowPromptTemplateDefinition;
    };
};
export type FlowPortFormatterKind = "prompt_template";
export type FlowPortKind = "input" | "output";
export type FlowPortMode = "final_value" | "stream";
export type FlowPortPhase = "snapshot" | "execution";
export type FlowPromptListTemplateNode = {
    "binding": string;
    "item_bindings"?: Array<string>;
    "separator": string;
    "template": FlowPromptTemplateNode;
};
export type FlowPromptOptionalTemplateNode = {
    "binding": string;
    "template": FlowPromptTemplateNode;
};
export type FlowPromptSequenceTemplateNode = {
    "items": Array<FlowPromptTemplateNode>;
    "separator": string;
};
export type FlowPromptTemplateDefinition = {
    "bindings"?: Array<string>;
    "template": FlowPromptTemplateNode;
};
export type FlowPromptTemplateNode = {
    "kind": "text";
    "text": string;
} | {
    "items": Array<FlowPromptTemplateNode>;
    "kind": "sequence";
    "separator": string;
} | {
    "binding": string;
    "kind": "optional";
    "template": FlowPromptTemplateNode;
} | {
    "binding": string;
    "item_bindings"?: Array<string>;
    "kind": "list";
    "separator": string;
    "template": FlowPromptTemplateNode;
};
export type FlowPromptTextTemplateNode = {
    "text": string;
};
export type FlowQueueLimits = {
    "bytes": number;
    "items": number;
    "max_event_bytes": number;
    "policy": FlowQueuePolicy;
};
export type FlowQueuePolicy = "backpressure" | "drop_oldest";
export type FlowSignalPortDefinition = {
    "display_class"?: FlowPortDisplayClass;
    "long_description"?: string;
    "name": string;
    "short_description"?: string;
};
export type FlowValidationIssue = {
    "edge_id"?: string;
    "message": string;
    "node_id"?: string;
};
export type FlowValidationResult = {
    "issues": Array<FlowValidationIssue>;
    "valid": boolean;
};

// defaults.d.ts
export declare const FLOW_DOCUMENT_VERSION = 2;
export declare const DEFAULT_FLOW_EXECUTION_LIMITS: {
    pending_events: number;
    retained_bytes: number;
    node_retained_bytes: number;
    provider_queue: {
        items: number;
        bytes: number;
        max_event_bytes: number;
        policy: "backpressure";
    };
};

// execution.d.ts
import type { FlowDocument, FlowExecutionPolicy } from "./contract.js";
/** Execution policy for a newly authored document. Stored documents require an explicit upgrade. */
export declare function createFlowExecutionPolicy(sourceOrder?: string[]): FlowExecutionPolicy;
/** Version admission; complete graph validation belongs to the catalogue. */
export declare function requireSupportedFlowVersion(document: Pick<FlowDocument, "version" | "execution">): void;

// fixtures.d.ts
export declare const flows: {
    edges: never[];
    execution: {
        limits: {
            node_retained_bytes: number;
            pending_events: number;
            provider_queue: {
                bytes: number;
                items: number;
                max_event_bytes: number;
                policy: "backpressure";
            };
            retained_bytes: number;
        };
        source_order: string[];
    };
    flow_key: string;
    layout: {
        "editor.example": {
            position: number[];
            viewport: {
                zoom: number;
            };
        };
    };
    metadata: {
        application: {
            tags: string[];
        };
    };
    nodes: {
        definition_name: string;
        id: string;
        instance_name: string;
        parameter_values: {
            label: string;
        };
    }[];
    output_encoding: string;
    plain_fragment_delimiter: string;
    title: string;
    version: number;
    whitespace_mode: string;
}[];

// index.d.ts
export type * from "./contract.js";
export { tokenConnectionCompatible } from "./ports.js";
export { FLOW_DOCUMENT_VERSION } from "./defaults.js";
export { createFlowExecutionPolicy, requireSupportedFlowVersion } from "./execution.js";

// ports.d.ts
/** Nominal token compatibility, including pass-through outputs with inferred input types. */
export declare function tokenConnectionCompatible(sourceTokenType: string, sourceNodeInputAccepted: readonly string[], targetAccepted: readonly string[]): boolean;
