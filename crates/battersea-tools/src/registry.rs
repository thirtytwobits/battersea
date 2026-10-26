//! The registry that offers a Clapham selection to providers and runs their calls.
use crate::{Tool, PROFILES_META_KEY};
use async_trait::async_trait;
use battersea_model::{
    local_tool_name, EngineAdapterRequestError, EngineLocalToolCall, EngineLocalToolDefinition,
    EngineLocalToolExecutor, EngineLocalToolResult, ErrorKind, ToolCallEvidence, ToolCallOutcome,
    ToolCode, ToolContentBlock, ToolContentResult, ToolResourceContents,
};
use clapham::{
    Cancellation, ComposedSet, Evidence, OAuthContext, Outcome, RecordSink, Refusal, Selection,
};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

struct OfferedTool {
    tool: Tool,
    definition: EngineLocalToolDefinition,
}

/// The composed set, selection and verified grant every call of a registry runs under.
struct Dispatch {
    set: ComposedSet,
    selection: Selection,
    auth: OAuthContext,
}

/// The operations of one Clapham selection, offered to providers as tools and run under one
/// verified OAuth grant.
///
/// A provider is offered each selected operation under its provider-facing name,
/// [`local_tool_name`] of the operation's name. [`ToolRegistry::tools`] presents each as
/// [`Tool::for_operation`] does, listing the profiles that select it. A call is validated
/// against the operation's input schema, then dispatched through the composed set, which checks
/// the grant's scopes, the gate and the resource policies, runs the handler and validates
/// successful output.
#[derive(Clone)]
pub struct ToolRegistry {
    dispatch: Arc<Dispatch>,
    profiles: Arc<BTreeMap<String, BTreeSet<String>>>,
    tools: BTreeMap<String, Arc<OfferedTool>>,
}

pub(crate) fn refusal(message: impl Into<String>, code: ToolCode) -> EngineAdapterRequestError {
    EngineAdapterRequestError::new("tools", message, ErrorKind::InvalidRequest).with_tool_code(code)
}

/// The tool runner's code for a refusal of Clapham's call runner or of an operation.
fn tool_code(code: &str) -> Option<ToolCode> {
    match code {
        "invalid_input" => Some(ToolCode::InvalidToolArguments),
        "invalid_result" => Some(ToolCode::InvalidToolResult),
        "unknown_operation" => Some(ToolCode::UnregisteredTool),
        code => ToolCode::ALL
            .iter()
            .copied()
            .find(|tool_code| tool_code.as_str() == code),
    }
}

/// A Clapham refusal as the tool loop reports it, with its evidence.
///
/// A code the tool runner names keeps the registry's `invalid_request` classification and sets
/// `tool_code`; any other code is the classification, unchanged.
pub(crate) fn refused(
    refusal: Refusal,
    mut evidence: ToolCallEvidence,
) -> EngineAdapterRequestError {
    evidence.outcome = match refusal.evidence {
        Evidence::NotDispatched => ToolCallOutcome::NotDispatched,
        Evidence::Completed => ToolCallOutcome::Completed,
        Evidence::Uncertain => ToolCallOutcome::Uncertain,
    };
    let error = match tool_code(&refusal.code) {
        Some(code) => crate::registry::refusal(refusal.message, code),
        None => EngineAdapterRequestError::new("tools", refusal.message, refusal.code),
    };
    error.with_tool_call(evidence)
}

impl ToolRegistry {
    /// Offers `selection`, chosen from `set`, under the verified grant `auth`.
    ///
    /// Refuses two operations whose provider-facing names are the same as
    /// [`ToolCode::DuplicateTool`]. Clapham's composition has already refused a name outside the
    /// tool-name alphabet, a missing description, an input schema without an object root and a
    /// schema that does not compile.
    pub fn new(
        set: ComposedSet,
        selection: Selection,
        auth: OAuthContext,
    ) -> Result<Self, EngineAdapterRequestError> {
        let mut seen = BTreeSet::new();
        for operation in selection.operations() {
            if !seen.insert(local_tool_name(operation.name.as_str())) {
                return Err(refusal(
                    format!("Tool \"{}\" collides with a selected tool.", operation.name),
                    ToolCode::DuplicateTool,
                ));
            }
        }
        let mut registry = Self {
            dispatch: Arc::new(Dispatch {
                set,
                selection,
                auth,
            }),
            profiles: Arc::default(),
            tools: BTreeMap::new(),
        };
        registry.offer();
        Ok(registry)
    }

