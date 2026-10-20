//! Copyright (c) Scott A Dixon
//! Behavioural contracts across the public MCP, registry and provider interfaces.
use battersea_mcp::{McpConnection, McpHttpOptions, McpSelection};
use battersea_model::{
    EngineAdapter, EngineAdapterRequestError, EngineTextStreamEvent, EngineTextStreamRequest,
    ToolCallOutcome,
};
use battersea_tools::{
    ToolApprovalDecision, ToolApprovalPolicy, ToolApprovalRequest, ToolRegistry,
};
use futures_util::StreamExt;
use mcp_example::{
    fixture::{self, Fault, HttpServer, McpFixture, ProviderFixture, Result},
    host::{self, StdioHost},
};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[derive(Clone, Copy, Debug)]
enum Transport {
    Stdio,
    Json,
    Sse,
}
const TRANSPORTS: [Transport; 3] = [Transport::Stdio, Transport::Json, Transport::Sse];

struct HostPolicy {
    decision: ToolApprovalDecision,
    binding_revision: String,
    calls: std::sync::atomic::AtomicUsize,
}
impl ToolApprovalPolicy for HostPolicy {
    fn decide(&self, request: ToolApprovalRequest<'_>) -> ToolApprovalDecision {
        assert_eq!(request.scope.run_id, "approval-run");
        assert_eq!(request.scope.binding_revision, self.binding_revision);
        assert_eq!(request.call.id, "effect-1");
        assert_eq!(request.call.name, "remote.write");
        assert_eq!(request.call.arguments, json!({"value":1}));
        assert!(request.tool.effect().destructive);
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.decision
    }
}

#[tokio::test]
async fn remote_approval_precedes_effects_and_provider_retries_reuse_the_decision() {
    use battersea_model::ToolCode;
    use std::sync::atomic::Ordering;
    for transport in TRANSPORTS {
        for decision in [
            ToolApprovalDecision::Allow,
            ToolApprovalDecision::Deny,
            ToolApprovalDecision::Pending,
        ] {
            let (_directory, ledger) = temporary_ledger();
            let remote = Remote::start(transport, &ledger, Fault::None, Duration::from_secs(10))
                .await
                .unwrap();
            let (binding, tools) = host::bind(&remote.connection).await.unwrap();
            let policy = Arc::new(HostPolicy {
                decision,
                binding_revision: binding.revision().into(),
                calls: 0.into(),
            });
            let fixture = ProviderFixture::new(json!({"value":1}), true);
            let provider = fixture::provider_http(fixture.clone()).await.unwrap();
            let (adapter, chat) = host::provider(&provider.endpoint).unwrap();
            let request =
                host::approved_request(&binding, &tools, chat, "approval-run", policy.clone())
                    .unwrap();
            let executor = request.local_tool_executor.clone().unwrap();
            let result = collect(&adapter, request).await;
            let count = match decision {
                ToolApprovalDecision::Allow => {
                    assert!(result.unwrap().iter().any(|event| matches!(
                        event,
                        EngineTextStreamEvent::ToolCallCompleted { ok: true, .. }
                    )));
                    let requests = fixture.requests.lock().unwrap();
                    assert_eq!(requests.len(), 3);
                    assert_eq!(requests[1], requests[2]);
                    1
                }
                ToolApprovalDecision::Deny | ToolApprovalDecision::Pending => {
                    let error = result.unwrap_err();
                    assert_eq!(
                        error.tool_call.unwrap().outcome,
                        ToolCallOutcome::NotDispatched
                    );
                    assert_eq!(
                        error.tool_code,
                        Some(if decision == ToolApprovalDecision::Deny {
                            ToolCode::ToolApprovalDenied
                        } else {
                            ToolCode::ToolApprovalPending
                        })
                    );
                    assert_eq!(fixture.requests.lock().unwrap().len(), 1);
                    0
                }
            };
            let duplicate = executor
                .call(battersea_model::EngineLocalToolCall {
                    id: "effect-1".into(),
                    name: "remote_write".into(),
                    arguments: json!({"value":2}),
                })
                .await
                .unwrap_err();
            assert_eq!(duplicate.tool_code, Some(ToolCode::InvalidToolCallId));
            assert_eq!(
                duplicate.tool_call.unwrap().outcome,
                ToolCallOutcome::NotDispatched
            );
            assert_eq!(policy.calls.load(Ordering::SeqCst), 1);
            assert_eq!(fixture::effect_count(&ledger).unwrap(), count);
            remote.close().await;
        }
    }
}
struct Remote {
    connection: McpConnection,
    stdio: Option<StdioHost>,
    http: Option<HttpServer>,
}
impl Remote {
    async fn start(
        transport: Transport,
        ledger: &Path,
        fault: Fault,
        timeout: Duration,
    ) -> Result<Self> {
        let config = host::mcp_config(timeout);
        match transport {
            Transport::Stdio => {
                let stdio = StdioHost::start(
                    Path::new(env!("CARGO_BIN_EXE_mcp-example")),
                    ledger,
                    fault,
                    config,
                )
                .await?;
                Ok(Self {
                    connection: stdio.connection.clone(),
                    stdio: Some(stdio),
                    http: None,
                })
            }
            _ => {
                let http = fixture::mcp_http(
                    McpFixture::new(ledger.to_owned(), fault),
                    matches!(transport, Transport::Sse),
                )
                .await?;
                let connection = McpConnection::http(
                    McpHttpOptions::new(format!("{}/mcp", http.endpoint)),
                    config,
                )
                .await?;
                Ok(Self {
                    connection,
                    stdio: None,
                    http: Some(http),
                })
            }
        }
    }
    async fn close(self) {
        self.connection.close();
        if let Some(stdio) = self.stdio {
            stdio.close().await.unwrap();
        }
        drop(self.http);
    }
}
async fn collect(
    adapter: &Arc<dyn EngineAdapter>,
    request: EngineTextStreamRequest,
) -> std::result::Result<Vec<EngineTextStreamEvent>, EngineAdapterRequestError> {
    let mut stream = adapter.stream_text(request).await?;
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event?);
    }
    Ok(events)
}
fn temporary_ledger() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let ledger = directory.path().join("effects");
    (directory, ledger)
}

