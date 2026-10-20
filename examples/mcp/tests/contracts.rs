//! Copyright (c) Scott A Dixon
//! Behavioural contracts across the public MCP, registry and provider interfaces.
use battersea_mcp::{McpConnection, McpHttpOptions, McpSelection};
use battersea_model::{
    EngineAdapter, EngineAdapterRequestError, EngineTextStreamEvent, EngineTextStreamRequest,
    ToolCallOutcome,
};
use battersea_tools::ToolRegistry;
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
            let remote = Remote::start(transport, &ledger, fault, Duration::from_secs(2))
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
            let remote = Remote::start(transport, &ledger, fault, Duration::from_secs(2))
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
                    Remote::start(transport, &ledger, Fault::None, Duration::from_secs(2))
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
        let restarted = Remote::start(transport, &ledger, Fault::None, Duration::from_secs(2))
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
            let remote = Remote::start(transport, &ledger, Fault::None, Duration::from_secs(2))
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
            host::mcp_config(Duration::from_secs(2)),
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
        host::mcp_config(Duration::from_secs(2)),
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
