//! Copyright (c) Scott A Dixon
//! Discovery, host profile assignment and immutable remote definitions.
use crate::{wire, McpConnection, McpError};
use async_trait::async_trait;
use battersea_model::{
    local_tool_name, EngineAdapterRequestError, EngineLocalToolCall, EngineLocalToolExecutor,
    EngineLocalToolResult, ErrorKind, ToolCallEvidence, ToolCallOutcome, ToolContentBlock,
    ToolContentResult, ToolResourceContents,
};
use battersea_tools::{Tool, ToolRegistry, PROFILES_META_KEY};
use rmcp::{model::*, service::ServiceError};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{atomic::Ordering, Arc},
};
use tokio::time::{timeout_at, Instant};

pub(crate) struct Published {
    epoch: u64,
    records: BTreeMap<String, Value>,
}

/// A refused catalogue entry. Messages contain no remote schema or metadata values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpDiagnostic {
    pub index: usize,
    pub name: Option<String>,
    pub error: McpError,
}

/// Bounded discovery snapshot. Original records are available explicitly as host provenance.
pub struct McpCatalogue {
    connection: McpConnection,
    revision: u64,
    epoch: u64,
    records: BTreeMap<String, Value>,
    admitted: BTreeMap<String, Tool>,
    diagnostics: Vec<McpDiagnostic>,
}
impl McpCatalogue {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn diagnostics(&self) -> &[McpDiagnostic] {
        &self.diagnostics
    }
    pub fn remote_names(&self) -> impl Iterator<Item = &str> {
        self.admitted.keys().map(String::as_str)
    }
    pub fn provenance(&self, name: &str) -> Option<&Value> {
        self.records.get(name)
    }
    pub(crate) async fn discover(connection: McpConnection) -> Result<Self, McpError> {
        connection.check()?;
        let deadline = Instant::now() + connection.0.config.operation_timeout;
        let _ticket = connection
            .0
            .admitted
            .try_acquire()
            .map_err(|_| McpError::Busy)?;
        let mut cancel_on_drop = None;
        let work = async {
            let _serial = connection.0.discovery.lock().await;
            let _active = connection
                .0
                .active
                .acquire()
                .await
                .map_err(|_| McpError::Transport)?;
            // Raw catalogue requests bypass the SDK cache. Close on abandonment so their
            // request state cannot outlive the operation and accumulate across cancelled refreshes.
            cancel_on_drop = Some(connection.0.state.stop.clone().drop_guard());
            // Invalidate before I/O. Any failed refresh leaves prior bindings stale.
            let epoch = connection.0.state.epoch.fetch_add(1, Ordering::SeqCst) + 1;
            let limits = &connection.0.config.limits;
            let mut cursor = None;
            let mut cursors = BTreeSet::new();
            let mut records = BTreeMap::new();
            let mut admitted = BTreeMap::new();
            let mut duplicates = BTreeSet::new();
            let mut diagnostics = Vec::new();
            let mut bytes = 0usize;
            let mut count = 0;
            for page in 0..limits.pages {
                let request = serde_json::from_value(
                    json!({"method":"tools/list","params":{"cursor":cursor}}),
                )
                .map_err(|_| McpError::Protocol)?;
                // Avoid the SDK's TTL cache and stale-on-error fallback during refresh.
                let response = connection
                    .0
                    .service
                    .peer()
                    .send_request(request)
                    .await
                    .map_err(McpError::from_service)?;
                let value = serde_json::to_value(response).map_err(|_| McpError::Protocol)?;
                let value = value.get("batterseaCatalogue").unwrap_or(&value);
                bytes = bytes
                    .checked_add(wire::encode(value, limits.catalogue_bytes)?.len())
                    .ok_or(McpError::Limit)?;
                if bytes > limits.catalogue_bytes {
                    return Err(McpError::Limit);
                }
                let tools = value
                    .get("tools")
                    .and_then(Value::as_array)
                    .ok_or(McpError::Protocol)?;
                for raw in tools {
                    count += 1;
                    if count > limits.tools {
                        return Err(McpError::Limit);
                    }
                    let name = raw.get("name").and_then(Value::as_str).map(str::to_owned);
                    let admission = (|| {
                        let name = name.as_ref().ok_or(McpError::Selection)?;
                        if records.insert(name.clone(), raw.clone()).is_some() {
                            duplicates.insert(name.clone());
                            admitted.remove(name);
                            return Err(McpError::Selection);
                        }
                        if duplicates.contains(name) {
                            return Err(McpError::Selection);
                        }
                        let mut exposed = raw.clone();
                        if let Some(meta) = exposed.get_mut("_meta").and_then(Value::as_object_mut)
                        {
                            meta.remove(PROFILES_META_KEY);
                        }
                        let tool: Tool =
                            serde_json::from_value(exposed).map_err(|_| McpError::Selection)?;
                        wire::encode(
                            &(&tool.input_schema, &tool.output_schema),
                            limits.schema_bytes,
                        )?;
                        let mut registry = ToolRegistry::default();
                        registry
                            .register(tool.clone(), Arc::new(Admission))
                            .map_err(|_| McpError::Selection)?;
                        admitted.insert(name.clone(), tool);
                        Ok(())
                    })();
                    if let Err(error) = admission {
                        diagnostics.push(McpDiagnostic {
                            index: count - 1,
                            name,
                            error,
                        });
                    }
                }
                cursor = match value.get("nextCursor") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(cursor)) => Some(cursor.clone()),
                    _ => return Err(McpError::Protocol),
                };
                if let Some(cursor) = &cursor {
                    if !cursors.insert(cursor.clone()) || page + 1 == limits.pages {
                        return Err(McpError::Limit);
                    }
                } else {
                    break;
                }
            }
            if epoch != connection.0.state.epoch.load(Ordering::SeqCst) {
                return Err(McpError::Stale);
            }
            *connection.0.published.lock().unwrap() = Some(Published {
                epoch,
                records: records.clone(),
            });
            let revision = connection.0.revision.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(Self {
                connection: connection.clone(),
                revision,
                epoch,
                records,
                admitted,
                diagnostics,
            })
        };
        let result = timeout_at(deadline, work).await;
        if let Some(guard) = cancel_on_drop {
            guard.disarm();
        }
        match result {
            Ok(result) => result,
            Err(_) => {
                connection.0.state.fail(McpError::Timeout);
                Err(McpError::Timeout)
            }
        }
    }
    /// Bind selected records and check aliases against all tools already registered by the host.
    pub fn bind(
        &self,
        selections: Vec<McpSelection>,
        existing: &ToolRegistry,
    ) -> Result<McpToolBinding, McpError> {
        if self.epoch != self.connection.0.state.epoch.load(Ordering::SeqCst) {
            return Err(McpError::Stale);
        }
        if selections.is_empty() || selections.len() > self.connection.0.config.limits.tools {
            return Err(McpError::Selection);
        }
        let mut tools = Vec::new();
        let mut aliases = BTreeMap::new();
        let mut pinned = BTreeMap::new();
        for selection in selections {
            let mut tool = self
                .admitted
                .get(&selection.remote_name)
                .cloned()
                .ok_or(McpError::Selection)?;
            tool.name = selection.alias.unwrap_or_else(|| {
                format!("{}.{}", self.connection.server_id(), selection.remote_name)
            });
            tool.meta
                .insert(PROFILES_META_KEY.to_owned(), json!(selection.profiles));
            if aliases
                .insert(local_tool_name(&tool.name), selection.remote_name.clone())
                .is_some()
                || pinned
                    .insert(
                        selection.remote_name.clone(),
                        self.records[&selection.remote_name].clone(),
                    )
                    .is_some()
            {
                return Err(McpError::Selection);
            }
            tools.push(tool);
        }
        let remote = Arc::new(Remote {
            connection: self.connection.clone(),
            aliases,
            pinned,
        });
        let mut combined = existing.clone();
        let mut registry = ToolRegistry::default();
        for tool in &tools {
            combined
                .register(tool.clone(), remote.clone())
                .map_err(|_| McpError::Selection)?;
            registry
                .register(tool.clone(), remote.clone())
                .map_err(|_| McpError::Selection)?;
        }
        let revision = battersea_model::configuration_digest(&json!({
            "server": self.connection.server_id(),
            "credentialScope": self.connection.credential_scope_revision(),
            "definitions": remote.pinned,
            "aliases": remote.aliases,
            "tools": tools,
        }));
        Ok(McpToolBinding {
            revision,
            remote,
            tools,
            executor: McpToolExecutor(registry),
        })
    }
}