#[tokio::test]
async fn independent_binary_demonstrates_all_transports() {
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_mcp-example"))
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Three effects recorded"));
}

#[tokio::test]
async fn provider_continuation_retries_reuse_mcp_results_and_exclude_metadata() {
    for transport in TRANSPORTS {
        for fault in [Fault::None, Fault::ToolError] {
            let (_directory, ledger) = temporary_ledger();
            let remote = Remote::start(transport, &ledger, fault, Duration::from_secs(10))
                .await
                .unwrap();
            let (_binding, tools) = host::bind(&remote.connection).await.unwrap();
            assert!(tools.for_profile("forged").definitions().is_empty());
            let fixture = ProviderFixture::new(json!({"value":1}), true);
            let provider = fixture::provider_http(fixture.clone()).await.unwrap();
            let (adapter, chat) = host::provider(&provider.endpoint).unwrap();
            let events = collect(&adapter, host::request(&tools, "operator", chat))
                .await
                .unwrap();
            let completed = events
                .iter()
                .filter_map(|event| match event {
                    EngineTextStreamEvent::ToolCallCompleted {
                        ok,
                        result,
                        tool_result,
                        tool_call,
                        ..
                    } => Some((ok, result, tool_result, tool_call)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(completed.len(), 1);
            let (ok, projected, retained, evidence) = completed[0];
            assert_eq!(*ok, fault == Fault::None);
            assert_eq!(
                evidence.as_ref().unwrap().outcome,
                ToolCallOutcome::Completed
            );
            let retained = serde_json::to_value(retained).unwrap().to_string();
            assert!(retained.contains("private-receipt"));
            assert!(!projected.to_string().contains("private-receipt"));
            assert!(events.iter().any(|e| matches!(e, EngineTextStreamEvent::TextDelta { text } if text == "Remote result received")));
            let requests = fixture.requests.lock().unwrap().clone();
            assert_eq!(requests.len(), 3);
            assert_eq!(requests[1], requests[2]);
            assert_eq!(requests[0]["tools"][0]["function"]["name"], "remote_write");
            let result = requests[1]["messages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["role"] == "tool")
                .unwrap();
            let content: Value = serde_json::from_str(result["content"].as_str().unwrap()).unwrap();
            assert_eq!(content["isError"], fault == Fault::ToolError);
            assert_eq!(content["structuredContent"]["count"], 1);
            assert!(content["content"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["uri"] == "https://example.invalid/receipt"));
            assert!(!serde_json::to_string(&requests)
                .unwrap()
                .contains("private-receipt"));
            assert_eq!(fixture::effect_count(&ledger).unwrap(), 1);
            remote.close().await;
        }
    }
}

#[tokio::test]
async fn remote_failures_preserve_tool_outcomes_without_provider_replay() {
    for transport in TRANSPORTS {
        for (fault, outcome) in [
            (Fault::LostReply, ToolCallOutcome::Uncertain),
            (Fault::Disconnect, ToolCallOutcome::Uncertain),
            (Fault::Oversized, ToolCallOutcome::Uncertain),
            (Fault::ProtocolError, ToolCallOutcome::Completed),
            (Fault::InvalidOutput, ToolCallOutcome::Completed),
        ] {
            let (_directory, ledger) = temporary_ledger();
            let remote = Remote::start(transport, &ledger, fault, Duration::from_secs(10))
                .await
                .unwrap();
            let (_binding, tools) = host::bind(&remote.connection).await.unwrap();
            let fixture = ProviderFixture::new(json!({"value":1}), true);
            let provider = fixture::provider_http(fixture.clone()).await.unwrap();
            let (adapter, chat) = host::provider(&provider.endpoint).unwrap();
            let error = collect(&adapter, host::request(&tools, "operator", chat))
                .await
                .unwrap_err();
            let evidence = error.tool_call.unwrap();
            assert_eq!(evidence.outcome, outcome, "{transport:?} {fault:?}");
            assert_eq!(evidence.call_id, "effect-1");
            assert_eq!(fixture.requests.lock().unwrap().len(), 1);
            assert_eq!(fixture::effect_count(&ledger).unwrap(), 1);
            remote.close().await;
            if outcome == ToolCallOutcome::Uncertain {
                // Restart the controlled server with its retained effect ledger, then reconnect.
                let restarted =
                    Remote::start(transport, &ledger, Fault::None, Duration::from_secs(10))
                        .await
                        .unwrap();
                host::bind(&restarted.connection).await.unwrap();
                assert_eq!(fixture::effect_count(&ledger).unwrap(), 1);
                assert_eq!(evidence.outcome, ToolCallOutcome::Uncertain);
                assert_eq!(fixture.requests.lock().unwrap().len(), 1);
                restarted.close().await;
            }
        }
    }
}

#[tokio::test]
async fn cancellation_after_remote_dispatch_does_not_resume_or_repeat_the_effect() {
    for transport in TRANSPORTS {
        let (_directory, ledger) = temporary_ledger();
        let remote = Remote::start(
            transport,
            &ledger,
            Fault::LostReply,
            Duration::from_secs(10),
        )
        .await
        .unwrap();
        let (_binding, tools) = host::bind(&remote.connection).await.unwrap();
        let fixture = ProviderFixture::new(json!({"value":1}), true);
        let provider = fixture::provider_http(fixture.clone()).await.unwrap();
        let (adapter, chat) = host::provider(&provider.endpoint).unwrap();
        let mut stream = adapter
            .stream_text(host::request(&tools, "operator", chat))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                tokio::select! {
                    event = stream.next() => {
                        let event = event.unwrap().unwrap();
                        assert!(!matches!(event, EngineTextStreamEvent::ToolCallCompleted { .. }));
                    }
                    _ = tokio::time::sleep(Duration::from_millis(5)) => {
                        if fixture::effect_count(&ledger).ok() == Some(1) { break; }
                    }
                }
            }
        })
        .await
        .unwrap();
        drop(stream);
        remote.close().await;
        let restarted = Remote::start(transport, &ledger, Fault::None, Duration::from_secs(10))
            .await
            .unwrap();
        host::bind(&restarted.connection).await.unwrap();
        assert_eq!(fixture::effect_count(&ledger).unwrap(), 1);
        assert_eq!(fixture.requests.lock().unwrap().len(), 1);
        restarted.close().await;
    }
}

#[tokio::test]
async fn schema_and_host_profile_refusals_precede_remote_dispatch() {
    for transport in TRANSPORTS {
        for (arguments, profile) in [
            (json!({"value":"wrong type"}), "operator"),
            (json!({"value":1}), "forged"),
        ] {
            let (_directory, ledger) = temporary_ledger();
            let remote = Remote::start(transport, &ledger, Fault::None, Duration::from_secs(10))
                .await
                .unwrap();
            let (_binding, tools) = host::bind(&remote.connection).await.unwrap();
            let fixture = ProviderFixture::new(arguments, false);
            let provider = fixture::provider_http(fixture.clone()).await.unwrap();
            let (adapter, chat) = host::provider(&provider.endpoint).unwrap();
            let error = collect(&adapter, host::request(&tools, profile, chat))
                .await
                .unwrap_err();
            assert_eq!(
                error.tool_call.unwrap().outcome,
                ToolCallOutcome::NotDispatched
            );
            assert_eq!(fixture::effect_count(&ledger).unwrap(), 0);
            assert_eq!(fixture.requests.lock().unwrap().len(), 1);
            remote.close().await;
        }
    }
}

#[tokio::test]
async fn changed_definitions_stop_a_pinned_provider_conversation() {
    for sse in [false, true] {
        let (_directory, ledger) = temporary_ledger();
        let server = McpFixture::new(ledger.clone(), Fault::None);
        let remote = fixture::mcp_http(server.clone(), sse).await.unwrap();
        let connection = McpConnection::http(
            McpHttpOptions::new(format!("{}/mcp", remote.endpoint)),
            host::mcp_config(Duration::from_secs(10)),
        )
        .await
        .unwrap();
        let (_binding, tools) = host::bind(&connection).await.unwrap();
        server.tools.lock().unwrap()[0]["description"] = json!("Changed definition");
        connection.discover().await.unwrap();
        let fixture = ProviderFixture::new(json!({"value":1}), false);
        let provider = fixture::provider_http(fixture).await.unwrap();
        let (adapter, chat) = host::provider(&provider.endpoint).unwrap();
        let error = collect(&adapter, host::request(&tools, "operator", chat))
            .await
            .unwrap_err();
        assert_eq!(
            error.tool_call.unwrap().outcome,
            ToolCallOutcome::NotDispatched
        );
        assert_eq!(fixture::effect_count(&ledger).unwrap(), 0);
        connection.close();
    }
}

#[tokio::test]
async fn catalogue_admission_and_alias_collisions_use_shared_registry_rules() {
    let (_directory, ledger) = temporary_ledger();
    let server = McpFixture::new(ledger, Fault::None);
    let mut tools = server.tools.lock().unwrap().clone();
    let mut invalid = tools[0].clone();
    invalid["name"] = json!("invalid");
    invalid["inputSchema"] =
        json!({"type":"object","properties":{"x":{"$ref":"https://example.invalid/schema"}}});
    for name in ["a.b", "a_b"] {
        let mut tool = tools[0].clone();
        tool["name"] = json!(name);
        tools.push(tool);
    }
    tools.push(invalid);
    *server.tools.lock().unwrap() = tools;
    let remote = fixture::mcp_http(server, false).await.unwrap();
    let connection = McpConnection::http(
        McpHttpOptions::new(format!("{}/mcp", remote.endpoint)),
        host::mcp_config(Duration::from_secs(10)),
    )
    .await
    .unwrap();
    let catalogue = connection.discover().await.unwrap();
    assert_eq!(catalogue.diagnostics().len(), 1);
    assert_eq!(catalogue.diagnostics()[0].name.as_deref(), Some("invalid"));
    assert!(catalogue
        .bind(
            vec![McpSelection::new("invalid", vec!["operator".into()])],
            &ToolRegistry::default()
        )
        .is_err());
    assert!(catalogue
        .bind(
            vec![
                McpSelection::new("a.b", vec![]),
                McpSelection::new("a_b", vec![])
            ],
            &ToolRegistry::default()
        )
        .is_err());
    connection.close();
}

#[tokio::test]
async fn durable_pending_calls_resume_from_disk_once_without_regenerating() {
    use battersea_tools::{
        DurableToolApprovalScope, DurableToolExecutor, ToolApprovalAction,
        ToolApprovalJournalLimits,
    };
    use mcp_example::journal::FileApprovalStore;
    use std::time::{SystemTime, UNIX_EPOCH};
    for transport in TRANSPORTS {
        let (directory, ledger) = temporary_ledger();
        let remote = Remote::start(transport, &ledger, Fault::None, Duration::from_secs(10))
            .await
            .unwrap();
        let (binding, tools) = host::bind(&remote.connection).await.unwrap();
        let scope = DurableToolApprovalScope {
            run_id: "durable-run".into(),
            binding_revision: binding.revision().into(),
            expires_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
                + 60000,
        };
        let store = || Arc::new(FileApprovalStore::new(directory.path().into()));
        let executor = DurableToolExecutor::create(
            store(),
            tools.for_profile("operator"),
            scope.clone(),
            ToolApprovalJournalLimits {
                max_calls: 8,
                max_bytes: 65536,
            },
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        let fixture = ProviderFixture::new(json!({"value":1}), false);
        let provider = fixture::provider_http(fixture.clone()).await.unwrap();
        let (adapter, chat) = host::provider(&provider.endpoint).unwrap();
        let mut request = host::request(&tools, "operator", chat);
        request.local_tools = executor.definitions();
        request.local_tool_executor = Some(Arc::new(executor));
        let error = collect(&adapter, request).await.unwrap_err();
        assert_eq!(
            error.tool_code,
            Some(battersea_model::ToolCode::ToolApprovalPending)
        );
        assert_eq!(fixture::effect_count(&ledger).unwrap(), 0);
        let executor = DurableToolExecutor::open(
            store(),
            tools.for_profile("operator"),
            scope.clone(),
            65536,
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        let pending = executor.inspect().await.unwrap();
        let approved = executor
            .decide(
                "effect-1",
                &pending.calls["effect-1"].request_revision,
                pending.revision,
                ToolApprovalAction::Allow,
                "operator",
            )
            .await
            .unwrap();
        drop(executor);
        let executor = DurableToolExecutor::open(
            store(),
            tools.for_profile("operator"),
            scope.clone(),
            65536,
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        let result = executor
            .resume("effect-1", approved.revision)
            .await
            .unwrap();
        assert!(serde_json::to_string(&result)
            .unwrap()
            .contains("private-receipt"));
        assert!(!format!("{:?}", executor.inspect().await.unwrap()).contains("private-receipt"));
        drop(executor);
        let executor = DurableToolExecutor::open(
            store(),
            tools.for_profile("operator"),
            scope,
            65536,
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        let completed = executor.inspect().await.unwrap();
        assert_eq!(
            executor
                .resume("effect-1", completed.revision)
                .await
                .unwrap(),
            result
        );
        assert_eq!(fixture::effect_count(&ledger).unwrap(), 1);
        assert_eq!(fixture.requests.lock().unwrap().len(), 1);
        remote.close().await;
        drop(provider);
    }
}

#[tokio::test]
async fn file_approval_store_fences_competing_handles_and_bounds_read_only_loads() {
    use battersea_tools::{
        DurableToolApprovalScope, DurableToolExecutor, ToolApprovalJournalLimits,
        ToolApprovalStore, ToolApprovalStoreError,
    };
    use mcp_example::journal::FileApprovalStore;
    use std::time::{SystemTime, UNIX_EPOCH};
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(FileApprovalStore::new(directory.path().into()));
    let scope = DurableToolApprovalScope {
        run_id: "file-race".into(),
        binding_revision: "binding-1".into(),
        expires_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 60000,
    };
    let executor = DurableToolExecutor::create(
        store.clone(),
        ToolRegistry::default(),
        scope,
        ToolApprovalJournalLimits {
            max_calls: 1,
            max_bytes: 4096,
        },
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    let mut next = executor.inspect().await.unwrap();
    next.revision += 1;
    let second = FileApprovalStore::new(directory.path().into());
    let (a, b) = tokio::join!(
        store.compare_exchange("file-race", Some(1), &next),
        second.compare_exchange("file-race", Some(1), &next)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(
        store
            .load("file-race", 4096)
            .await
            .unwrap()
            .unwrap()
            .revision,
        2
    );
    let before = std::fs::read(directory.path().join("approval.json")).unwrap();
    assert_eq!(
        store.load("file-race", 1).await.unwrap_err(),
        ToolApprovalStoreError::Unavailable
    );
    assert_eq!(
        store.load("another-run", 4096).await.unwrap_err(),
        ToolApprovalStoreError::Conflict
    );
    assert_eq!(
        std::fs::read(directory.path().join("approval.json")).unwrap(),
        before
    );
    std::fs::write(
        directory.path().join("approval.lock"),
        b"host must inspect crash-left lock",
    )
    .unwrap();
    next.revision += 1;
    assert_eq!(
        store
            .compare_exchange("file-race", Some(2), &next)
            .await
            .unwrap_err(),
        ToolApprovalStoreError::Conflict
    );
    assert_eq!(
        store
            .load("file-race", 4096)
            .await
            .unwrap()
            .unwrap()
            .revision,
        2
    );
}
