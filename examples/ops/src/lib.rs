//! Copyright (c) Scott A Dixon
//! A deterministic Battersea host: stored flows in memory, runs through the mock provider whose
//! tool calls wait in durable approval journals, and fixed MCP connections. Every fresh host
//! holds the same state, so each front end of the conformance matrix sees the same results.
use async_trait::async_trait;
use battersea_flow::{
    document::{
        canonicalize_document, inspect_document, load_document, serialize_canonical_document,
        DocumentInspection,
    },
    FlowDocument, FlowValidationResult,
};
use battersea_model::{engine::*, *};
use battersea_ops::{codes, *};
use battersea_tools::{
    DurableToolApprovalScope, DurableToolExecutor, StoredToolCallState, ToolApprovalJournal,
    ToolApprovalJournalLimits, ToolApprovalStore, ToolApprovalStoreError, ToolRegistry,
};
use clapham::{
    Bounds, Call, ComposeOptions, ComposedSet, Contribution, Effect, ExecutionMode, OAuthContext,
    OAuthResource, RecordSink, Refusal, TypedOperation,
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

pub const ISSUER: &str = "https://identity.example.com";
pub const AUDIENCE: &str = "https://battersea.example.com";

/// Every scope Battersea's operations require.
pub const SCOPES: &[&str] = &[
    "battersea:flows:read",
    "battersea:flows:admin",
    "battersea:runs:read",
    "battersea:runs:write",
    "battersea:runs:admin",
    "battersea:approvals:read",
    "battersea:approvals:decide",
    "battersea:approvals:admin",
    "battersea:mcp:read",
    "battersea:mcp:admin",
];

/// An operator's grant, holding every scope.
pub fn operator() -> OAuthContext {
    OAuthContext::from_verified(ISSUER, AUDIENCE, "operator", SCOPES.iter().copied())
}

/// The grant runs' agents call their tools under. Agent grants hold no administrative or
/// decision scope.
pub fn agent() -> OAuthContext {
    OAuthContext::from_verified(
        ISSUER,
        AUDIENCE,
        "agent",
        [
            "battersea:flows:read",
            "battersea:runs:read",
            "battersea:runs:write",
            "battersea:approvals:read",
        ],
    )
}

/// A fixed expiry, so request revisions are the same in every fresh host.
const EXPIRES_AT_UNIX_MS: u64 = 4_102_444_800_000;

/// The seeded run whose tool call waits for a decision.
pub const PENDING_RUN: &str = "run-1";
/// The seeded run that has finished.
pub const FINISHED_RUN: &str = "run-0";
/// The seeded run's tool call.
pub const PENDING_CALL: &str = "mock-tool-r1-0";

#[derive(Default)]
struct MemoryStore(Mutex<Option<ToolApprovalJournal>>);
#[async_trait]
impl ToolApprovalStore for MemoryStore {
    async fn load(
        &self,
        _: &str,
        _: usize,
    ) -> Result<Option<ToolApprovalJournal>, ToolApprovalStoreError> {
        Ok(self.0.lock().unwrap().clone())
    }
    async fn compare_exchange(
        &self,
        _: &str,
        expected: Option<u64>,
        next: &ToolApprovalJournal,
    ) -> Result<(), ToolApprovalStoreError> {
        let mut journal = self.0.lock().unwrap();
        if journal.as_ref().map(|journal| journal.revision) != expected {
            return Err(ToolApprovalStoreError::Conflict);
        }
        *journal = Some(next.clone());
        Ok(())
    }
}

#[derive(Clone)]
struct Run {
    status: String,
    terminal: bool,
    events: Vec<Value>,
}
impl Run {
    fn status(&self, run_id: &str) -> RunStatus {
        RunStatus {
            run_id: run_id.into(),
            status: self.status.clone(),
            terminal: self.terminal,
            detail: None,
        }
    }
}

/// One host's state.
pub struct Host {
    flows: Mutex<BTreeMap<String, String>>,
    runs: Mutex<BTreeMap<String, Run>>,
    journals: tokio::sync::Mutex<BTreeMap<String, Arc<DurableToolExecutor>>>,
    servers: Mutex<BTreeMap<String, McpServer>>,
    tools: ToolRegistry,
}

/// The operation the mock provider's tool call reaches, which writes the value it is given.
fn tools() -> ToolRegistry {
    #[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
    struct Write {
        value: String,
    }
    let write = TypedOperation::<Write, Write>::new(
        "fixture.write",
        "Write a value to the fixture.",
        Effect::Mutation,
    )
    .expect("a valid name");
    let set = clapham::compose(
        [Contribution::new("fixture")
            .typed_operation(write, |_: Call, input: Write| async move { Ok(input) })],
        Bounds::default(),
    )
    .expect("the fixture's tools compose");
    let selection = set.select(["fixture.write"]).expect("a composed name");
    ToolRegistry::new(set, selection, agent()).expect("a registry")
}

fn backend() -> EngineBackendConfig {
    EngineBackendConfig {
        binding: None,
        id: "fixture-chat".into(),
        provider: "mock".into(),
        label: "Fixture chat".into(),
        enabled: true,
        endpoint: String::new(),
        model: "fixture".into(),
        display_order: None,
        chat: EngineChatParameters::default_for_provider("mock"),
        capabilities: EngineBackendCapabilities::mock(),
        context_window_tokens: 8192,
        options: EngineBackendOptions {
            timeout_ms: Some(5000),
            mock_tool_calls: Some(vec![MockToolCallSpec {
                name: "fixture.write".into(),
                arguments: json!({"value": "recorded"}),
                round: 1,
            }]),
            mock_stream_delay_multiplier: Some(0.0),
            ..Default::default()
        },
        auth: EngineAuthConfig {
            auth_type: "none".into(),
            api_key_env: String::new(),
            header: None,
            version_header: None,
            version: None,
            has_api_key: false,
            api_key: None,
        },
        short_description: String::new(),
        long_description: String::new(),
    }
}

/// An execution event as the fixture records it, without its timestamps.
fn record(event: &EngineTextStreamEvent) -> Option<Value> {
    Some(match event {
        EngineTextStreamEvent::TextDelta { text } => json!({"kind": "text", "text": text}),
        EngineTextStreamEvent::ToolCallStarted { name, .. } => {
            json!({"kind": "tool_call_started", "name": name})
        }
        EngineTextStreamEvent::ToolCallCompleted { name, ok, .. } => {
            json!({"kind": "tool_call_completed", "name": name, "ok": ok})
        }
        _ => return None,
    })
}

fn unknown_run(run_id: &str) -> Refusal {
    codes::refusal(codes::UNKNOWN_RUN, format!("No run \"{run_id}\"."))
}

impl Host {
    pub fn new() -> Arc<Self> {
        let flows = BTreeMap::from([
            (
                "example".to_string(),
                include_str!("../fixtures/example.json").to_string(),
            ),
            (
                "future".to_string(),
                r#"{"version":999,"flow_key":"future","title":"Future document","nodes":[]}"#
                    .to_string(),
            ),
        ]);
        let finished = Run {
            status: "succeeded".into(),
            terminal: true,
            events: vec![
                json!({"kind": "text", "text": "Finished."}),
                json!({"kind": "outcome", "status": "succeeded"}),
            ],
        };
        let pending = Run {
            status: "awaiting_approval".into(),
            terminal: false,
            events: vec![json!({"kind": "tool_call_started", "name": "fixture_write"})],
        };
        let servers = BTreeMap::from([(
            "remote".to_string(),
            McpServer {
                server_id: "remote".into(),
                binding_revision: "binding-1".into(),
                credential_scope_revision: "account-1".into(),
            },
        )]);
        Arc::new(Self {
            flows: Mutex::new(flows),
            runs: Mutex::new(BTreeMap::from([
                (FINISHED_RUN.to_string(), finished),
                (PENDING_RUN.to_string(), pending),
            ])),
            journals: Default::default(),
            servers: Mutex::new(servers),
            tools: tools(),
        })
    }

    fn source(&self, flow_key: &str) -> Result<String, Refusal> {
        self.flows
            .lock()
            .unwrap()
            .get(flow_key)
            .cloned()
            .ok_or_else(|| codes::refusal(codes::UNKNOWN_FLOW, format!("No flow \"{flow_key}\".")))
    }

    async fn create_journal(&self, run_id: &str) -> Arc<DurableToolExecutor> {
        let executor = DurableToolExecutor::create(
            Arc::new(MemoryStore::default()),
            self.tools.clone(),
            DurableToolApprovalScope {
                run_id: run_id.into(),
                binding_revision: self.tools.revision().into(),
                expires_at_unix_ms: EXPIRES_AT_UNIX_MS,
            },
            ToolApprovalJournalLimits {
                max_calls: 8,
                max_bytes: 64 * 1024,
            },
            Duration::from_secs(5),
        )
        .await
        .expect("an empty journal");
        Arc::new(executor)
    }

    /// The journal of `run_id`. The pending run's journal is seeded with its waiting call.
    async fn journal(&self, run_id: &str) -> Result<Arc<DurableToolExecutor>, Refusal> {
        let mut journals = self.journals.lock().await;
        if let Some(journal) = journals.get(run_id) {
            return Ok(journal.clone());
        }
        if run_id != PENDING_RUN {
            return Err(unknown_run(run_id));
        }
        let journal = self.create_journal(run_id).await;
        journal
            .submit(
                EngineLocalToolCall {
                    id: PENDING_CALL.into(),
                    name: "fixture.write".into(),
                    arguments: json!({"value": "recorded"}),
                },
                1,
            )
            .await
            .expect("the seeded call is accepted");
        journals.insert(run_id.into(), journal.clone());
        Ok(journal)
    }

    /// Runs the activation of the reserved run `run_id` through the mock provider, whose tool
    /// call waits in the run's approval journal.
    async fn execute(
        &self,
        run_id: &str,
        activation: Activation,
        events: &RecordSink,
    ) -> Result<Run, Refusal> {
        let journal = self.create_journal(run_id).await;
        self.journals
            .lock()
            .await
            .insert(run_id.into(), journal.clone());
        let mut providers = ProviderRegistry::default();
        battersea_providers::register_builtin_providers(&mut providers)
            .map_err(|error| codes::refusal(codes::HOST_FAILURE, error.to_string()))?;
        let backend = backend();
        let adapter = providers
            .create_chat(backend.clone(), None)
            .map_err(|error| codes::refusal(codes::HOST_FAILURE, error.to_string()))?;
        let text = activation
            .parameters
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("Record the value.")
            .to_string();
        let mut stream = adapter
            .stream_text(EngineTextStreamRequest {
                shared: EngineAdapterRequest {
                    operation: EngineOperation::FlowTextStream,
                    messages: vec![Message::text(Role::User, text)],
                    chat: backend.chat,
                    debug_rules: None,
                },
                local_tools: journal.definitions(),
                local_tool_executor: Some(journal),
                max_tool_rounds: 2,
                mock_response: None,
            })
            .await
            .map_err(|error| codes::refusal(codes::HOST_UNAVAILABLE, error.message.to_string()))?;
        let mut run = Run {
            status: "succeeded".into(),
            terminal: true,
            events: Vec::new(),
        };
        while let Some(event) = stream.next().await {
            match event {
                Ok(event) => {
                    if let Some(record) = record(&event) {
                        events.send(record.clone()).await?;
                        run.events.push(record);
                    }
                }
                Err(error) => {
                    (run.status, run.terminal) =
                        if error.tool_code == Some(ToolCode::ToolApprovalPending) {
                            ("awaiting_approval".into(), false)
                        } else {
                            ("failed".into(), true)
                        };
                    break;
                }
            }
        }
        Ok(run)
    }

    fn plan(&self, flow_key: &str) -> Result<FlowUpgrade, Refusal> {
        let source = self.source(flow_key)?;
        let document = load_document(&source)
            .map_err(|error| codes::refusal(codes::INVALID_FLOW, error.to_string()))?;
        Ok(FlowUpgrade {
            flow_key: flow_key.into(),
            source_revision: configuration_digest(&json!({ "source": source })),
            document: canonicalize_document(&document),
            applied: false,
        })
    }
}

#[async_trait]
impl FlowHost for Host {
    async fn list(&self) -> Result<Vec<FlowSummary>, Refusal> {
        Ok(self
            .flows
            .lock()
            .unwrap()
            .iter()
            .map(|(flow_key, source)| match inspect_document(source) {
                Ok(DocumentInspection::Supported(flow)) => FlowSummary {
                    flow_key: flow_key.clone(),
                    title: flow.title,
                    unavailable_reason: None,
                },
                Ok(DocumentInspection::Incompatible { version, title, .. }) => FlowSummary {
                    flow_key: flow_key.clone(),
                    title,
                    unavailable_reason: Some(format!("Unsupported document version {version}")),
                },
                Err(error) => FlowSummary {
                    flow_key: flow_key.clone(),
                    title: flow_key.clone(),
                    unavailable_reason: Some(error.to_string()),
                },
            })
            .collect())
    }
    async fn get(&self, flow_key: &str) -> Result<FlowDocument, Refusal> {
        load_document(&self.source(flow_key)?)
            .map_err(|error| codes::refusal(codes::INVALID_FLOW, error.to_string()))
    }
    async fn validate(&self, flow_key: &str) -> Result<FlowValidationResult, Refusal> {
        self.get(flow_key).await?;
        Ok(FlowValidationResult {
            valid: true,
            issues: Vec::new(),
        })
    }
    async fn plan_upgrade(&self, flow_key: &str) -> Result<FlowUpgrade, Refusal> {
        self.plan(flow_key)
    }
    async fn apply_upgrade(
        &self,
        flow_key: &str,
        expected_revision: &str,
    ) -> Result<FlowUpgrade, Refusal> {
        let mut plan = self.plan(flow_key)?;
        if plan.source_revision != expected_revision {
            return Err(codes::refusal(
                codes::STALE_UPGRADE_REVISION,
                "The flow changed; preview the upgrade again.",
            ));
        }
        let upgraded = serialize_canonical_document(&plan.document)
            .map_err(|error| codes::refusal(codes::HOST_FAILURE, error.to_string()))?;
        self.flows.lock().unwrap().insert(flow_key.into(), upgraded);
        plan.applied = true;
        Ok(plan)
    }
}

#[async_trait]
impl RunHost for Host {
    async fn activate(
        &self,
        _: &OAuthContext,
        activation: Activation,
        events: &RecordSink,
    ) -> Result<RunStatus, Refusal> {
        let flow = self.get(&activation.flow_key).await?;
        if !flow.nodes.iter().any(|node| node.id == activation.node_id) {
            return Err(codes::refusal(
                battersea_runtime::error_codes::UNKNOWN_NODE,
                "The node is not in the flow.",
            ));
        }
        // Reserve the run's identity before the first await, so concurrent activations differ.
        let run_id = {
            let mut runs = self.runs.lock().unwrap();
            let run_id = format!("run-{}", runs.len());
            runs.insert(
                run_id.clone(),
                Run {
                    status: "running".into(),
                    terminal: false,
                    events: Vec::new(),
                },
            );
            run_id
        };
        let outcome = self.execute(&run_id, activation, events).await;
        let mut runs = self.runs.lock().unwrap();
        let reserved = runs.get_mut(&run_id).expect("the reserved run");
        // A cancellation that arrived during the activation stands.
        let cancelled = reserved.terminal;
        match outcome {
            Ok(run) if cancelled => {
                reserved.events = run.events;
                Ok(reserved.status(&run_id))
            }
            Ok(run) => {
                *reserved = run;
                Ok(reserved.status(&run_id))
            }
            Err(refusal) => {
                if !cancelled {
                    (reserved.status, reserved.terminal) = ("failed".into(), true);
                }
                Err(refusal)
            }
        }
    }
    async fn status(&self, run_id: &str) -> Result<RunStatus, Refusal> {
        self.runs
            .lock()
            .unwrap()
            .get(run_id)
            .map(|run| run.status(run_id))
            .ok_or_else(|| unknown_run(run_id))
    }
    async fn watch(&self, run_id: &str, events: &RecordSink) -> Result<RunStatus, Refusal> {
        let run = self
            .runs
            .lock()
            .unwrap()
            .get(run_id)
            .cloned()
            .ok_or_else(|| unknown_run(run_id))?;
        for event in &run.events {
            events.send(event.clone()).await?;
        }
        // The fixture's runs do not progress, so a watch replays what each recorded.
        Ok(run.status(run_id))
    }
    async fn cancel(&self, run_id: &str) -> Result<RunStatus, Refusal> {
        let mut runs = self.runs.lock().unwrap();
        let run = runs.get_mut(run_id).ok_or_else(|| unknown_run(run_id))?;
        if !run.terminal {
            (run.status, run.terminal) = ("cancelled".into(), true);
        }
        Ok(run.status(run_id))
    }
    async fn resume(&self, run_id: &str) -> Result<RunStatus, Refusal> {
        if self.status(run_id).await?.terminal {
            return Err(codes::refusal(
                battersea_runtime::error_codes::EXECUTION_TERMINAL,
                "The run has finished; its tool calls no longer dispatch.",
            ));
        }
        let journal = self.journal(run_id).await?;
        let inspected = journal
            .inspect()
            .await
            .map_err(|error| codes::refusal(codes::HOST_FAILURE, error.message.to_string()))?;
        let Some((call_id, _)) = inspected
            .calls
            .iter()
            .find(|(_, call)| matches!(call.state, StoredToolCallState::Approved))
        else {
            return Err(codes::refusal(
                ToolCode::ToolApprovalPending.as_str(),
                "The run's tool call awaits a decision.",
            ));
        };
        journal
            .resume(call_id, inspected.revision)
            .await
            .map_err(|error| {
                codes::refusal(
                    error
                        .tool_code
                        .map_or(codes::HOST_FAILURE, ToolCode::as_str),
                    error.message.to_string(),
                )
            })?;
        let mut runs = self.runs.lock().unwrap();
        let run = runs.get_mut(run_id).ok_or_else(|| unknown_run(run_id))?;
        (run.status, run.terminal) = ("succeeded".into(), true);
        Ok(run.status(run_id))
    }
}

#[async_trait]
impl ApprovalHost for Host {
    async fn approvals(&self, run_id: &str) -> Result<Arc<DurableToolExecutor>, Refusal> {
        self.journal(run_id).await
    }
    async fn agent(&self, run_id: &str) -> Result<GrantIdentity, Refusal> {
        self.status(run_id).await?;
        Ok(GrantIdentity::of(&agent()))
    }
}

#[async_trait]
impl McpHost for Host {
    async fn servers(&self) -> Result<Vec<McpServer>, Refusal> {
        Ok(self.servers.lock().unwrap().values().cloned().collect())
    }
    async fn refresh(&self, server_id: &str) -> Result<McpServer, Refusal> {
        let mut servers = self.servers.lock().unwrap();
        let server = servers.get_mut(server_id).ok_or_else(|| {
            codes::refusal(
                codes::UNKNOWN_MCP_SERVER,
                format!("No MCP connection \"{server_id}\"."),
            )
        })?;
        let revision = server
            .binding_revision
            .strip_prefix("binding-")
            .and_then(|n| n.parse::<u64>().ok())
            .unwrap_or_default();
        server.binding_revision = format!("binding-{}", revision + 1);
        Ok(server.clone())
    }
}

/// A fresh host's `battersea` contribution, with every capability.
pub fn contributions() -> Vec<Contribution> {
    let host = Host::new();
    vec![BatterseaOps::new()
        .with_flows(host.clone())
        .with_runs(host.clone())
        .with_approvals(host.clone())
        .with_mcp(host)
        .contribution()]
}

/// A fresh host's composed set, as its daemon serves it.
pub fn composed() -> Result<ComposedSet, clapham::CompositionError> {
    clapham::compose_with_options(
        contributions(),
        Bounds::default(),
        ComposeOptions {
            execution: ExecutionMode::Daemon,
            oauth: Some(OAuthResource {
                issuer: ISSUER.into(),
                audience: AUDIENCE.into(),
            }),
            ..ComposeOptions::default()
        },
    )
}

/// The input of a decision on the pending run's seeded call, at its seeded revision.
pub async fn seeded_decision(action: &str) -> Value {
    let host = Host::new();
    let journal = host.journal(PENDING_RUN).await.expect("the seeded journal");
    let inspected = journal.inspect().await.expect("a readable journal");
    json!({
        "run_id": PENDING_RUN,
        "call_id": PENDING_CALL,
        "request_revision": inspected.calls[PENDING_CALL].request_revision,
        "expected_revision": inspected.revision,
        "action": action,
    })
}