    /// The registry, with the agent profile `profile` permitted to call the operations `names`.
    ///
    /// Refuses a blank profile, a profile already named, and a name outside the selection as
    /// [`ToolCode::InvalidToolDefinition`]. An operation no profile names is permitted to none.
    pub fn with_profile<I, S>(
        mut self,
        profile: impl Into<String>,
        names: I,
    ) -> Result<Self, EngineAdapterRequestError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let profile = profile.into();
        if profile.trim().is_empty() || self.profiles.contains_key(&profile) {
            return Err(refusal(
                format!("Profile \"{profile}\" is blank or already named."),
                ToolCode::InvalidToolDefinition,
            ));
        }
        let names = names.into_iter().map(Into::into).collect::<BTreeSet<_>>();
        if let Some(name) = names
            .iter()
            .find(|name| !self.dispatch.selection.contains(name))
        {
            return Err(refusal(
                format!("Profile \"{profile}\" names \"{name}\", which is not selected."),
                ToolCode::InvalidToolDefinition,
            ));
        }
        Arc::make_mut(&mut self.profiles).insert(profile, names);
        self.offer();
        Ok(self)
    }

    fn offer(&mut self) {
        self.tools = self
            .dispatch
            .selection
            .operations()
            .map(|operation| {
                let mut tool = Tool::for_operation(operation);
                let profiles = self
                    .profiles
                    .iter()
                    .filter(|(_, names)| names.contains(operation.name.as_str()))
                    .map(|(profile, _)| Value::from(profile.as_str()))
                    .collect::<Vec<_>>();
                if !profiles.is_empty() {
                    tool.meta
                        .insert(PROFILES_META_KEY.to_string(), Value::from(profiles));
                }
                let provider_name = local_tool_name(&tool.name);
                let definition = EngineLocalToolDefinition {
                    name: provider_name.clone(),
                    description: operation.description.clone(),
                    input_schema: Value::Object(operation.input.clone()),
                };
                (provider_name, Arc::new(OfferedTool { tool, definition }))
            })
            .collect();
    }

    /// The selection the registry offers.
    pub fn selection(&self) -> &Selection {
        &self.dispatch.selection
    }

    /// The selection's revision: its descriptors and the remote binding revisions it includes.
    pub fn revision(&self) -> &str {
        self.dispatch.selection.revision()
    }

    /// The offered tools, by provider-facing name.
    pub fn tools(&self) -> impl Iterator<Item = &Tool> {
        self.tools.values().map(|offered| &offered.tool)
    }

    /// The offered tools as a provider is offered them.
    pub fn definitions(&self) -> Vec<EngineLocalToolDefinition> {
        self.tools
            .values()
            .map(|offered| offered.definition.clone())
            .collect()
    }

    /// The registry over the operations of its selection that the agent profile `profile` is
    /// permitted to call. Narrowing never restores an operation the selection excludes.
    pub fn for_profile(&self, profile: &str) -> Self {
        let names = self
            .profiles
            .get(profile)
            .into_iter()
            .flatten()
            .filter(|name| self.dispatch.selection.contains(name));
        let selection = self
            .dispatch
            .set
            .select(names)
            .expect("a profile names composed operations");
        let mut registry = Self {
            dispatch: Arc::new(Dispatch {
                set: self.dispatch.set.clone(),
                selection,
                auth: self.dispatch.auth.clone(),
            }),
            profiles: self.profiles.clone(),
            tools: BTreeMap::new(),
        };
        registry.offer();
        registry
    }

    pub(crate) fn prepare(
        &self,
        mut call: EngineLocalToolCall,
    ) -> Result<PreparedCall, EngineAdapterRequestError> {
        let mut evidence = ToolCallEvidence {
            call_id: call.id.clone(),
            tool_name: call.name.clone(),
            outcome: ToolCallOutcome::NotDispatched,
            request_id: None,
        };
        let unregistered = |evidence: &ToolCallEvidence| {
            refusal(
                format!("Tool \"{}\" is not registered.", call.name),
                ToolCode::UnregisteredTool,
            )
            .with_tool_call(evidence.clone())
        };
        let offered = self
            .tools
            .get(&local_tool_name(&call.name))
            .ok_or_else(|| unregistered(&evidence))?
            .clone();
        evidence.tool_name.clone_from(&offered.tool.name);
        self.dispatch
            .set
            .get(&offered.tool.name)
            .ok_or_else(|| unregistered(&evidence))?
            .validate_input(&call.arguments)
            .map_err(|error| refused(error, evidence.clone()))?;
        call.name.clone_from(&offered.tool.name);
        Ok(PreparedCall {
            dispatch: self.dispatch.clone(),
            offered,
            call,
            evidence,
        })
    }
}