/// Host-selected remote tool, alias and permitted profiles.
#[derive(Clone, Debug)]
pub struct McpSelection {
    pub remote_name: String,
    pub alias: Option<String>,
    pub profiles: Vec<String>,
}
impl McpSelection {
    pub fn new(remote_name: impl Into<String>, profiles: Vec<String>) -> Self {
        Self {
            remote_name: remote_name.into(),
            alias: None,
            profiles,
        }
    }
}

/// An accepted catalogue revision and its immutable aliases, definitions and credential scope.
pub struct McpToolBinding {
    revision: String,
    remote: Arc<Remote>,
    tools: Vec<Tool>,
    executor: McpToolExecutor,
}
impl McpToolBinding {
    pub fn revision(&self) -> &str {
        &self.revision
    }
    pub fn credential_scope_revision(&self) -> &str {
        self.remote.connection.credential_scope_revision()
    }
    pub fn tools(&self) -> &[Tool] {
        &self.tools
    }
    pub fn executor(&self) -> McpToolExecutor {
        self.executor.clone()
    }
    /// Register atomically; collisions leave the host registry unchanged.
    pub fn register_into(&self, registry: &mut ToolRegistry) -> Result<(), McpError> {
        let mut next = registry.clone();
        for tool in &self.tools {
            next.register(tool.clone(), self.remote.clone())
                .map_err(|_| McpError::Selection)?;
        }
        *registry = next;
        Ok(())
    }
}

