//! Host-owned datatype, token and validation registrations.
use crate::{
    FlowDocument, FlowNodeDefinition, FlowParameterDataType, FlowParameterEditorKind,
    FlowValidationIssue,
};
use anyhow::{bail, ensure, Result};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

type AutomationConversion = dyn Fn(&Value) -> Result<Value, String> + Send + Sync;
type ValueCheck = dyn Fn(&Value) -> Result<(), String> + Send + Sync;
type DefinitionCheck = dyn Fn(&FlowNodeDefinition) -> Result<(), String> + Send + Sync;
type GraphCheck = dyn Fn(&FlowDocument, &HashMap<String, FlowNodeDefinition>) -> Vec<FlowValidationIssue>
    + Send
    + Sync;

/// A schema and optional semantic validation for a host datatype.
pub struct ParameterType {
    pub schema: Value,
    pub default_editor: Option<FlowParameterEditorKind>,
    validate: Arc<ValueCheck>,
    automation: Arc<AutomationConversion>,
}
impl ParameterType {
    pub fn new(
        schema: Value,
        default_editor: Option<FlowParameterEditorKind>,
        validate: impl Fn(&Value) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            schema,
            default_editor,
            validate: Arc::new(validate),
            automation: Arc::new(|value| Ok(value.clone())),
        }
    }
    pub fn with_automation_conversion(
        mut self,
        conversion: impl Fn(&Value) -> Result<Value, String> + Send + Sync + 'static,
    ) -> Self {
        self.automation = Arc::new(conversion);
        self
    }
}
struct RegisteredParameter {
    contract: ParameterType,
    schema: jsonschema::Validator,
}