// async-trait adds must_use to boxed futures, which already carry it.
#[allow(clippy::double_must_use)]
#[async_trait]
impl EngineLocalToolExecutor for ToolRegistry {
    /// Runs `call` through the composed set, under either the tool's own name or its
    /// provider-facing one. Missing or invalid successful output is refused as
    /// [`ToolCode::InvalidToolResult`]. Tool-reported errors bypass the success schema.
    async fn call(
        &self,
        call: EngineLocalToolCall,
    ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
        self.prepare(call)?.execute().await
    }
}

pub(crate) struct PreparedCall {
    dispatch: Arc<Dispatch>,
    offered: Arc<OfferedTool>,
    pub call: EngineLocalToolCall,
    pub evidence: ToolCallEvidence,
}
impl PreparedCall {
    pub fn tool(&self) -> &Tool {
        &self.offered.tool
    }

    pub fn validate_result(
        &self,
        result: &EngineLocalToolResult,
    ) -> Result<(), EngineAdapterRequestError> {
        let outcome = Outcome {
            result: result.structured_output().cloned(),
            content: None,
            is_error: result.is_error(),
            evidence: Evidence::Completed,
        };
        match self.dispatch.set.get(&self.call.name) {
            Some(operation) => operation
                .validate_output(&outcome)
                .map_err(|error| refused(error, self.evidence.clone())),
            None => Err(refusal(
                format!("Tool \"{}\" is not registered.", self.call.name),
                ToolCode::UnregisteredTool,
            )
            .with_tool_call(self.evidence.clone())),
        }
    }

    pub async fn execute(self) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
        let Self {
            dispatch,
            call,
            mut evidence,
            ..
        } = self;
        let outcome = dispatch
            .set
            .call(
                &dispatch.selection,
                dispatch.auth.clone(),
                &call.name,
                call.arguments,
                RecordSink::default(),
                Cancellation::new(),
                false,
            )
            .await
            .map_err(|error| refused(error, evidence.clone()))?;
        evidence.outcome = ToolCallOutcome::Completed;
        result(outcome).map_err(|error| error.with_tool_call(evidence))
    }
}

/// An outcome as the tool loop receives it. A completed JSON result with no content is
/// ordinary application JSON; anything else is a content result whose blocks are MCP
/// content blocks.
fn result(outcome: Outcome) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
    if let (Some(content), None, false) = (&outcome.result, &outcome.content, outcome.is_error) {
        return Ok(EngineLocalToolResult::json(content.clone()));
    }
    Ok(EngineLocalToolResult::Content(ToolContentResult {
        content_blocks: outcome
            .content
            .unwrap_or_default()
            .iter()
            .map(block)
            .collect::<Result<_, _>>()?,
        structured_content: outcome.result,
        is_error: outcome.is_error,
        meta: Map::new(),
    }))
}

fn block(raw: &Value) -> Result<ToolContentBlock, EngineAdapterRequestError> {
    let unsupported = || {
        EngineAdapterRequestError::new(
            "tools",
            "The tool returned a content block the tool loop cannot represent.",
            ErrorKind::InvalidResponse,
        )
        .with_tool_code(ToolCode::UnsupportedToolContent)
    };
    let string = |value: &Value, key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(unsupported)
    };
    Ok(match raw["type"].as_str() {
        Some("text") => ToolContentBlock::Text {
            text: string(raw, "text")?,
        },
        Some(kind @ ("image" | "audio")) => {
            let mime_type = string(raw, "mimeType")?;
            let url = format!("data:{mime_type};base64,{}", string(raw, "data")?);
            if kind == "image" {
                ToolContentBlock::Image { url, mime_type }
            } else {
                ToolContentBlock::Audio { url, mime_type }
            }
        }
        Some("resource_link") => ToolContentBlock::ResourceLink {
            uri: string(raw, "uri")?,
            name: string(raw, "name")?,
            mime_type: raw["mimeType"].as_str().map(str::to_owned),
        },
        Some("resource") => {
            let resource = &raw["resource"];
            let contents = if let Some(text) = resource["text"].as_str() {
                ToolResourceContents::Text(text.to_owned())
            } else {
                ToolResourceContents::Base64(string(resource, "blob")?)
            };
            ToolContentBlock::Resource {
                uri: string(resource, "uri")?,
                mime_type: resource["mimeType"].as_str().map(str::to_owned),
                contents,
            }
        }
        _ => return Err(unsupported()),
    })
}
