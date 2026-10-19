//! The registry that offers application tools to providers and runs their calls.
use crate::Tool;
use async_trait::async_trait;
use battersea_model::{
    local_tool_name, EngineAdapterRequestError, EngineLocalToolCall, EngineLocalToolDefinition,
    EngineLocalToolExecutor, EngineLocalToolResult, ErrorKind, ToolCode,
};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

/// The longest tool name the Model Context Protocol admits.
const MAX_NAME_LENGTH: usize = 128;

struct RegisteredTool {
    tool: Tool,
    definition: EngineLocalToolDefinition,
    validator: jsonschema::Validator,
    executor: Arc<dyn EngineLocalToolExecutor>,
}

/// Application tools, each run by the executor it was registered with.
///
/// A provider is offered each tool under its provider-facing name, [`local_tool_name`] of its
/// own. A call is validated against the tool's input schema before its executor runs, and the
/// executor receives it under the tool's own name.
#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<RegisteredTool>>,
}

fn refusal(message: impl Into<String>, code: ToolCode) -> EngineAdapterRequestError {
    EngineAdapterRequestError::new("tools", message, ErrorKind::InvalidRequest).with_tool_code(code)
}

fn portable_name(name: &str) -> bool {
    (1..=MAX_NAME_LENGTH).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

impl ToolRegistry {
    /// Registers `tool`, run by `executor`.
    ///
    /// Refuses a tool whose name is outside the protocol's tool-name alphabet, that has no
    /// description, whose input schema is not an object schema, or whose profiles entry is
    /// not a list of distinct, non-blank names, as [`ToolCode::InvalidToolDefinition`]; an
    /// input schema that does not compile as [`ToolCode::InvalidToolSchema`]; and a tool
    /// whose provider-facing name is already registered as [`ToolCode::DuplicateTool`].
    pub fn register(
        &mut self,
        tool: Tool,
        executor: Arc<dyn EngineLocalToolExecutor>,
    ) -> Result<(), EngineAdapterRequestError> {
        if !portable_name(&tool.name) {
            return Err(refusal(
                format!(
                    "Tool name \"{}\" is not 1 to {MAX_NAME_LENGTH} ASCII letters, digits, '_', '-' or '.'.",
                    tool.name
                ),
                ToolCode::InvalidToolDefinition,
            ));
        }
        // The description is prompt text: it reaches the provider exactly as written.
        let description = tool
            .description
            .clone()
            .filter(|description| !description.trim().is_empty())
            .ok_or_else(|| {
                refusal(
                    format!("Tool \"{}\" has no description.", tool.name),
                    ToolCode::InvalidToolDefinition,
                )
            })?;
        if tool.input_schema.get("type").and_then(Value::as_str) != Some("object") {
            return Err(refusal(
                format!(
                    "Tool \"{}\" has an input schema that is not an object schema.",
                    tool.name
                ),
                ToolCode::InvalidToolDefinition,
            ));
        }
        let profiles = tool.profiles().ok_or_else(|| {
            refusal(
                format!(
                    "Tool \"{}\" lists its profiles as something other than names.",
                    tool.name
                ),
                ToolCode::InvalidToolDefinition,
            )
        })?;
        let mut seen = std::collections::BTreeSet::new();
        if profiles
            .iter()
            .any(|profile| profile.trim().is_empty() || !seen.insert(*profile))
        {
            return Err(refusal(
                format!("Tool \"{}\" lists a blank or repeated profile.", tool.name),
                ToolCode::InvalidToolDefinition,
            ));
        }
        let provider_name = local_tool_name(&tool.name);
        if self.tools.contains_key(&provider_name) {
            return Err(refusal(
                format!("Tool \"{}\" collides with a registered tool.", tool.name),
                ToolCode::DuplicateTool,
            ));
        }
        let input_schema = Value::Object(tool.input_schema.clone());
        let validator = jsonschema::validator_for(&input_schema)
            .map_err(|error| refusal(error.to_string(), ToolCode::InvalidToolSchema))?;
        let definition = EngineLocalToolDefinition {
            name: provider_name.clone(),
            description,
            input_schema,
        };
        self.tools.insert(
            provider_name,
            Arc::new(RegisteredTool {
                tool,
                definition,
                validator,
                executor,
            }),
        );
        Ok(())
    }

    /// The registered tools, by provider-facing name.
    pub fn tools(&self) -> impl Iterator<Item = &Tool> {
        self.tools.values().map(|registered| &registered.tool)
    }

    /// The registered tools as a provider is offered them.
    pub fn definitions(&self) -> Vec<EngineLocalToolDefinition> {
        self.tools
            .values()
            .map(|registered| registered.definition.clone())
            .collect()
    }

    /// The registered tools that the agent profile `profile` is permitted to call.
    pub fn for_profile(&self, profile: &str) -> Self {
        Self {
            tools: self
                .tools
                .iter()
                .filter(|(_, registered)| registered.tool.permits(profile))
                .map(|(name, registered)| (name.clone(), Arc::clone(registered)))
                .collect(),
        }
    }
}

// async-trait adds must_use to boxed futures, which already carry it.
#[allow(clippy::double_must_use)]
#[async_trait]
impl EngineLocalToolExecutor for ToolRegistry {
    /// Runs `call` with the executor of the tool it names, under either the tool's own name or
    /// its provider-facing one.
    async fn call(
        &self,
        mut call: EngineLocalToolCall,
    ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
        let registered = self
            .tools
            .get(&local_tool_name(&call.name))
            .ok_or_else(|| {
                refusal(
                    format!("Tool \"{}\" is not registered.", call.name),
                    ToolCode::UnregisteredTool,
                )
            })?;
        registered
            .validator
            .validate(&call.arguments)
            .map_err(|error| refusal(error.to_string(), ToolCode::InvalidToolArguments))?;
        call.name.clone_from(&registered.tool.name);
        registered.executor.call(call).await
    }
}
