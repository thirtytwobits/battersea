#![cfg(feature = "stdio")]
use battersea_mcp::*;
use battersea_model::{EngineLocalToolCall, EngineLocalToolExecutor, ToolCallOutcome};
use battersea_tools::{ToolRegistry, PROFILES_META_KEY};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

struct Server {
    tools: Mutex<Vec<Value>>,
    result: Mutex<Value>,
    calls: AtomicUsize,
    lists: AtomicUsize,
    cancelled: AtomicUsize,
    mode: AtomicUsize,
    replies: Mutex<Vec<Value>>,
}
fn tool(name: &str) -> Value {
    json!({"name":name,"description":"A controlled tool","inputSchema":{"type":"object","properties":{"value":{"type":"integer"}},"required":["value"]}})
}
fn server() -> Arc<Server> {
    Arc::new(Server {
        tools: Mutex::new(vec![tool("write")]),
        result: Mutex::new(
            json!({"content":[{"type":"text","text":"done"}],"structuredContent":{"value":1},"_meta":{"private":"secret"}}),
        ),
        calls: AtomicUsize::new(0),
        lists: AtomicUsize::new(0),
        cancelled: AtomicUsize::new(0),
        mode: AtomicUsize::new(0),
        replies: Mutex::new(Vec::new()),
    })
}
fn config() -> McpConfig {
    let mut config = McpConfig::new("server", "credential-scope-1");
    config.operation_timeout = Duration::from_millis(500);
    config
}
fn call() -> EngineLocalToolCall {
    EngineLocalToolCall {
        id: "call-1".into(),
        name: "server_write".into(),
        arguments: json!({"value":1}),
    }
}
fn discover_result() -> Value {
    json!({"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{}},"ttlMs":0,"cacheScope":"private"})
}
async fn connect(
    server: Arc<Server>,
    config: McpConfig,
) -> (
    McpConnection,
    tokio::sync::mpsc::Sender<Value>,
    CancellationToken,
) {
    let (client, remote) = tokio::io::duplex(8192);
    let (read, write) = tokio::io::split(client);
    let (reader, mut writer) = tokio::io::split(remote);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Value>(4);
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        loop {
            let outgoing = tokio::select! {
                value = rx.recv() => match value { Some(value) => value, None => break },
                line = lines.next_line() => {
                    let Ok(Some(line)) = line else { break; };
                    let request: Value = serde_json::from_str(&line).unwrap();
                    let Some(method) = request["method"].as_str() else {
                        server.replies.lock().unwrap().push(request);
                        continue;
                    };
                    let result = match method {
                        "server/discover" => {
                            assert!(request["params"]["_meta"].is_object(), "{request}");
                            discover_result()
                        },
                        "tools/list" => {
                            server.lists.fetch_add(1, Ordering::SeqCst);
                            if server.mode.load(Ordering::SeqCst) == 6 { continue; }
                            let mut result = json!({"tools":server.tools.lock().unwrap().clone()});
                            if server.mode.load(Ordering::SeqCst) == 5 { result["nextCursor"] = json!("repeated"); }
                            result
                        },
                        "tools/call" => {
                            assert_eq!(request["params"]["name"], "write");
                            server.calls.fetch_add(1, Ordering::SeqCst);
                            match server.mode.load(Ordering::SeqCst) {
                                1 => continue, // Effect applied; reply lost.
                                2 => { let _ = writer.write_all(&vec![b'x'; 2 * 1024 * 1024]).await; continue; },
                                3 => break,
                                4 => {
                                    let error = json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32603,"message":"secret remote diagnostic","data":{"secret":true}}});
                                    if writer.write_all(format!("{error}\n").as_bytes()).await.is_err() { break; }
                                    continue;
                                }
                                _ => server.result.lock().unwrap().clone(),
                            }
                        },
                        "notifications/cancelled" => { server.cancelled.fetch_add(1, Ordering::SeqCst); continue; },
                        _ => continue,
                    };
                    json!({"jsonrpc":"2.0","id":request["id"],"result":result})
                }
            };
            if writer
                .write_all(format!("{outgoing}\n").as_bytes())
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let shutdown = CancellationToken::new();
    (
        McpConnection::stdio(read, write, shutdown.clone(), config)
            .await
            .unwrap(),
        tx,
        shutdown,
    )
}
async fn binding(connection: &McpConnection) -> McpToolBinding {
    connection
        .discover()
        .await
        .unwrap()
        .bind(
            vec![McpSelection::new("write", vec!["host".into()])],
            &ToolRegistry::default(),
        )
        .unwrap()
}
async fn wait_count(count: &AtomicUsize, expected: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while count.load(Ordering::SeqCst) < expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn profiles_provenance_and_results_follow_host_contracts() {
    let server = server();
    server.tools.lock().unwrap()[0]["_meta"] = json!({PROFILES_META_KEY:["forged"]});
    let (connection, _tx, shutdown) = connect(server.clone(), config()).await;
    let catalogue = connection.discover().await.unwrap();
    assert_eq!(
        catalogue.provenance("write").unwrap()["_meta"][PROFILES_META_KEY],
        json!(["forged"])
    );
    let binding = catalogue
        .bind(
            vec![McpSelection::new("write", vec!["host".into()])],
            &ToolRegistry::default(),
        )
        .unwrap();
    let mut registry = ToolRegistry::default();
    binding.register_into(&mut registry).unwrap();
    assert!(registry.for_profile("forged").definitions().is_empty());
    assert_eq!(registry.for_profile("host").definitions().len(), 1);
    let result = binding.executor().call(call()).await.unwrap();
    assert!(!result
        .provider_content()
        .unwrap()
        .to_string()
        .contains("secret"));
    assert!(!format!("{result:?}").contains("secret"));
    let mut invalid = call();
    invalid.arguments = json!({"value":"bad"});
    assert_eq!(
        binding
            .executor()
            .call(invalid)
            .await
            .unwrap_err()
            .tool_call
            .unwrap()
            .outcome,
        ToolCallOutcome::NotDispatched
    );
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    connection.close();
    tokio::time::timeout(Duration::from_secs(1), shutdown.cancelled())
        .await
        .unwrap();
}
#[tokio::test]
async fn invalid_tools_and_normalised_alias_collisions_are_refused() {
    let server = server();
    let mut invalid = tool("invalid");
    invalid["inputSchema"] =
        json!({"type":"object","properties":{"x":{"$ref":"https://invalid.example/schema"}}});
    *server.tools.lock().unwrap() = vec![
        tool("a.b"),
        tool("a_b"),
        invalid,
        tool("duplicate"),
        tool("duplicate"),
    ];
    let (connection, _tx, _) = connect(server, config()).await;
    let catalogue = connection.discover().await.unwrap();
    assert_eq!(catalogue.diagnostics().len(), 2);
    for name in ["invalid", "duplicate"] {
        assert!(catalogue
            .bind(
                vec![McpSelection::new(name, vec![])],
                &ToolRegistry::default()
            )
            .is_err());
    }
    assert!(catalogue
        .bind(
            vec![
                McpSelection::new("a.b", vec![]),
                McpSelection::new("a_b", vec![])
            ],
            &ToolRegistry::default()
        )
        .is_err());
    let first = catalogue
        .bind(
            vec![McpSelection::new("a.b", vec![])],
            &ToolRegistry::default(),
        )
        .unwrap();
    let mut existing = ToolRegistry::default();
    first.register_into(&mut existing).unwrap();
    assert!(catalogue
        .bind(vec![McpSelection::new("a_b", vec![])], &existing)
        .is_err());
    assert!(first.register_into(&mut existing).is_err());
    assert_eq!(existing.definitions().len(), 1);
}
#[tokio::test]
async fn notifications_and_refresh_guard_pinned_definitions() {
    let server = server();
    let (connection, tx, _) = connect(server.clone(), config()).await;
    let binding = binding(&connection).await;
    tx.send(json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let error = binding.executor().call(call()).await.unwrap_err();
    assert_eq!(
        error.tool_call.unwrap().outcome,
        ToolCallOutcome::NotDispatched
    );
    connection.discover().await.unwrap();
    binding.executor().call(call()).await.unwrap();
    server.tools.lock().unwrap()[0]["description"] = json!("Changed");
    connection.discover().await.unwrap();
    assert!(binding.executor().call(call()).await.is_err());
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn applied_effect_with_lost_reply_is_uncertain_and_never_replayed() {
    let server = server();
    server.mode.store(1, Ordering::SeqCst);
    let (connection, _tx, _) = connect(server.clone(), config()).await;
    let binding = binding(&connection).await;
    let error = binding.executor().call(call()).await.unwrap_err();
    assert_eq!(error.tool_call.unwrap().outcome, ToolCallOutcome::Uncertain);
    wait_count(&server.cancelled, 1).await;
    connection.close();
    let (reconnected, _tx2, _) = connect(server.clone(), config()).await;
    reconnected.discover().await.unwrap();
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn dropping_call_requests_cancellation_without_replay() {
    let server = server();
    server.mode.store(1, Ordering::SeqCst);
    let (connection, _tx, _) = connect(server.clone(), config()).await;
    let executor = binding(&connection).await.executor();
    let task = tokio::spawn(async move { executor.call(call()).await });
    wait_count(&server.calls, 1).await;
    task.abort();
    let _ = task.await;
    wait_count(&server.cancelled, 1).await;
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn saturated_queue_refuses_without_dispatch() {
    let server = server();
    server.mode.store(1, Ordering::SeqCst);
    let mut config = config();
    config.limits.in_flight = 1;
    config.limits.waiting = 0;
    let (connection, _tx, _) = connect(server.clone(), config).await;
    let executor = binding(&connection).await.executor();
    let first = {
        let executor = executor.clone();
        tokio::spawn(async move { executor.call(call()).await })
    };
    wait_count(&server.calls, 1).await;
    let error = executor.call(call()).await.unwrap_err();
    assert_eq!(
        error.tool_call.unwrap().outcome,
        ToolCallOutcome::NotDispatched
    );
    first.abort();
    let _ = first.await;
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn oversized_frames_and_disconnect_preserve_uncertainty() {
    for mode in [2, 3] {
        let server = server();
        server.mode.store(mode, Ordering::SeqCst);
        let (connection, _tx, _) = connect(server.clone(), config()).await;
        let error = binding(&connection)
            .await
            .executor()
            .call(call())
            .await
            .unwrap_err();
        assert_eq!(error.tool_call.unwrap().outcome, ToolCallOutcome::Uncertain);
        if mode == 2 {
            assert!(error.message.contains("limit"));
        }
        assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn tool_errors_continue_and_protocol_errors_have_bounded_diagnostics() {
    let server = server();
    server.tools.lock().unwrap()[0]["outputSchema"] =
        json!({"type":"object","required":["answer"]});
    *server.result.lock().unwrap() =
        json!({"content":[{"type":"text","text":"Tool failed"}],"isError":true});
    let (connection, _tx, _) = connect(server.clone(), config()).await;
    let executor = binding(&connection).await.executor();
    assert!(executor.call(call()).await.unwrap().is_error());
    server.mode.store(4, Ordering::SeqCst);
    let error = executor.call(call()).await.unwrap_err();
    assert_eq!(
        error.tool_call.as_ref().unwrap().outcome,
        ToolCallOutcome::Completed
    );
    assert!(!format!("{error:?}").contains("secret"));
    server.mode.store(0, Ordering::SeqCst);
    *server.result.lock().unwrap() = json!({"content":[],"structuredContent":{"wrong":1}});
    assert_eq!(
        executor
            .call(call())
            .await
            .unwrap_err()
            .tool_call
            .unwrap()
            .outcome,
        ToolCallOutcome::Completed
    );
}

#[tokio::test]
async fn discovery_bounds_and_failed_refresh_invalidate_existing_bindings() {
    let server = server();
    let (connection, _tx, _) = connect(server.clone(), config()).await;
    let binding = binding(&connection).await;
    server.mode.store(5, Ordering::SeqCst);
    assert!(matches!(connection.discover().await, Err(McpError::Limit)));
    assert_eq!(
        binding
            .executor()
            .call(call())
            .await
            .unwrap_err()
            .tool_call
            .unwrap()
            .outcome,
        ToolCallOutcome::NotDispatched
    );
    server.mode.store(0, Ordering::SeqCst);
    let mut config = config();
    config.limits.tools = 1;
    server.tools.lock().unwrap().push(tool("second"));
    let (connection, _tx, _) = connect(server, config).await;
    assert!(matches!(connection.discover().await, Err(McpError::Limit)));
}
#[tokio::test]
async fn binding_revision_pins_host_profiles_and_aliases() {
    let (connection, _tx, _) = connect(server(), config()).await;
    let catalogue = connection.discover().await.unwrap();
    let bind = |selection| {
        catalogue
            .bind(vec![selection], &ToolRegistry::default())
            .unwrap()
    };
    let first = bind(McpSelection::new("write", vec!["a".into()]));
    let second = bind(McpSelection::new("write", vec!["b".into()]));
    assert_ne!(first.revision(), second.revision());
    let mut selection = McpSelection::new("write", vec!["a".into()]);
    selection.alias = Some("alternate".into());
    assert_ne!(first.revision(), bind(selection).revision());
}
#[tokio::test]
async fn unsupported_interactions_are_rejected_without_follow_up_calls() {
    let server = server();
    let (connection, tx, _) = connect(server.clone(), config()).await;
    let executor = binding(&connection).await.executor();
    for (id, method, params) in [
        (1, "roots/list", json!({})),
        (
            2,
            "elicitation/create",
            json!({"message":"Input","requestedSchema":{"type":"object","properties":{}}}),
        ),
        (
            3,
            "sampling/createMessage",
            json!({"messages":[],"maxTokens":1}),
        ),
        (4, "tasks/get", json!({"taskId":"task"})),
    ] {
        tx.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if server.replies.lock().unwrap().len() == 4 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    {
        let replies = server.replies.lock().unwrap();
        assert!(
            replies
                .iter()
                .all(|r| r["error"]["code"].as_i64().is_some_and(|code| code < 0)
                    && r.get("result").is_none()),
            "{replies:?}"
        );
    }
    *server.result.lock().unwrap() = json!({"resultType":"input_required","inputRequests":{"roots":{"method":"roots/list"}},"requestState":"opaque"});
    let error = executor.call(call()).await.unwrap_err();
    assert_eq!(error.tool_call.unwrap().outcome, ToolCallOutcome::Uncertain);
    assert!(error.message.contains("unsupported"), "{}", error.message);
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn unsupported_revision_is_refused_and_host_shutdown_signalled() {
    let (client, remote) = tokio::io::duplex(4096);
    let (read, write) = tokio::io::split(client);
    tokio::spawn(async {
        let (read, mut write) = tokio::io::split(remote);
        let request: Value = serde_json::from_str(
            &BufReader::new(read)
                .lines()
                .next_line()
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        let mut result = discover_result();
        result["supportedVersions"] = json!(["2025-03-26"]);
        write
            .write_all(
                format!(
                    "{}\n",
                    json!({"jsonrpc":"2.0","id":request["id"],"result":result})
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    });
    let shutdown = CancellationToken::new();
    assert!(matches!(
        McpConnection::stdio(read, write, shutdown.clone(), config()).await,
        Err(McpError::Protocol)
    ));
    tokio::time::timeout(Duration::from_secs(1), shutdown.cancelled())
        .await
        .unwrap();
}

#[tokio::test]
async fn aggregate_queue_capacity_and_notification_limits_are_enforced() {
    let (client, _) = tokio::io::duplex(64);
    let (read, write) = tokio::io::split(client);
    let mut invalid = config();
    invalid.limits.message_bytes = 8 * 1024 * 1024;
    assert!(matches!(
        McpConnection::stdio(read, write, CancellationToken::new(), invalid).await,
        Err(McpError::Configuration)
    ));
    let mut config = config();
    config.limits.notifications = 1;
    let (connection, tx, shutdown) = connect(server(), config).await;
    for _ in 0..2 {
        tx.send(json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}))
            .await
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(1), shutdown.cancelled())
        .await
        .unwrap();
    assert!(matches!(connection.discover().await, Err(McpError::Limit)));
}

#[tokio::test]
async fn abandoned_discovery_closes_its_pending_request_state() {
    let server = server();
    server.mode.store(6, Ordering::SeqCst);
    let (connection, _tx, shutdown) = connect(server.clone(), config()).await;
    let copy = connection.clone();
    let task = tokio::spawn(async move { copy.discover().await });
    wait_count(&server.lists, 1).await;
    task.abort();
    let _ = task.await;
    tokio::time::timeout(Duration::from_secs(1), shutdown.cancelled())
        .await
        .unwrap();
    assert!(connection.discover().await.is_err());
}

#[tokio::test]
async fn duplicate_names_in_a_refresh_invalidate_existing_bindings() {
    let server = server();
    let original = tool("write");
    let mut changed = original.clone();
    changed["description"] = json!("Another definition");
    let (connection, _tx, _) = connect(server.clone(), config()).await;
    let executor = binding(&connection).await.executor();
    for records in [
        vec![original.clone(), original.clone()],
        vec![changed, original.clone()],
    ] {
        *server.tools.lock().unwrap() = records;
        let catalogue = connection.discover().await.unwrap();
        assert!(!catalogue.diagnostics().is_empty());
        let error = executor.call(call()).await.unwrap_err();
        assert_eq!(
            error.tool_call.unwrap().outcome,
            ToolCallOutcome::NotDispatched
        );
        assert_eq!(server.calls.load(Ordering::SeqCst), 0);
    }
    *server.tools.lock().unwrap() = vec![original];
    connection.discover().await.unwrap();
    executor.call(call()).await.unwrap();
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
}
