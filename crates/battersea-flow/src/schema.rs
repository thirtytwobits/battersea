//! JSON Schema exported from the Rust wire contract.
use crate::*;
use schemars::generate::SchemaSettings;
use serde_json::{json, Value};

pub fn contract_schema() -> Value {
    let mut generator = SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator();
    generator.subschema_for::<FlowPortKind>();
    generator.subschema_for::<FlowNodeClass>();
    generator.subschema_for::<FlowPortDisplayClass>();
    generator.subschema_for::<FlowParameterDataType>();
    generator.subschema_for::<FlowParameterEditorKind>();
    generator.subschema_for::<FlowParameterEditor>();
    generator.subschema_for::<FlowParameterDefinition>();
    generator.subschema_for::<FlowPortFormatterKind>();
    generator.subschema_for::<FlowControllerOutputDefinition>();
    generator.subschema_for::<FlowControllerActionDefinition>();
    generator.subschema_for::<FlowEdgeKind>();
    generator.subschema_for::<FlowActionPortDefinition>();
    generator.subschema_for::<FlowSignalPortDefinition>();
    generator.subschema_for::<FlowAutomationPortDefinition>();
    generator.subschema_for::<FlowPromptTextTemplateNode>();
    generator.subschema_for::<FlowPromptSequenceTemplateNode>();
    generator.subschema_for::<FlowPromptOptionalTemplateNode>();
    generator.subschema_for::<FlowPromptListTemplateNode>();
    generator.subschema_for::<FlowPromptTemplateNode>();
    generator.subschema_for::<FlowPromptTemplateDefinition>();
    generator.subschema_for::<FlowPortFormatter>();
    generator.subschema_for::<FlowPort>();
    generator.subschema_for::<FlowDynamicPortGroup>();
    generator.subschema_for::<FlowDynamicSignalPortGroup>();
    generator.subschema_for::<FlowNodeDefinition>();
    generator.subschema_for::<FlowNodePortNames>();
    generator.subschema_for::<FlowNodePortOrder>();
    generator.subschema_for::<FlowNodePortParameterValues>();
    generator.subschema_for::<FlowNode>();
    generator.subschema_for::<FlowEdge>();
    generator.subschema_for::<FlowDocument>();
    generator.subschema_for::<FlowValidationIssue>();
    generator.subschema_for::<FlowValidationResult>();
    json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$defs": generator.definitions()})
}