/// Registry-backed executor; validates arguments and successful output before delivery.
#[derive(Clone)]
pub struct McpToolExecutor(ToolRegistry);
#[allow(clippy::double_must_use)]
#[async_trait]
impl EngineLocalToolExecutor for McpToolExecutor {
    async fn call(
        &self,
        call: EngineLocalToolCall,
    ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
        self.0.call(call).await
    }
}
struct Admission;
#[allow(clippy::double_must_use)]
#[async_trait]
impl EngineLocalToolExecutor for Admission {
    async fn call(
        &self,
        _: EngineLocalToolCall,
    ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
        Err(EngineAdapterRequestError::new(
            "mcp",
            "Catalogue admission executor",
            ErrorKind::InvalidRequest,
        ))
    }
}
struct Remote {
    connection: McpConnection,
    aliases: BTreeMap<String, String>,
    pinned: BTreeMap<String, Value>,
}
impl Remote {
    fn check(&self) -> Result<(), McpError> {
        self.connection.check()?;
        let published = self.connection.0.published.lock().unwrap();
        let published = published.as_ref().ok_or(McpError::Stale)?;
        if published.epoch != self.connection.0.state.epoch.load(Ordering::SeqCst)
            || self
                .pinned
                .iter()
                .any(|(name, record)| published.records.get(name) != Some(record))
        {
            return Err(McpError::Stale);
        }
        Ok(())
    }
}
fn invocation_error(
    error: McpError,
    call: &EngineLocalToolCall,
    outcome: ToolCallOutcome,
) -> EngineAdapterRequestError {
    let kind = match error {
        McpError::Timeout => ErrorKind::Timeout,
        McpError::Limit if outcome != ToolCallOutcome::NotDispatched => ErrorKind::InvalidResponse,
        McpError::Transport => ErrorKind::Transport,
        McpError::Invocation(_) | McpError::Protocol | McpError::UnsupportedInteraction => {
            ErrorKind::InvalidResponse
        }
        _ => ErrorKind::InvalidRequest,
    };
    EngineAdapterRequestError::new("mcp", error.to_string(), kind).with_tool_call(
        ToolCallEvidence {
            call_id: call.id.clone(),
            tool_name: call.name.clone(),
            outcome,
            request_id: None,
        },
    )
}
#[allow(clippy::double_must_use)]
#[async_trait]
impl EngineLocalToolExecutor for Remote {
    async fn call(
        &self,
        call: EngineLocalToolCall,
    ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
        let deadline = Instant::now() + self.connection.0.config.operation_timeout;
        let refused = |error| invocation_error(error, &call, ToolCallOutcome::NotDispatched);
        self.check().map_err(refused)?;
        let name = self
            .aliases
            .get(&local_tool_name(&call.name))
            .ok_or_else(|| refused(McpError::Selection))?;
        let arguments = call
            .arguments
            .as_object()
            .cloned()
            .ok_or_else(|| refused(McpError::Selection))?;
        let params = CallToolRequestParams::new(name.clone()).with_arguments(arguments);
        // Reserve room for the SDK's per-request metadata and JSON-RPC envelope.
        wire::encode(
            &params,
            self.connection
                .0
                .config
                .limits
                .message_bytes
                .saturating_sub(4096),
        )
        .map_err(refused)?;
        let _ticket = self
            .connection
            .0
            .admitted
            .try_acquire()
            .map_err(|_| refused(McpError::Busy))?;
        let _active = timeout_at(deadline, self.connection.0.active.acquire())
            .await
            .map_err(|_| refused(McpError::Timeout))?
            .map_err(|_| refused(McpError::Transport))?;
        self.check().map_err(refused)?;
        let response = timeout_at(
            deadline,
            self.connection.0.service.peer().call_tool_once(params),
        )
        .await;
        let response = match response {
            Err(_) => {
                return Err(invocation_error(
                    McpError::Timeout,
                    &call,
                    ToolCallOutcome::Uncertain,
                ))
            }
            Ok(Err(error)) => {
                let outcome = if matches!(
                    error,
                    ServiceError::McpError(_) | ServiceError::UnexpectedResponse
                ) {
                    ToolCallOutcome::Completed
                } else {
                    ToolCallOutcome::Uncertain
                };
                let error = self
                    .connection
                    .0
                    .state
                    .check()
                    .err()
                    .unwrap_or_else(|| McpError::from_service(error));
                return Err(invocation_error(error, &call, outcome));
            }
            Ok(Ok(response)) => response,
        };
        match response {
            CallToolResponse::Complete(result) => {
                convert(result).map_err(|e| invocation_error(e, &call, ToolCallOutcome::Completed))
            }
            _ => Err(invocation_error(
                McpError::UnsupportedInteraction,
                &call,
                ToolCallOutcome::Uncertain,
            )),
        }
    }
}
fn convert(result: CallToolResult) -> Result<EngineLocalToolResult, McpError> {
    let mut blocks = Vec::new();
    for content in result.content {
        let raw = serde_json::to_value(content).map_err(|_| McpError::Protocol)?;
        let string = |key: &str| {
            raw.get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or(McpError::Protocol)
        };
        blocks.push(match raw["type"].as_str() {
            Some("text") => ToolContentBlock::Text {
                text: string("text")?,
            },
            Some("image") | Some("audio") => {
                let mime_type = string("mimeType")?;
                let url = format!("data:{mime_type};base64,{}", string("data")?);
                if raw["type"] == "image" {
                    ToolContentBlock::Image { url, mime_type }
                } else {
                    ToolContentBlock::Audio { url, mime_type }
                }
            }
            Some("resource_link") => ToolContentBlock::ResourceLink {
                uri: string("uri")?,
                name: string("name")?,
                mime_type: raw["mimeType"].as_str().map(str::to_owned),
            },
            Some("resource") => {
                let resource = &raw["resource"];
                let contents = if let Some(text) = resource["text"].as_str() {
                    ToolResourceContents::Text(text.to_owned())
                } else if let Some(blob) = resource["blob"].as_str() {
                    ToolResourceContents::Base64(blob.to_owned())
                } else {
                    return Err(McpError::Protocol);
                };
                ToolContentBlock::Resource {
                    uri: resource["uri"]
                        .as_str()
                        .ok_or(McpError::Protocol)?
                        .to_owned(),
                    mime_type: resource["mimeType"].as_str().map(str::to_owned),
                    contents,
                }
            }
            _ => return Err(McpError::UnsupportedInteraction),
        });
    }
    Ok(EngineLocalToolResult::Content(ToolContentResult {
        content_blocks: blocks,
        structured_content: result.structured_content,
        is_error: result.is_error.unwrap_or(false),
        meta: result.meta.map(|m| m.0).unwrap_or_default(),
    }))
}