#[derive(Default)]
pub struct Registry {
    parameters: HashMap<String, RegisteredParameter>,
    tokens: HashMap<String, (Value, jsonschema::Validator)>,
    handlers: HashSet<String>,
    definition_checks: Vec<Arc<DefinitionCheck>>,
    graph_checks: Vec<Arc<GraphCheck>>,
}
impl Registry {
    pub fn register_handler(&mut self, name: &str) -> Result<()> {
        ensure!(!name.trim().is_empty(), "A handler name is required");
        ensure!(
            self.handlers.insert(name.into()),
            "Handler {name} is already registered"
        );
        Ok(())
    }
    pub fn register_parameter_type(&mut self, name: &str, contract: ParameterType) -> Result<()> {
        ensure!(!name.trim().is_empty(), "A datatype name is required");
        ensure!(
            !matches!(name, "boolean" | "int" | "string" | "list" | "map"),
            "Datatype {name} is reserved"
        );
        ensure!(
            !self.parameters.contains_key(name),
            "Datatype {name} is already registered"
        );
        let schema = jsonschema::validator_for(&contract.schema)?;
        self.parameters
            .insert(name.into(), RegisteredParameter { contract, schema });
        Ok(())
    }
    pub fn register_token_type(&mut self, name: &str, schema: Value) -> Result<()> {
        ensure!(
            !name.trim().is_empty() && !matches!(name, "auto" | "oneof"),
            "A concrete token name is required"
        );
        ensure!(
            !self.tokens.contains_key(name),
            "Token {name} is already registered"
        );
        let validator = jsonschema::validator_for(&schema)?;
        self.tokens.insert(name.into(), (schema, validator));
        Ok(())
    }
    pub fn register_definition_validator(
        &mut self,
        check: impl Fn(&FlowNodeDefinition) -> Result<(), String> + Send + Sync + 'static,
    ) {
        self.definition_checks.push(Arc::new(check));
    }
    pub fn register_graph_validator(
        &mut self,
        check: impl Fn(&FlowDocument, &HashMap<String, FlowNodeDefinition>) -> Vec<FlowValidationIssue>
            + Send
            + Sync
            + 'static,
    ) {
        self.graph_checks.push(Arc::new(check));
    }
    pub fn parameter_schema(&self, name: &str) -> Option<&Value> {
        self.parameters
            .get(name)
            .map(|entry| &entry.contract.schema)
    }
    pub fn token_schema(&self, name: &str) -> Option<&Value> {
        self.tokens.get(name).map(|entry| &entry.0)
    }
    pub fn validate_token(&self, name: &str, value: &Value) -> Result<()> {
        let (_, schema) = self
            .tokens
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Unknown token type {name}"))?;
        schema
            .validate(value)
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    }
    pub fn validate_datatype(&self, datatype: &FlowParameterDataType) -> Result<()> {
        match datatype.kind.as_str() {
            "list" => {
                ensure!(
                    datatype.value_type.is_none(),
                    "A list cannot declare value_type"
                );
                self.validate_datatype(
                    datatype
                        .item_type
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("A list requires item_type"))?,
                )
            }
            "map" => {
                ensure!(
                    datatype.item_type.is_none(),
                    "A map cannot declare item_type"
                );
                self.validate_datatype(
                    datatype
                        .value_type
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("A map requires value_type"))?,
                )
            }
            name => {
                ensure!(
                    datatype.item_type.is_none() && datatype.value_type.is_none(),
                    "Only list and map datatypes declare nested types"
                );
                ensure!(
                    matches!(name, "boolean" | "int" | "string")
                        || self.parameters.contains_key(name),
                    "Unknown datatype {name}"
                );
                Ok(())
            }
        }
    }
    pub fn validate_value(&self, datatype: &FlowParameterDataType, value: &Value) -> Result<()> {
        self.validate_datatype(datatype)?;
        match datatype.kind.as_str() {
            "boolean" if value.is_boolean() => Ok(()),
            "int" if value.as_i64().is_some() => Ok(()),
            "string" if value.is_string() => Ok(()),
            "list" if value.is_array() => {
                for item in value.as_array().unwrap() {
                    self.validate_value(datatype.item_type.as_deref().unwrap(), item)?;
                }
                Ok(())
            }
            "map" if value.is_object() => {
                for item in value.as_object().unwrap().values() {
                    self.validate_value(datatype.value_type.as_deref().unwrap(), item)?;
                }
                Ok(())
            }
            name => {
                let Some(entry) = self.parameters.get(name) else {
                    bail!("value does not match its declared datatype");
                };
                entry
                    .schema
                    .validate(value)
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                (entry.contract.validate)(value).map_err(anyhow::Error::msg)
            }
        }
    }

    pub fn convert_automation_value(
        &self,
        parameter: &crate::FlowParameterDefinition,
        value: &Value,
    ) -> Result<Value> {
        fn convert(
            registry: &Registry,
            datatype: &FlowParameterDataType,
            value: &Value,
        ) -> Result<Value> {
            registry.validate_datatype(datatype)?;
            match datatype.kind.as_str() {
                "list" => {
                    let values = match value {
                        Value::Null => vec![],
                        Value::Array(values) => values.clone(),
                        value => vec![value.clone()],
                    };
                    values
                        .iter()
                        .map(|v| convert(registry, datatype.item_type.as_deref().unwrap(), v))
                        .collect::<Result<Vec<_>>>()
                        .map(Value::Array)
                }
                "map" if value.is_object() => value
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(key, value)| {
                        convert(registry, datatype.value_type.as_deref().unwrap(), value)
                            .map(|v| (key.clone(), v))
                    })
                    .collect::<Result<serde_json::Map<_, _>>>()
                    .map(Value::Object),
                name => match registry.parameters.get(name) {
                    Some(entry) => (entry.contract.automation)(value).map_err(anyhow::Error::msg),
                    None => Ok(value.clone()),
                },
            }
        }
        let converted = convert(self, &parameter.datatype, value)?;
        if let Some(error) =
            crate::validation::validate_parameter_value(parameter, &converted, self)
        {
            bail!("{error}");
        }
        Ok(converted)
    }
    pub(crate) fn default_editor(
        &self,
        datatype: &FlowParameterDataType,
    ) -> Option<FlowParameterEditorKind> {
        match datatype.kind.as_str() {
            "boolean" => Some(FlowParameterEditorKind::Boolean),
            "string" => Some(FlowParameterEditorKind::String),
            "list" => Some(FlowParameterEditorKind::List),
            name => self
                .parameters
                .get(name)
                .and_then(|entry| entry.contract.default_editor.clone()),
        }
    }
    pub(crate) fn has_handler(&self, name: &str) -> bool {
        self.handlers.contains(name)
    }
    pub(crate) fn has_token(&self, name: &str) -> bool {
        matches!(name, "auto" | "oneof") || self.tokens.contains_key(name)
    }
    pub(crate) fn validate_definition(&self, definition: &FlowNodeDefinition) -> Result<()> {
        for check in &self.definition_checks {
            check(definition).map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
    pub(crate) fn validate_graph(
        &self,
        flow: &FlowDocument,
        definitions: &HashMap<String, FlowNodeDefinition>,
    ) -> Vec<FlowValidationIssue> {
        self.graph_checks
            .iter()
            .flat_map(|check| check(flow, definitions))
            .collect()
    }
}

#[cfg(test)]
mod automation_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn automation_conversion_obeys_the_registered_element_contract_and_list_bounds() {
        let mut registry = Registry::default();
        registry
            .register_parameter_type(
                "label",
                ParameterType::new(json!({"type":"string","minLength":1}), None, |_| Ok(()))
                    .with_automation_conversion(|value| {
                        value
                            .as_str()
                            .map(|s| Value::String(s.trim().into()))
                            .ok_or_else(|| "A label must be text".into())
                    }),
            )
            .unwrap();
        let parameter: crate::FlowParameterDefinition = serde_json::from_value(json!({
            "name":"labels", "datatype":{"kind":"list","item_type":{"kind":"label"}}, "editor":{"kind":"list","max":2}
        })).unwrap();
        let label = "authored label";
        assert_eq!(
            registry
                .convert_automation_value(&parameter, &json!(format!("  {label}  ")))
                .unwrap(),
            json!([label])
        );
        assert_eq!(
            registry
                .convert_automation_value(&parameter, &Value::Null)
                .unwrap(),
            json!([])
        );
        for invalid in [json!(42), json!(" "), json!([label, label, label])] {
            assert!(registry
                .convert_automation_value(&parameter, &invalid)
                .is_err());
        }
    }
}
