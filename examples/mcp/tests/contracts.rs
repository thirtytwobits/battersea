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

#[tokio::test]
async fn durable_provider_continuation_reopens_both_journals_and_retries_the_saved_transcript() {
    use battersea_model::{
        ConversationPhase, ConversationRequest, ConversationScope, ConversationToolResult,
        DurableConversation,
    };
    use battersea_model::{EngineAdapterLogger, EngineAdapterRequestLog, EngineAdapterResponseLog};
    use battersea_tools::{
        DurableToolApprovalScope, DurableToolExecutor, ToolApprovalAction,
        ToolApprovalJournalLimits,
    };
    use mcp_example::{conversation_journal::FileConversationStore, journal::FileApprovalStore};
    use std::{
        sync::Mutex,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[derive(Default)]
    struct Logs {
        requests: Mutex<Vec<EngineAdapterRequestLog>>,
        responses: Mutex<Vec<EngineAdapterResponseLog>>,
    }
    #[async_trait::async_trait]
    impl EngineAdapterLogger for Logs {
        async fn on_request(&self, entry: EngineAdapterRequestLog) {
            self.requests.lock().unwrap().push(entry);
        }
        async fn on_response(&self, entry: EngineAdapterResponseLog) {
            self.responses.lock().unwrap().push(entry);
        }
    }
    for (transport, streaming, responses, background) in TRANSPORTS.into_iter().flat_map(|t| {
        [
            (false, false, false),
            (true, false, false),
            (true, true, false),
            (true, true, true),
        ]
        .map(|(s, r, b)| (t, s, r, b))
    }) {
        let (directory, ledger) = temporary_ledger();
        let remote = Remote::start(transport, &ledger, Fault::None, Duration::from_secs(10))
            .await
            .unwrap();
        let (binding, tools) = host::bind(&remote.connection).await.unwrap();
        let mut fixture = ProviderFixture::new(json!({"value":1}), true);
        if responses {
            Arc::get_mut(&mut fixture).unwrap().final_text = "  Résultat reçu ✓  ".into();
        }
        let jobs = mcp_example::background_fixture::BackgroundFixture::new(fixture.clone());
        let provider = if background {
            mcp_example::background_fixture::serve(jobs.clone()).await
        } else if responses {
            fixture::responses_http(fixture.clone()).await
        } else {
            fixture::provider_http(fixture.clone()).await
        }
        .unwrap();
        let logs = Arc::new(Logs::default());
        let connect = || {
            if background {
                host::background_responses_provider(&provider.endpoint, Some(logs.clone()))
            } else if responses {
                host::responses_provider_with_logger(&provider.endpoint, logs.clone())
            } else {
                host::provider(&provider.endpoint)
            }
        };
        let (adapter, chat) = connect().unwrap();
        let mut original = host::request(&tools, "operator", chat);
        original.shared.chat.stream = streaming;
        let request = ConversationRequest {
            shared: original.shared,
            tools: original.local_tools,
            max_turns: 2,
        };
        let scope = ConversationScope {
            run_id: "durable-provider".into(),
            tool_binding_revision: binding.revision().into(),
            expires_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
                + 4000,
        };
        let approval_scope = DurableToolApprovalScope {
            run_id: scope.run_id.clone(),
            binding_revision: scope.tool_binding_revision.clone(),
            expires_at_unix_ms: scope.expires_at_unix_ms,
        };
        let store = || Arc::new(FileConversationStore::new(directory.path().into()));
        let approvals = || Arc::new(FileApprovalStore::new(directory.path().into()));
        let conversation = DurableConversation::create(
            store(),
            adapter,
            scope.clone(),
            request.clone(),
            65536,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        let mut pending = conversation.resume(1).await.unwrap();
        if background {
            assert_eq!(pending.phase, ConversationPhase::ProviderPending);
            drop(conversation);
            let (adapter, _) = connect().unwrap();
            let reopened = DurableConversation::open(
                store(),
                adapter,
                scope.clone(),
                request.clone(),
                65536,
                Duration::from_secs(1),
            )
            .await
            .unwrap();
            pending = reopened.resume(pending.revision).await.unwrap();
        } else {
            drop(conversation);
        }
        assert_eq!(pending.phase, ConversationPhase::AwaitingTools);
        let gate = DurableToolExecutor::create(
            approvals(),
            tools.for_profile("operator"),
            approval_scope.clone(),
            ToolApprovalJournalLimits {
                max_calls: 8,
                max_bytes: 65536,
            },
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        gate.submit(pending.turns[0].response.calls[0].clone(), 1)
            .await
            .unwrap();
        assert_eq!(fixture.requests.lock().unwrap().len(), 1);
        assert_eq!(fixture::effect_count(&ledger).unwrap(), 0);
        drop(gate);

        let (adapter, _) = connect().unwrap();
        let conversation = DurableConversation::open(
            store(),
            adapter,
            scope.clone(),
            request.clone(),
            65536,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        let gate = DurableToolExecutor::open(
            approvals(),
            tools.for_profile("operator"),
            approval_scope.clone(),
            65536,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        let approval = gate.inspect().await.unwrap();
        gate.decide(
            "effect-1",
            &approval.calls["effect-1"].request_revision,
            approval.revision,
            ToolApprovalAction::Allow,
            "operator",
        )
        .await
        .unwrap();
        let result = gate.resume("effect-1", 3).await.unwrap();
        let call = approval.calls["effect-1"].call.clone();
        let retained = ConversationToolResult { call, result };
        conversation
            .record_tool_result(pending.revision, retained.clone())
            .await
            .unwrap();
        assert_eq!(fixture::effect_count(&ledger).unwrap(), 1);
        drop(gate);
        drop(conversation);

        let (adapter, _) = connect().unwrap();
        let conversation = DurableConversation::open(
            store(),
            adapter,
            scope.clone(),
            request.clone(),
            65536,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(
            conversation
                .record_tool_result(pending.revision + 1, retained)
                .await
                .unwrap()
                .revision,
            pending.revision + 1
        );
        let mut completed = conversation.resume(pending.revision + 1).await.unwrap();
        if background {
            assert_eq!(completed.phase, ConversationPhase::ProviderPending);
            let (adapter, _) = connect().unwrap();
            let reopened = DurableConversation::open(
                store(),
                adapter,
                scope.clone(),
                request.clone(),
                65536,
                Duration::from_secs(1),
            )
            .await
            .unwrap();
            completed = reopened.resume(completed.revision).await.unwrap();
            assert_eq!(
                *jobs.operations.lock().unwrap(),
                ["create", "poll:job-1", "create", "create", "poll:job-3"]
            );
            assert_eq!(completed.attempts, 2);
        }
        assert_eq!(completed.phase, ConversationPhase::Completed);
        assert!(completed.turns[1].response.events.iter().any(
            |e| matches!(e,EngineTextStreamEvent::TextDelta{text} if text==&fixture.final_text)
        ));
        assert!(serde_json::to_string(&completed)
            .unwrap()
            .contains("private-receipt"));
        assert!(!format!("{completed:?}").contains("private-receipt"));
        {
            let requests = fixture.requests.lock().unwrap();
            assert_eq!(requests.len(), 3);
            assert_eq!(requests[1], requests[2]);
            assert!(!serde_json::to_string(&*requests)
                .unwrap()
                .contains("private-receipt"));
            if responses {
                assert_eq!(
                    requests[1]["input"][1]["encrypted_content"],
                    "opaque-reasoning"
                );
                assert_eq!(
                    requests[1]["input"][2]["vendor"]["signature"],
                    "retained-call"
                );
                assert_eq!(requests[1]["input"][3]["type"], "function_call_output");
                assert_eq!(requests[1]["input"][3]["call_id"], "effect-1");
                assert!(requests[1]["include"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("reasoning.encrypted_content")));
                assert!(requests[1].get("previous_response_id").is_none());
            } else {
                assert_eq!(
                    requests[1]["messages"][1]["tool_calls"][0]["id"],
                    "effect-1"
                );
                assert_eq!(requests[1]["messages"][2]["tool_call_id"], "effect-1");
                if !streaming {
                    assert_eq!(
                        requests[1]["messages"][1]["fixture_state"]["token"],
                        "retained-provider-token"
                    );
                }
            }
        }
        drop(conversation);
        let (adapter, _) = connect().unwrap();
        let conversation = DurableConversation::open(
            store(),
            adapter,
            scope,
            request,
            65536,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(
            conversation.resume(completed.revision).await.unwrap(),
            completed
        );
        let gate = DurableToolExecutor::open(
            approvals(),
            tools.for_profile("operator"),
            approval_scope,
            65536,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(gate.inspect().await.unwrap().revision, 5);
        assert_eq!(fixture.requests.lock().unwrap().len(), 3);
        assert_eq!(fixture::effect_count(&ledger).unwrap(), 1);
        if responses {
            // Reopening and retrying a rejected HTTP request do not duplicate completed logs.
            let requests = logs.requests.lock().unwrap();
            let responses = logs.responses.lock().unwrap();
            assert_eq!(requests.len(), if background { 4 } else { 2 });
            assert_eq!(responses.len(), if background { 4 } else { 2 });
            for (index, response) in responses
                .iter()
                .filter(|entry| entry.detail.output_chars.is_some())
                .enumerate()
            {
                let turn = &completed.turns[index].response;
                let native = &turn.state["response"];
                assert_eq!(response.context, requests[index].context);
                assert_eq!(response.operation, requests[index].operation);
                assert_eq!(response.status, 200);
                assert!(response.detail.ok);
                assert_eq!(response.detail.streaming, Some(!background));
                assert_eq!(
                    response.detail.response_id.as_deref(),
                    native["id"].as_str()
                );
                assert_eq!(response.detail.usage.as_ref(), native.get("usage"));
                assert_eq!(
                    response.detail.output_chars,
                    Some(if index == 0 {
                        0
                    } else {
                        fixture.final_text.chars().count()
                    })
                );
            }
            assert!(!serde_json::to_string(&*requests)
                .unwrap()
                .contains("private-receipt"));
            assert!(!serde_json::to_string(&*responses)
                .unwrap()
                .contains("private-receipt"));
        }
        remote.close().await;
    }
}

#[tokio::test]
async fn interrupted_or_invalid_provider_continuations_keep_the_tool_result_and_block_replay() {
    use battersea_model::{
        ConversationPhase, ConversationRequest, ConversationResolution, ConversationScope,
        ConversationToolResult, DurableConversation, EngineLocalToolExecutor, ToolCode,
    };
    use mcp_example::conversation_journal::FileConversationStore;
    use std::{
        sync::atomic::Ordering,
        time::{SystemTime, UNIX_EPOCH},
    };
    for (responses, failure) in [false, true]
        .into_iter()
        .flat_map(|r| (0..3).map(move |f| (r, f)))
    {
        let (directory, ledger) = temporary_ledger();
        let remote = Remote::start(
            Transport::Json,
            &ledger,
            Fault::None,
            Duration::from_secs(10),
        )
        .await
        .unwrap();
        let (binding, tools) = host::bind(&remote.connection).await.unwrap();
        let fixture = ProviderFixture::new(json!({"value":1}), false);
        fixture
            .fail_continuation
            .store(failure == 0, Ordering::SeqCst);
        fixture
            .hang_continuation
            .store(failure == 1, Ordering::SeqCst);
        let provider = if responses {
            fixture::responses_http(fixture.clone()).await
        } else {
            fixture::provider_http(fixture.clone()).await
        }
        .unwrap();
        let connect = |endpoint: &str| {
            if responses {
                host::responses_provider(endpoint)
            } else {
                host::provider(endpoint)
            }
        };
        let (adapter, chat) = connect(&provider.endpoint).unwrap();
        let original = host::request(&tools, "operator", chat);
        let mut request = ConversationRequest {
            shared: original.shared,
            tools: original.local_tools,
            max_turns: 2,
        };
        if failure == 2 {
            request.shared.chat.response_format = Some(battersea_model::ResponseFormat::JsonObject);
        }
        let scope = ConversationScope {
            run_id: "failed-continuation".into(),
            tool_binding_revision: binding.revision().into(),
            expires_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
                + 4000,
        };
        let store = || Arc::new(FileConversationStore::new(directory.path().into()));
        let conversation = DurableConversation::create(
            store(),
            adapter,
            scope.clone(),
            request.clone(),
            65536,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        let pending = conversation.resume(1).await.unwrap();
        let call = pending.turns[0].response.calls[0].clone();
        let result = tools
            .for_profile("operator")
            .call(call.clone())
            .await
            .unwrap();
        conversation
            .record_tool_result(3, ConversationToolResult { call, result })
            .await
            .unwrap();
        if failure == 1 {
            let task = tokio::spawn(async move { conversation.resume(4).await });
            tokio::time::timeout(Duration::from_secs(2), async {
                while fixture.requests.lock().unwrap().len() < 2 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            let error = conversation.resume(4).await.unwrap_err();
            assert_eq!(error.tool_call.unwrap().outcome, ToolCallOutcome::Completed);
            drop(conversation);
        }
        let (adapter, _) = connect(&provider.endpoint).unwrap();
        let conversation = DurableConversation::open(
            store(),
            adapter,
            scope.clone(),
            request.clone(),
            65536,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        let intent = conversation.inspect().await.unwrap();
        assert_eq!(intent.phase, ConversationPhase::ProviderIntent);
        assert_eq!(intent.turns[0].results.len(), 1);
        assert_eq!(
            conversation
                .resume(intent.revision)
                .await
                .unwrap_err()
                .tool_code,
            Some(ToolCode::ToolContinuationUnresolved)
        );
        let alternate = fixture::provider_http(ProviderFixture::new(json!({"value":1}), false))
            .await
            .unwrap();
        let (changed, _) = connect(&alternate.endpoint).unwrap();
        assert!(DurableConversation::open(
            store(),
            changed,
            scope,
            request,
            65536,
            Duration::from_secs(1)
        )
        .await
        .is_err());
        conversation
            .reconcile(
                intent.revision,
                ConversationResolution::Abandon,
                "operator",
                "Local task fenced; provider completion unavailable",
            )
            .await
            .unwrap();
        assert_eq!(fixture.requests.lock().unwrap().len(), 2);
        assert_eq!(fixture::effect_count(&ledger).unwrap(), 1);
        remote.close().await;
    }
}

#[tokio::test]
async fn reconciled_and_reopened_calls_must_match_the_provider_transcript() {
    use battersea_model::{
        configuration_digest, ContentBlock, ConversationCheckpoint, ConversationPhase,
        ConversationRequest, ConversationResolution, ConversationScope, ConversationStore,
        ConversationStoreError, ConversationToolResult, DurableConversation,
        EngineLocalToolExecutor, ResponseFormat,
    };
    use mcp_example::conversation_journal::FileConversationStore;
    use std::{
        sync::Mutex,
        time::{SystemTime, UNIX_EPOCH},
    };

    // Retain the provider reply while simulating failure before turn publication.
    struct InterruptedPublication {
        inner: FileConversationStore,
        retained: Mutex<Option<ConversationCheckpoint>>,
    }
    #[async_trait::async_trait]
    impl ConversationStore for InterruptedPublication {
        async fn load(
            &self,
            run: &str,
            max: usize,
        ) -> std::result::Result<Option<ConversationCheckpoint>, ConversationStoreError> {
            self.inner.load(run, max).await
        }
        async fn compare_exchange(
            &self,
            run: &str,
            expected: Option<u64>,
            next: &ConversationCheckpoint,
        ) -> std::result::Result<(), ConversationStoreError> {
            if (next.revision == 3 && next.resolutions.is_empty())
                || (next.revision == 6 && next.resolutions.len() == 1)
            {
                *self.retained.lock().unwrap() = Some(next.clone());
                return Err(ConversationStoreError::Unavailable);
            }
            self.inner.compare_exchange(run, expected, next).await
        }
    }
    for (responses, streaming, schema) in [(false, false), (false, true), (true, true)]
        .into_iter()
        .flat_map(|(r, s)| [false, true].map(move |f| (r, s, f)))
    {
        let (directory, ledger) = temporary_ledger();
        let remote = Remote::start(
            Transport::Json,
            &ledger,
            Fault::None,
            Duration::from_secs(10),
        )
        .await
        .unwrap();
        let (binding, tools) = host::bind(&remote.connection).await.unwrap();
        let mut fixture = ProviderFixture::new(json!({"value":1}), false);
        Arc::get_mut(&mut fixture).unwrap().final_text = r#"{"received":true}"#.into();
        let provider = if responses {
            fixture::responses_http(fixture.clone()).await
        } else {
            fixture::provider_http(fixture.clone()).await
        }
        .unwrap();
        let (adapter, chat) = if responses {
            host::responses_provider(&provider.endpoint)
        } else {
            host::provider(&provider.endpoint)
        }
        .unwrap();
        let mut original = host::request(&tools, "operator", chat);
        original.shared.chat.stream = streaming;
        original.shared.chat.response_format = Some(if schema {
            ResponseFormat::JsonSchema {
                name: "receipt".into(),
                schema: json!({"type":"object","required":["received"],"properties":{"received":{"type":"boolean"}},"additionalProperties":false}),
            }
        } else {
            ResponseFormat::JsonObject
        });
        let mut request = ConversationRequest {
            shared: original.shared,
            tools: original.local_tools,
            max_turns: 2,
        };
        let mut alternative = request.tools[0].clone();
        alternative.name = "alternative".into();
        request.tools.push(alternative);
        let scope = ConversationScope {
            run_id: "recovered-provider".into(),
            tool_binding_revision: binding.revision().into(),
            expires_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
                + 4000,
        };
        let store = Arc::new(InterruptedPublication {
            inner: FileConversationStore::new(directory.path().into()),
            retained: Mutex::new(None),
        });
        let conversation = DurableConversation::create(
            store.clone(),
            adapter.clone(),
            scope.clone(),
            request.clone(),
            65536,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert!(conversation.resume(1).await.is_err());
        let turn = store.retained.lock().unwrap().as_ref().unwrap().turns[0]
            .response
            .clone();
        for mutation in 0..9 {
            let mut changed = turn.clone();
            match mutation {
                0 => changed.calls[0].id = "different-id".into(),
                1 => changed.calls[0].name = "alternative".into(),
                2 => changed.calls[0].arguments = json!({"value":2}),
                3 => changed.calls.clear(),
                4 => changed.calls.push(changed.calls[0].clone()),
                5 => changed.events.insert(
                    1,
                    EngineTextStreamEvent::TextDelta {
                        text: "invented".into(),
                    },
                ),
                6 => changed.events.insert(
                    1,
                    EngineTextStreamEvent::StructuredOutput {
                        value: json!({"invented":true}),
                    },
                ),
                7 => {
                    for event in &mut changed.events {
                        if let EngineTextStreamEvent::ContentBlock {
                            block: ContentBlock::NativeV2 { payload, .. },
                        } = event
                        {
                            payload["content"] = json!("invented");
                        }
                    }
                }
                _ if responses => {
                    changed.state["response"]["output"][0]["status"] = json!("incomplete");
                }
                _ => {
                    let messages = changed.state["messages"].as_array_mut().unwrap();
                    messages.last_mut().unwrap()["refusal"] = json!("refused");
                    for event in &mut changed.events {
                        if let EngineTextStreamEvent::ContentBlock {
                            block: ContentBlock::NativeV2 { payload, .. },
                        } = event
                        {
                            payload["refusal"] = json!("refused");
                        }
                    }
                }
            }
            assert!(conversation
                .reconcile(
                    2,
                    ConversationResolution::Completed { turn: changed },
                    "operator",
                    "Retained response from fenced task"
                )
                .await
                .is_err());
            let unchanged = conversation.inspect().await.unwrap();
            assert_eq!(unchanged.revision, 2);
            assert_eq!(unchanged.phase, ConversationPhase::ProviderIntent);
            assert!(unchanged.turns.is_empty());
        }
        let accepted = conversation
            .reconcile(
                2,
                ConversationResolution::Completed { turn },
                "operator",
                "Retained response from fenced task",
            )
            .await
            .unwrap();
        assert_eq!(accepted.phase, ConversationPhase::AwaitingTools);
        assert_eq!(fixture.requests.lock().unwrap().len(), 1);
        assert_eq!(fixture::effect_count(&ledger).unwrap(), 0);

        // A valid shared checksum cannot substitute for adapter transcript validation on reopen.
        let mut changed = serde_json::to_value(&accepted).unwrap();
        changed["turns"][0]["response"]["calls"][0]["arguments"] = json!({"value":2});
        changed.as_object_mut().unwrap().remove("checksum");
        changed["checksum"] = json!(configuration_digest(&changed));
        let changed: ConversationCheckpoint = serde_json::from_value(changed).unwrap();
        changed.validate().unwrap();
        std::fs::write(
            directory.path().join("conversation.json"),
            serde_json::to_vec(&changed).unwrap(),
        )
        .unwrap();
        assert!(DurableConversation::open(
            store.clone(),
            adapter.clone(),
            scope.clone(),
            request.clone(),
            65536,
            Duration::from_secs(1)
        )
        .await
        .is_err());
        assert_eq!(fixture.requests.lock().unwrap().len(), 1);
        assert_eq!(fixture::effect_count(&ledger).unwrap(), 0);

        std::fs::write(
            directory.path().join("conversation.json"),
            serde_json::to_vec(&accepted).unwrap(),
        )
        .unwrap();
        let call = accepted.turns[0].response.calls[0].clone();
        let result = tools
            .for_profile("operator")
            .call(call.clone())
            .await
            .unwrap();
        conversation
            .record_tool_result(3, ConversationToolResult { call, result })
            .await
            .unwrap();
        assert!(conversation.resume(4).await.is_err());
        let retained = store.retained.lock().unwrap().clone().unwrap();
        let final_turn = retained.turns[1].response.clone();
        for mutation in 0..if responses { 24 } else { 13 } {
            let mut changed = final_turn.clone();
            match mutation {
                0 => {
                    // Both display projections agree with each other but differ from native evidence.
                    for event in &mut changed.events {
                        match event {
                            EngineTextStreamEvent::TextDelta { text } => {
                                *text = r#"{"received":false}"#.into()
                            }
                            EngineTextStreamEvent::StructuredOutput { value } => {
                                *value = json!({"received":false})
                            }
                            _ => (),
                        }
                    }
                }
                1 => {
                    for event in &mut changed.events {
                        if let EngineTextStreamEvent::ContentBlock {
                            block: ContentBlock::NativeV2 { payload, .. },
                        } = event
                        {
                            payload["content"] = json!("invented");
                        }
                    }
                }
                2 => {
                    for event in &mut changed.events {
                        if let EngineTextStreamEvent::TokenUsage { usage } = event {
                            usage.total_tokens = Some(1000);
                        }
                    }
                }
                3 => changed
                    .events
                    .retain(|e| !matches!(e, EngineTextStreamEvent::ContentBlock { .. })),
                4 if responses => {
                    changed.state["input"][3]["output"] = json!("different result");
                }
                4 => {
                    let messages = changed.state["messages"].as_array_mut().unwrap();
                    let tool = messages.iter_mut().find(|m| m["role"] == "tool").unwrap();
                    tool["content"] = json!("{\"different_result\":true}");
                }
                5 if responses => {
                    changed.state["input"][1]["encrypted_content"] = json!("different reasoning");
                }
                5 => {
                    let messages = changed.state["messages"].as_array_mut().unwrap();
                    let assistant = messages
                        .iter_mut()
                        .find(|m| m["role"] == "assistant")
                        .unwrap();
                    assistant["content"] = json!("different preceding response");
                }
                6 if responses => {
                    changed.state["input"].as_array_mut().unwrap().remove(2);
                }
                6 => {
                    let messages = changed.state["messages"].as_array_mut().unwrap();
                    let last = messages.last().unwrap().clone();
                    messages.truncate(1);
                    messages.push(last);
                }
                7 => {
                    let event = changed
                        .events
                        .iter()
                        .find(|e| matches!(e, EngineTextStreamEvent::ContentBlock { .. }))
                        .unwrap()
                        .clone();
                    changed.events.insert(1, event);
                }
                8 => changed.state["version"] = json!(0),
                9 => changed
                    .events
                    .retain(|e| !matches!(e, EngineTextStreamEvent::StructuredOutput { .. })),
                10 => {
                    let event = changed
                        .events
                        .iter()
                        .find(|e| matches!(e, EngineTextStreamEvent::StructuredOutput { .. }))
                        .unwrap()
                        .clone();
                    changed.events.insert(1, event);
                }
                11 if responses => {
                    changed.state["response"]["output"][0]["content"] =
                        json!([{"type":"refusal","refusal":"refused"}]);
                }
                12 if responses => changed.state["response"]["status"] = json!("incomplete"),
                13 => changed.state["response"]["error"] = json!({"message":"failed"}),
                14 => {
                    changed.state["response"]["incomplete_details"] =
                        json!({"reason":"max_output_tokens"})
                }
                15 => changed.state["response"]["output"][0]["type"] = json!("unknown"),
                16 => changed.state["response"]["output_text"] = json!("different text"),
                17 => changed.state["input"][2]["vendor"] = json!({"signature":"different"}),
                18 => changed.state["request_revision"] = json!("different"),
                19 => changed.state["binding"]["configuration_revision"] = json!("different"),
                20 => changed.events.insert(
                    1,
                    EngineTextStreamEvent::ReasoningDelta {
                        text: "invented".into(),
                    },
                ),
                21 => {
                    changed.state["input"][0] = json!({"role":"user","content":"different request"})
                }
                22 => {
                    changed.state["response"]
                        .as_object_mut()
                        .unwrap()
                        .remove("id");
                }
                23 => changed.state["response"]["usage"]["input_tokens"] = json!(-1),
                _ => {
                    let messages = changed.state["messages"].as_array_mut().unwrap();
                    let index = if mutation == 11 {
                        messages.len() - 1
                    } else {
                        1
                    };
                    messages[index]["refusal"] = json!("refused");
                    if mutation == 11 {
                        for event in &mut changed.events {
                            if let EngineTextStreamEvent::ContentBlock {
                                block: ContentBlock::NativeV2 { payload, .. },
                            } = event
                            {
                                payload["refusal"] = json!("refused");
                            }
                        }
                    }
                    assert!(
                        adapter
                            .validate_conversation(&request, Some(&changed.state))
                            .is_err(),
                        "refusal in assistant message {index}"
                    );
                }
            }
            assert!(
                conversation
                    .reconcile(
                        5,
                        ConversationResolution::Completed {
                            turn: changed.clone()
                        },
                        "operator",
                        "Retained response from fenced task"
                    )
                    .await
                    .is_err(),
                "mutation {mutation}"
            );
            let unchanged = conversation.inspect().await.unwrap();
            assert_eq!(unchanged.revision, 5);
            assert_eq!(unchanged.phase, ConversationPhase::ProviderIntent);
            assert_eq!(unchanged.turns.len(), 1);
            assert_eq!(unchanged.turns[0].results, retained.turns[0].results);

            // Reopening validates the same evidence with a recomputed checksum.
            let mut stored = retained.clone();
            stored.turns[1].response = changed;
            let mut stored = serde_json::to_value(stored).unwrap();
            stored.as_object_mut().unwrap().remove("checksum");
            stored["checksum"] = json!(configuration_digest(&stored));
            let stored: ConversationCheckpoint = serde_json::from_value(stored).unwrap();
            if (9..=10).contains(&mutation) {
                assert!(stored.validate().is_err());
            } else {
                stored.validate().unwrap();
            }
            std::fs::write(
                directory.path().join("conversation.json"),
                serde_json::to_vec(&stored).unwrap(),
            )
            .unwrap();
            assert!(
                DurableConversation::open(
                    store.clone(),
                    adapter.clone(),
                    scope.clone(),
                    request.clone(),
                    65536,
                    Duration::from_secs(1)
                )
                .await
                .is_err(),
                "reopen mutation {mutation}"
            );
            std::fs::write(
                directory.path().join("conversation.json"),
                serde_json::to_vec(&unchanged).unwrap(),
            )
            .unwrap();
        }
        let completed = conversation
            .reconcile(
                5,
                ConversationResolution::Completed { turn: final_turn },
                "operator",
                "Retained response from fenced task",
            )
            .await
            .unwrap();
        assert_eq!(completed.phase, ConversationPhase::Completed);
        let reopened = DurableConversation::open(
            store,
            adapter,
            scope,
            request,
            65536,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(
            reopened.resume(completed.revision).await.unwrap(),
            completed
        );
        assert_eq!(fixture.requests.lock().unwrap().len(), 2);
        assert_eq!(fixture::effect_count(&ledger).unwrap(), 1);
        remote.close().await;
    }
}

#[tokio::test]
async fn background_retrieval_failures_preserve_identity_and_cancellation_is_explicit() {
    use battersea_model::{
        ConversationPhase, ConversationRequest, ConversationScope, DurableConversation,
    };
    use mcp_example::{
        background_fixture::{self, BackgroundFixture},
        conversation_journal::FileConversationStore,
    };
    use std::{
        sync::atomic::Ordering,
        time::{SystemTime, UNIX_EPOCH},
    };
    let (directory, _) = temporary_ledger();
    let fixture = BackgroundFixture::new(ProviderFixture::new(json!({"value":1}), false));
    let server = background_fixture::serve(fixture.clone()).await.unwrap();
    let (adapter, chat) = host::background_responses_provider(&server.endpoint, None).unwrap();
    let original = host::request(&ToolRegistry::default(), "operator", chat);
    let request = ConversationRequest {
        shared: original.shared,
        tools: vec![],
        max_turns: 1,
    };
    let scope = ConversationScope {
        run_id: "background-failure".into(),
        tool_binding_revision: "empty".into(),
        expires_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 4000,
    };
    let store = || Arc::new(FileConversationStore::new(directory.path().into()));
    let conversation = DurableConversation::create(
        store(),
        adapter,
        scope.clone(),
        request.clone(),
        65536,
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    let pending = conversation.resume(1).await.unwrap();
    assert_eq!(pending.phase, ConversationPhase::ProviderPending);
    drop(conversation);
    let path = directory.path().join("conversation.json");
    let pending_bytes = std::fs::read(&path).unwrap();
    assert_eq!(*fixture.operations.lock().unwrap(), ["create"]);
    let (adapter, _) = host::background_responses_provider(&server.endpoint, None).unwrap();
    let conversation = DurableConversation::open(
        store(),
        adapter,
        scope.clone(),
        request.clone(),
        65536,
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    fixture.pending.store(true, Ordering::SeqCst);
    let waiting = conversation.resume(3).await.unwrap();
    assert_eq!(waiting.pending, pending.pending);
    assert_eq!(waiting.attempts, 1);
    fixture.fail_poll.store(true, Ordering::SeqCst);
    assert!(conversation.resume(5).await.is_err());
    fixture.fail_poll.store(false, Ordering::SeqCst);
    fixture.wrong_identity.store(true, Ordering::SeqCst);
    assert!(conversation.resume(6).await.is_err());
    fixture.wrong_identity.store(false, Ordering::SeqCst);
    let retained = conversation.inspect().await.unwrap();
    assert_eq!(retained.pending, pending.pending);
    assert_eq!(retained.phase, ConversationPhase::ProviderPending);
    fixture.fail_cancel.store(true, Ordering::SeqCst);
    assert!(conversation.cancel(7).await.is_err());
    assert!(conversation.resume(8).await.is_err());
    drop(conversation);
    fixture.fail_cancel.store(false, Ordering::SeqCst);
    let (adapter, _) = host::background_responses_provider(&server.endpoint, None).unwrap();
    let conversation = DurableConversation::open(
        store(),
        adapter,
        scope.clone(),
        request.clone(),
        65536,
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    let cancelled = conversation.cancel(8).await.unwrap();
    assert_eq!(cancelled.phase, ConversationPhase::Abandoned);
    assert_eq!(
        conversation.cancel(cancelled.revision).await.unwrap(),
        cancelled
    );
    assert_eq!(
        *fixture.operations.lock().unwrap(),
        [
            "create",
            "poll:job-1",
            "poll:job-1",
            "poll:job-1",
            "cancel:job-1",
            "cancel:job-1"
        ]
    );
    assert_eq!(fixture.provider.requests.lock().unwrap().len(), 1);
    let bytes = std::fs::read(&path).unwrap();
    for mutation in 0..3 {
        let mut value: Value = serde_json::from_slice(&bytes).unwrap();
        match mutation {
            0 => value["cancellation"]["id"] = json!("another-job"),
            1 => value["cancellation"]["status"] = json!("in_progress"),
            _ => value["cancellation"]["background"] = json!(false),
        }
        value.as_object_mut().unwrap().remove("checksum");
        value["checksum"] = json!(battersea_model::configuration_digest(&value));
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let (adapter, _) = host::background_responses_provider(&server.endpoint, None).unwrap();
        assert!(DurableConversation::open(
            store(),
            adapter,
            scope.clone(),
            request.clone(),
            65536,
            Duration::from_secs(1)
        )
        .await
        .is_err());
    }
    for mutation in 0..10 {
        let mut value: Value = serde_json::from_slice(&pending_bytes).unwrap();
        match mutation {
            0 => value["pending"]["version"] = json!(999),
            1 => value["pending"]["binding"]["configuration_revision"] = json!("changed"),
            2 => value["pending"]["request_revision"] = json!("changed"),
            3 => value["pending"]["input"] = json!([]),
            4 => value["pending"]["response"]["id"] = json!(""),
            5 => value["pending"]["response"]["status"] = json!("completed"),
            6 => value["pending"]["response"]["background"] = json!(false),
            7 => value["version"] = json!(1),
            8 => value["pending"]["response"]["output"] = json!("malformed"),
            _ => value["pending"]["response"]["extra"] = json!("x".repeat(65536)),
        }
        value.as_object_mut().unwrap().remove("checksum");
        value["checksum"] = json!(battersea_model::configuration_digest(&value));
        let changed = serde_json::to_vec(&value).unwrap();
        std::fs::write(&path, &changed).unwrap();
        let (adapter, _) = host::background_responses_provider(&server.endpoint, None).unwrap();
        assert!(
            DurableConversation::open(
                store(),
                adapter,
                scope.clone(),
                request.clone(),
                65536,
                Duration::from_secs(1)
            )
            .await
            .is_err(),
            "mutation {mutation}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), changed);
    }
    std::fs::write(&path, bytes).unwrap();
    assert_eq!(fixture.operations.lock().unwrap().len(), 6);
}
