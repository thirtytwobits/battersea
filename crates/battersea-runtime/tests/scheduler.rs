//! Distinguishing cases from contracts/execution.md. All documents are isolated fixtures.
use async_trait::async_trait;
use battersea_flow::{execution::validate_execution_contract, *};
use battersea_runtime::*;
use futures_util::{stream, StreamExt};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    ops::{Deref, DerefMut},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
struct Error {
    class: &'static str,
    reason: Option<&'static str>,
    message: String,
}
impl Error {
    fn new(class: &'static str, reason: Option<&'static str>, message: impl Into<String>) -> Self {
        Self {
            class,
            reason,
            message: message.into(),
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.class, self.message)
    }
}
impl std::error::Error for Error {}
impl ExecutionError for Error {
    fn recoverable_node_failure(&self) -> Option<RecoverableNodeFailure> {
        (self.class == "invalid_request" && self.reason == Some("fixture_recoverable")).then(|| {
            RecoverableNodeFailure {
                code: "fixture_recoverable".into(),
                message: self.message.clone(),
            }
        })
    }

    fn invalid_request(reason: &'static str, s: impl Into<String>) -> Self {
        Self::new("invalid_request", Some(reason), s)
    }
    fn internal(s: impl Into<String>) -> Self {
        Self::new("internal", None, s)
    }
    fn cancelled(reason: &'static str, s: impl Into<String>) -> Self {
        Self::new("cancelled", Some(reason), s)
    }
    fn interrupted(reason: &'static str, s: impl Into<String>) -> Self {
        Self::new("interrupted", Some(reason), s)
    }
    fn code(&self) -> &str {
        self.class
    }
    fn reason_code(&self) -> Option<&str> {
        self.reason
    }
    fn message(&self) -> &str {
        &self.message
    }
}
/// The fixture host's own refusal codes.
const FIXTURE_BORROWED_RESULTS: &str = "fixture_borrowed_results";
const FIXTURE_STATE: &str = "fixture_state";
struct Run {
    scheduler: SchedulerState,
    seen: Vec<(String, Value)>,
    calls: Vec<String>,
    held: Vec<Retained<Token>>,
    max_usage: retention::Usage,
}
impl Deref for Run {
    type Target = SchedulerState;
    fn deref(&self) -> &SchedulerState {
        &self.scheduler
    }
}
impl DerefMut for Run {
    fn deref_mut(&mut self) -> &mut SchedulerState {
        &mut self.scheduler
    }
}
struct Host {
    handlers: HandlerRegistry<Self>,
    events: Mutex<Vec<ExecutionEvent>>,
    phases: Mutex<Vec<RunPhase>>,
    polled: Arc<AtomicUsize>,
    entered: Arc<tokio::sync::Notify>,
    recovery_store: Option<Arc<RecoveryStore>>,
}

#[derive(Default)]
struct RecoveryStore {
    record: Mutex<Option<battersea_runtime::recovery::JournalRecord>>,
    fault: AtomicUsize,
    writes: AtomicUsize,
    fail_write: AtomicUsize,
    fail_after: std::sync::atomic::AtomicBool,
}
impl battersea_runtime::recovery::JournalStore for RecoveryStore {
    fn load(
        &self,
        _: &str,
    ) -> Result<
        Option<battersea_runtime::recovery::JournalRecord>,
        battersea_runtime::recovery::RecoveryError,
    > {
        Ok(self.record.lock().unwrap().clone())
    }
    fn compare_exchange(
        &self,
        _: &str,
        expected: Option<u64>,
        next: &battersea_runtime::recovery::JournalRecord,
    ) -> Result<(), battersea_runtime::recovery::RecoveryError> {
        use battersea_runtime::recovery::{EffectState, RecoveryError};
        let write = self.writes.fetch_add(1, Ordering::SeqCst) + 1;
        let inject = self.fail_write.load(Ordering::SeqCst) == write;
        if inject && !self.fail_after.load(Ordering::SeqCst) {
            return Err(RecoveryError::Storage("before publication".into()));
        }
        let mut record = self.record.lock().unwrap();
        if record.as_ref().map(|record| record.revision) != expected {
            return Err(RecoveryError::Conflict);
        }
        let completed_source = next.checkpoint.value["scheduler"]["executed_nodes"]
            .as_array()
            .is_some_and(|nodes| nodes.contains(&json!("source")));
        let intent = next
            .effects
            .values()
            .any(|effect| matches!(effect.state, EffectState::Intent));
        let fault = self.fault.load(Ordering::SeqCst);
        let triggered = (matches!(fault, 1 | 3) && completed_source) || (fault == 2 && intent);
        if triggered && fault == 3 {
            self.fault.store(0, Ordering::SeqCst);
            return Err(RecoveryError::Storage(
                "failure before result publication".into(),
            ));
        }
        *record = Some(next.clone());
        if inject {
            return Err(RecoveryError::Storage("after publication".into()));
        }
        if triggered {
            self.fault.store(0, Ordering::SeqCst);
            return Err(RecoveryError::Storage(
                "lost durable write acknowledgement".into(),
            ));
        }
        Ok(())
    }
}
fn injected_failure(node: &FlowNode, operation: &str) -> Result<(), Error> {
    if node
        .parameter_values
        .get("failure_at")
        .and_then(Value::as_str)
        == Some(operation)
    {
        let reason = node
            .parameter_values
            .get("failure_reason")
            .and_then(Value::as_str);
        return Err(match reason {
            Some("capacity") => Error::invalid_request(error_codes::CAPACITY_EXHAUSTED, "capacity"),
            Some("cancel") => Error::cancelled(error_codes::ACTIVATION_CANCELLED, "cancelled"),
            Some("effect") => Error::interrupted(error_codes::UNRESOLVED_EFFECTS, "uncertain"),
            Some("unclassified") => Error::internal("unclassified"),
            _ => Error::invalid_request("fixture_recoverable", "recoverable failure"),
        });
    }
    Ok(())
}
struct Handler;
#[async_trait]
impl NodeHandler<Host> for Handler {
    fn handler_id(&self) -> &'static str {
        "test.node"
    }
    async fn execute_node(
        &self,
        host: &Host,
        run: &mut Run,
        node: &FlowNode,
        def: &FlowNodeDefinition,
        _: Option<&HashMap<String, Value>>,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        run.calls.push(node.id.clone());
        injected_failure(node, "execute")?;
        if node.parameter_values.get("failure_at") == Some(&json!("controller")) {
            return dispatch::execute_registered_controller_action(host, run, node, def, "refresh")
                .await;
        }
        if def.kind == FlowNodeClass::Instrument {
            return Err(Error::invalid_request(
                "instrument_executed",
                "An instrument runs only through its action ports.",
            ));
        }
        if let Some(expected) = node.parameter_values.get("require_input") {
            let received = host.take_flow_input_token(run, &node.id, "in")?;
            assert_eq!(
                &received.value, expected,
                "snapshot dependency must arrive before materialisation"
            );
        }
        if def.kind == FlowNodeClass::Source {
            let values = node
                .parameter_values
                .get("values")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut events = values
                .into_iter()
                .map(|value| ProviderEvent::Token {
                    port: "out".into(),
                    value: Token {
                        token_type: "text".into(),
                        value,
                    },
                })
                .collect::<Vec<_>>();
            if node.parameter_values.get("failure_at") == Some(&json!("provider_event")) {
                events.push(ProviderEvent::Data { value: json!({}) });
            }
            if node.parameter_values.get("recoverable_provider") == Some(&json!(true)) {
                events.push(ProviderEvent::RecoverableFailure {
                    failure: RecoverableNodeFailure {
                        code: "fixture_recoverable".into(),
                        message: "provider failed".into(),
                    },
                });
            }
            if node.parameter_values.get("provider_fail") == Some(&json!(true)) {
                events.push(ProviderEvent::Failure {
                    message: "fixture provider failure".into(),
                });
            }
            if node.parameter_values.get("pump") == Some(&json!(true)) {
                let polls = host.polled.clone();
                let repeated = events.first().cloned();
                let source = stream::iter(events).inspect(move |_| {
                    polls.fetch_add(1, Ordering::SeqCst);
                });
                if node.parameter_values.get("repeat") == Some(&json!(true)) {
                    let polls = host.polled.clone();
                    host.start_provider(
                        run,
                        &node.id,
                        stream::repeat(repeated.unwrap()).inspect(move |_| {
                            polls.fetch_add(1, Ordering::SeqCst);
                        }),
                    )
                    .await?;
                } else if node.parameter_values.get("never_close") == Some(&json!(true)) {
                    host.start_provider(run, &node.id, source.chain(stream::pending()))
                        .await?;
                } else if node.parameter_values.get("panic") == Some(&json!(true)) {
                    host.start_provider(
                        run,
                        &node.id,
                        stream::poll_fn(|_| -> std::task::Poll<Option<ProviderEvent>> {
                            panic!("fixture provider panic")
                        }),
                    )
                    .await?;
                } else {
                    host.start_provider(run, &node.id, source).await?;
                }
                if node.parameter_values.get("restart") == Some(&json!(true)) {
                    host.start_provider(run, &node.id, stream::pending())
                        .await?;
                }
            } else {
                for event in events {
                    if let ProviderEvent::Token { port, value } = event {
                        host.emit_flow_token(run, &node.id, &port, value, cancel)
                            .await?;
                    }
                }
                if node.parameter_values.get("signal") == Some(&json!(true)) {
                    host.emit_flow_signal(run, node, "done").await?;
                }
            }
            if let Some(peer) = node
                .parameter_values
                .get("start_peer")
                .and_then(Value::as_str)
            {
                host.execute_flow_node(run, peer, None, cancel).await?;
            }
        } else {
            if def.kind == FlowNodeClass::Hybrid {
                assert!(run.materialized_sources.contains(&node.id));
                host.emit_flow_token(
                    run,
                    &node.id,
                    "out",
                    Token {
                        token_type: "text".into(),
                        value: node.parameter_values["execution"].clone(),
                    },
                    cancel,
                )
                .await?;
            }
            for input in host.connected_input_ports(run, &node.id) {
                if run
                    .input_tokens
                    .contains_key(&(node.id.clone(), input.clone()))
                {
                    let value = host.take_flow_input_token(run, &node.id, &input)?;
                    run.held.push(value);
                }
            }
        }
        Ok(())
    }
    async fn materialize_node(
        &self,
        host: &Host,
        run: &mut Run,
        node: &FlowNode,
        def: &FlowNodeDefinition,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        injected_failure(node, "materialize")?;
        if def.kind == FlowNodeClass::Hybrid {
            run.calls.push(format!("{}:snapshot", node.id));
            let value = node.parameter_values["snapshot"].clone();
            host.start_provider(
                run,
                &node.id,
                stream::once(async move {
                    tokio::task::yield_now().await;
                    ProviderEvent::Token {
                        port: "snapshot".into(),
                        value: Token {
                            token_type: "text".into(),
                            value,
                        },
                    }
                }),
            )
            .await
        } else {
            self.execute_node(host, run, node, def, None, cancel).await
        }
    }
    async fn execute_sink(
        &self,
        host: &Host,
        run: &mut Run,
        node: &FlowNode,
        _: &str,
        value: Retained<Token>,
    ) -> Result<(), Error> {
        run.calls.push(node.id.clone());
        injected_failure(node, "sink")?;
        if node.parameter_values.get("block") == Some(&json!(true)) {
            host.entered.notify_one();
            std::future::pending::<()>().await;
        }
        if node.parameter_values.get("fail") == Some(&json!(true)) {
            return Err(Error::internal("fixture destination failure"));
        }
        if node.parameter_values.get("forward") == Some(&json!(true)) {
            let cancel = run.activation_token.clone();
            host.emit_flow_token(run, &node.id, "out", (*value).clone(), &cancel)
                .await?;
        }
        run.seen.push((node.id.clone(), value.value.clone()));
        if node.parameter_values.get("cancel") == Some(&json!(true)) {
            return host.request_activation_cancel(run, "fixture cancel after delivery");
        }
        let usage = run.retention.usage();
        run.max_usage.items = run.max_usage.items.max(usage.items);
        run.max_usage.bytes = run.max_usage.bytes.max(usage.bytes);
        Ok(())
    }
    async fn receive_input_token(
        &self,
        _: &Host,
        run: &mut Run,
        node: &FlowNode,
        _: &FlowNodeDefinition,
        port: &str,
        value: &Token,
        _: &CancellationToken,
    ) -> Result<(), Error> {
        injected_failure(node, "receive")?;
        assert!(
            !run.input_tokens
                .contains_key(&(node.id.clone(), port.into())),
            "stream delta must not also be queued for final execution"
        );
        run.seen.push((node.id.clone(), value.value.clone()));
        Ok(())
    }
    async fn receive_provider_event(
        &self,
        _: &Host,
        _: &mut Run,
        node: &FlowNode,
        _: &FlowNodeDefinition,
        _: &Value,
        _: &CancellationToken,
    ) -> Result<(), Error> {
        injected_failure(node, "provider_event")?;
        Err(Error::invalid_request(
            error_codes::UNSUPPORTED_HANDLER_OPERATION,
            "provider events unsupported",
        ))
    }
    async fn provider_complete(
        &self,
        _: &Host,
        _: &mut Run,
        node: &FlowNode,
        _: &FlowNodeDefinition,
        _: &CancellationToken,
    ) -> Result<(), Error> {
        injected_failure(node, "provider_complete")
    }
    async fn execute_controller_action(
        &self,
        _: &Host,
        _: &mut Run,
        node: &FlowNode,
        _: &FlowNodeDefinition,
        _: &str,
    ) -> Result<(), Error> {
        injected_failure(node, "controller")?;
        Err(Error::invalid_request(
            error_codes::UNSUPPORTED_HANDLER_OPERATION,
            "controller unsupported",
        ))
    }
    async fn execute_action(
        &self,
        host: &Host,
        run: &mut Run,
        node: &FlowNode,
        _: &FlowNodeDefinition,
        _: &str,
    ) -> Result<(), Error> {
        run.calls.push(node.id.clone());
        injected_failure(node, "action")?;
        if node.parameter_values.get("cancel") == Some(&json!(true)) {
            return host.request_activation_cancel(run, "cancel action");
        }
        if node.parameter_values.get("forward") == Some(&json!(true)) {
            host.emit_flow_signal(run, node, "done").await?;
        }
        Ok(())
    }
}
#[async_trait]
impl ExecutionHost for Host {
    type State = Run;
    type Error = Error;
    fn checkpoint_host_state(
        &self,
        run: &Run,
    ) -> Result<battersea_runtime::recovery::VersionedState, Error> {
        if !run.held.is_empty() {
            return Err(Error::invalid_request(
                FIXTURE_BORROWED_RESULTS,
                "Fixture does not checkpoint borrowed join results.",
            ));
        }
        Ok(battersea_runtime::recovery::VersionedState {
            version: 1,
            value: json!({"seen":run.seen,"calls":run.calls}),
        })
    }
    fn controller_activation_effects(&self, _: &mut Run, _: &str, _: &HashMap<String, Value>) {}
    fn handlers(&self) -> &HandlerRegistry<Self> {
        &self.handlers
    }
    async fn execution_event(&self, _: &Run, event: ExecutionEvent) {
        self.events.lock().unwrap().push(event);
    }
    async fn resolve_parameter_write(
        &self,
        _: &Run,
        _: &FlowNode,
        _: &FlowParameterDefinition,
        value: &Value,
    ) -> Result<Value, Error> {
        Ok(value.clone())
    }
    async fn commit_parameter_write(
        &self,
        _: &mut Run,
        _: &FlowNode,
        _: &FlowParameterDefinition,
        _: &Value,
    ) -> Result<(), Error> {
        Ok(())
    }
    fn prepare_logic_outputs(
        &self,
        _: &FlowNode,
        _: &FlowNodeDefinition,
        _: &[FlowActionPortDefinition],
        _: &[FlowSignalPortDefinition],
        fired: &HashSet<String>,
        enabled: bool,
    ) -> Result<Vec<String>, Error> {
        Ok(if enabled && !fired.is_empty() {
            vec!["done".into()]
        } else {
            vec![]
        })
    }
    fn logic_gate_label(&self, _: &FlowNode, _: &FlowNodeDefinition) -> String {
        "fixture".into()
    }
}
#[async_trait]
impl ActivationHost for Host {
    fn durability(&self, _: &Run) -> Result<Option<battersea_runtime::durable::Durability>, Error> {
        Ok(self
            .recovery_store
            .as_ref()
            .map(|store| battersea_runtime::durable::Durability {
                store: store.clone(),
                configuration_revision: "fixture-configuration".into(),
            }))
    }
    fn restore_host_state(
        &self,
        scheduler: SchedulerState,
        state: battersea_runtime::recovery::VersionedState,
    ) -> Result<Run, Error> {
        if state.version != 1 {
            return Err(Error::invalid_request(
                FIXTURE_STATE,
                "Unknown fixture state version.",
            ));
        }
        Ok(Run {
            scheduler,
            seen: serde_json::from_value(state.value["seen"].clone())
                .map_err(|error| Error::invalid_request(FIXTURE_STATE, error.to_string()))?,
            calls: serde_json::from_value(state.value["calls"].clone())
                .map_err(|error| Error::invalid_request(FIXTURE_STATE, error.to_string()))?,
            held: vec![],
            max_usage: Default::default(),
        })
    }
    fn validate_activation(&self, run: &Run) -> Result<(), Error> {
        validate_execution_contract(&run.flow, &run.definitions).map_err(|issue| {
            Error::invalid_request(error_codes::INVALID_EXECUTION_CONTRACT, issue.message)
        })
    }
    async fn accept_activation(&self, _: &Run, _: &str) -> Result<(), Error> {
        self.phases.lock().unwrap().push(RunPhase::Accepted);
        Ok(())
    }
    async fn preflight_activation(&self, run: &mut Run) -> Result<(), Error> {
        if let Some(node) = run
            .flow
            .nodes
            .iter()
            .find(|node| node.parameter_values.get("preflight") == Some(&json!(true)))
            .cloned()
        {
            self.emit_flow_signal(run, &node, "done").await?;
        }
        Ok(())
    }
    async fn complete_execution(&self, _: &mut Run) -> Result<(), Error> {
        Ok(())
    }
    async fn retain_execution_phase(&self, _: &Run, phase: RunPhase) -> Result<(), Error> {
        self.phases.lock().unwrap().push(phase);
        Ok(())
    }
}
fn definitions(mode: &str) -> HashMap<String, FlowNodeDefinition> {
    [
        json!({"class_name":"Source","kind":"source","short_description":"source","long_description":"source","handler_id":"test.node","interfaces":["IFlowNodeActivate"],"output_ports":[{"name":"out","kind":"output","token_type":"text","mode":mode,"phase":"execution"}],"signal_ports":[{"name":"done"}],"action_ports":[{"name":"disable"}]}),
        json!({"class_name":"Sink","kind":"sink","short_description":"sink","long_description":"sink","handler_id":"test.node","input_ports":[{"name":"in","kind":"input","token_type":"text","mode":mode,"phase":"execution"}],"action_ports":[{"name":"go"},{"name":"back"}],"signal_ports":[{"name":"done"}]}),
        json!({"class_name":"Join","kind":"inline","short_description":"join","long_description":"join","handler_id":"test.node","output_ports":[{"name":"out","kind":"output","token_type":"text","mode":mode,"phase":"execution"}],"signal_ports":[{"name":"done"},{"name":"post_activate"}],"input_ports":[{"name":"in","kind":"input","token_type":"text","mode":mode,"phase":"execution"},{"name":"unused","kind":"input","token_type":"text","mode":"final_value","phase":"execution"}]}),
        json!({"class_name":"Action","kind":"control","short_description":"action","long_description":"action","handler_id":"test.node","action_ports":[{"name":"go"},{"name":"back"}],"signal_ports":[{"name":"done"}]}),
        json!({"class_name":"Tap","kind":"instrument","short_description":"tap","long_description":"tap","handler_id":"test.node","action_ports":[{"name":"go"}],"signal_ports":[{"name":"done"}]}),
    ].into_iter().map(|v| {let d:FlowNodeDefinition=serde_json::from_value(v).unwrap();(d.class_name.clone(),d)}).collect()
}
fn document(nodes: Vec<Value>, edges: Vec<Value>) -> FlowDocument {
    let sources = nodes
        .iter()
        .filter(|n| n["definition_name"] == "Source")
        .map(|n| n["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    serde_json::from_value(json!({"version":battersea_flow::document::FLOW_DOCUMENT_VERSION,"flow_key":"test","title":"test","execution":{"source_order":sources,"limits":FlowExecutionLimits::default()},"nodes":nodes,"edges":edges})).unwrap()
}
fn fixture(nodes: Vec<Value>, edges: Vec<Value>, mode: &str) -> (Host, Run) {
    let flow = document(nodes, edges);
    let mut defs = definitions(mode);
    let mut error_sink = defs["Sink"].clone();
    error_sink.class_name = "ErrorSink".into();
    error_sink.input_ports[0].token_type = "node_failure".into();
    error_sink.input_ports[0].mode = FlowPortMode::FinalValue;
    defs.insert("ErrorSink".into(), error_sink);
    for node in &flow.nodes {
        if node.failure_policy == FlowFailurePolicy::Emit {
            let d = defs.get_mut(&node.definition_name).unwrap();
            d.failure_ports = battersea_flow::failure::failure_phases(d)
                .into_iter()
                .map(|phase| FlowFailurePort {
                    name: if phase == FlowPortPhase::Snapshot {
                        "snapshot_error"
                    } else {
                        "error"
                    }
                    .into(),
                    phase,
                })
                .collect();
        }
    }
    let scheduler = SchedulerState::new("test-run".into(), &flow, defs).unwrap();
    let mut builder = RegistryBuilder::default();
    builder.register(Handler).unwrap();
    (
        Host {
            handlers: builder.build(),
            events: Mutex::default(),
            phases: Mutex::default(),
            polled: Arc::default(),
            entered: Arc::default(),
            recovery_store: None,
        },
        Run {
            scheduler,
            seen: vec![],
            calls: vec![],
            held: vec![],
            max_usage: retention::Usage::default(),
        },
    )
}
fn node(id: &str, kind: &str, params: Value) -> Value {
    json!({"id":id,"definition_name":kind,"instance_name":id,"parameter_values":params})
}
fn edge(id: &str, source: &str, target: &str, order: u32, mode: &str) -> Value {
    let mut edge = json!({"id":id,"source_node_id":source,"source_port":"out","target_node_id":target,"target_port":"in","order":order});
    if mode == "stream" {
        edge["queue"] =
            json!({"items":1,"bytes":1024,"max_event_bytes":512,"policy":"backpressure"});
    }
    edge
}
fn signal(id: &str, source: &str, target: &str, port: &str) -> Value {
    json!({"id":id,"kind":"signal","source_node_id":source,"source_port":"done","target_node_id":target,"target_port":port})
}
async fn run(host: &Host, run: &mut Run) -> Result<(), Error> {
    host.run_activation(run, "source", HashMap::new(), CancellationToken::new())
        .await
}

fn policy_node(id: &str, kind: &str, policy: &str, params: Value) -> Value {
    let mut node = node(id, kind, params);
    node["failure_policy"] = json!(policy);
    node
}
fn error_edge(source: &str, phase: &str) -> Value {
    json!({"id":"failure","source_node_id":source,"source_port":phase,
        "target_node_id":"errors","target_port":"in"})
}

#[tokio::test]
async fn failure_checkpoint_retains_emission_identity_and_rejects_corrupt_settlement() {
    use battersea_runtime::checkpoint::SchedulerCheckpoint;
    let (host, mut state) = fixture(
        vec![
            policy_node("source", "Source", "emit", json!({"failure_at":"execute"})),
            node("errors", "ErrorSink", json!({})),
        ],
        vec![error_edge("source", "error")],
        "final_value",
    );
    host.execute_flow_node(&mut state, "source", None, &CancellationToken::new())
        .await
        .unwrap();
    let checkpoint = SchedulerCheckpoint::capture(&state).unwrap();
    let saved = serde_json::to_value(&checkpoint).unwrap();
    let id = saved["handled_failures"]["source"]["failure"]["id"].clone();
    let restored = checkpoint.restore().unwrap();
    state.scheduler = restored;
    host.drain_flow_work(&mut state, &CancellationToken::new())
        .await
        .unwrap();
    host.drain_flow_work(&mut state, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(state.seen.len(), 1);
    assert_eq!(state.seen[0].1["id"], id);
    for field in ["id", "node_id", "code", "operation"] {
        let mut corrupt = saved.clone();
        corrupt["handled_failures"]["source"]["failure"][field] = json!("");
        let checkpoint: SchedulerCheckpoint = serde_json::from_value(corrupt).unwrap();
        assert!(checkpoint.restore().is_err(), "{field}");
    }
    let mut corrupt = saved.clone();
    corrupt["stopped_nodes"] = json!([]);
    assert!(serde_json::from_value::<SchedulerCheckpoint>(corrupt)
        .unwrap()
        .restore()
        .is_err());
    let mut corrupt = saved.clone();
    corrupt["failed_outputs"] = json!([["source", "error"]]);
    assert!(serde_json::from_value::<SchedulerCheckpoint>(corrupt)
        .unwrap()
        .restore()
        .is_err());
    let mut corrupt = saved;
    corrupt["version"] = json!(1);
    assert!(matches!(
        serde_json::from_value::<SchedulerCheckpoint>(corrupt)
            .unwrap()
            .restore(),
        Err(recovery::RecoveryError::Incompatible(_))
    ));
}

#[tokio::test]
async fn stopped_nodes_require_their_failure_or_dependency_record() {
    use battersea_runtime::checkpoint::SchedulerCheckpoint;
    for policy in ["emit", "skip"] {
        let (host, mut state) = fixture(
            vec![policy_node(
                "source",
                "Source",
                policy,
                json!({"failure_at":"execute"}),
            )],
            vec![],
            "final_value",
        );
        host.execute_flow_node(&mut state, "source", None, &CancellationToken::new())
            .await
            .unwrap();
        let saved = serde_json::to_value(SchedulerCheckpoint::capture(&state).unwrap()).unwrap();
        let mut missing = saved.clone();
        missing["handled_failures"] = json!({});
        assert!(
            serde_json::from_value::<SchedulerCheckpoint>(missing.clone())
                .unwrap()
                .restore()
                .is_err()
        );
        missing["dependency_skips"] = json!({"source":"failed_input"});
        assert!(serde_json::from_value::<SchedulerCheckpoint>(missing)
            .unwrap()
            .restore()
            .is_err());
    }
}

#[tokio::test]
async fn failure_retention_exhaustion_remains_a_control_outcome() {
    for policy in ["emit", "skip"] {
        let (host, mut state) = fixture(
            vec![
                policy_node("source", "Source", policy, json!({"failure_at":"execute"})),
                node("errors", "ErrorSink", json!({})),
            ],
            if policy == "emit" {
                vec![error_edge("source", "error")]
            } else {
                vec![]
            },
            "final_value",
        );
        let mut flow = state.flow.clone();
        flow.execution.limits.node_retained_bytes = 64;
        state.scheduler =
            SchedulerState::new("capacity".into(), &flow, state.definitions.clone()).unwrap();
        let error = run(&host, &mut state).await.unwrap_err();
        assert_eq!(error.reason_code(), Some(error_codes::CAPACITY_EXHAUSTED));
        assert!(state.seen.is_empty());
        assert!(host
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.kind == EventKind::NodeError
                && e.detail.as_ref().unwrap()["policy"] == policy));
    }
}

#[tokio::test]
async fn failure_outputs_are_reserved_for_the_driver() {
    let (host, mut state) = fixture(
        vec![policy_node("source", "Source", "emit", json!({}))],
        vec![],
        "final_value",
    );
    let error = host
        .emit_flow_token(
            &mut state,
            "source",
            "error",
            Token {
                token_type: "node_failure".into(),
                value: json!({}),
            },
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.reason_code(), Some(error_codes::CLOSED_OUTPUT));
}

#[tokio::test]
async fn hybrid_failures_stop_both_phases_and_use_the_failing_phases_error_port() {
    for policy in ["fail", "emit", "skip"] {
        for operation in ["materialize", "execute"] {
            let (host, mut state) = fixture(
                vec![
                    node("source", "Source", json!({"values":["delta"]})),
                    policy_node(
                        "hybrid",
                        "Join",
                        policy,
                        json!({"failure_at":operation,"snapshot":"saved","execution":"unused"}),
                    ),
                    node("errors", "ErrorSink", json!({})),
                ],
                vec![],
                "stream",
            );
            let mut defs = state.definitions.clone();
            let hybrid = defs.get_mut("Join").unwrap();
            hybrid.kind = FlowNodeClass::Hybrid;
            hybrid.output_ports.push(
                serde_json::from_value(
                    json!({"name":"snapshot","kind":"output","token_type":"text",
                "mode":"final_value","phase":"snapshot"}),
                )
                .unwrap(),
            );
            if policy == "emit" {
                hybrid.failure_ports = vec![
                    FlowFailurePort {
                        name: "snapshot_error".into(),
                        phase: FlowPortPhase::Snapshot,
                    },
                    FlowFailurePort {
                        name: "error".into(),
                        phase: FlowPortPhase::Execution,
                    },
                ];
            }
            let mut flow = state.flow.clone();
            flow.execution.source_order.push("hybrid".into());
            let mut edges = vec![edge("data", "source", "hybrid", 0, "stream")];
            if policy == "emit" {
                edges.push(error_edge(
                    "hybrid",
                    if operation == "materialize" {
                        "snapshot_error"
                    } else {
                        "error"
                    },
                ));
            }
            flow.edges = edges
                .into_iter()
                .map(|e| serde_json::from_value(e).unwrap())
                .collect();
            state.scheduler = SchedulerState::new("hybrid-failure".into(), &flow, defs).unwrap();
            assert_eq!(
                run(&host, &mut state).await.is_ok(),
                policy != "fail",
                "{policy} {operation}"
            );
            assert_eq!(
                state.seen.iter().filter(|(id, _)| id == "errors").count(),
                usize::from(policy == "emit")
            );
            if operation == "materialize" {
                assert!(!state.calls.iter().any(|id| id == "hybrid"));
            }
            assert!(!host
                .events
                .lock()
                .unwrap()
                .iter()
                .any(|e| e.node_id == "hybrid" && e.kind == EventKind::SignalEmit));
        }
    }
}

#[tokio::test]
async fn published_final_values_survive_a_handled_provider_completion_failure() {
    for policy in ["emit", "skip"] {
        let mut edges = vec![edge("data", "source", "sink", 0, "final_value")];
        if policy == "emit" {
            edges.push(error_edge("source", "error"));
        }
        let (host, mut state) = fixture(
            vec![
                policy_node(
                    "source",
                    "Source",
                    policy,
                    json!({"values":["retained"],"pump":true,"failure_at":"provider_complete"}),
                ),
                node("sink", "Sink", json!({})),
                node("errors", "ErrorSink", json!({})),
            ],
            edges,
            "final_value",
        );
        run(&host, &mut state).await.unwrap();
        assert_eq!(
            state
                .seen
                .iter()
                .filter(|(id, value)| id == "sink" && value == &json!("retained"))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn node_policies_settle_middle_fanout_without_redispatch() {
    for mode in ["final_value", "stream"] {
        for policy in ["fail", "emit", "skip"] {
            let mut edges = vec![
                edge("a", "source", "first", 0, mode),
                edge("b", "source", "middle", 1, mode),
                edge("c", "source", "last", 2, mode),
            ];
            if policy == "emit" {
                edges.push(error_edge("middle", "error"));
            }
            let (host, mut state) = fixture(
                vec![
                    node("source", "Source", json!({"values":["one"]})),
                    node("first", "Sink", json!({})),
                    policy_node("middle", "Sink", policy, json!({"failure_at":"sink"})),
                    node("last", "Sink", json!({})),
                    node("errors", "ErrorSink", json!({})),
                ],
                edges,
                mode,
            );
            let result = run(&host, &mut state).await;
            assert_eq!(result.is_ok(), policy != "fail", "{policy} {mode}");
            let ordinary: Vec<_> = state
                .seen
                .iter()
                .filter(|(id, _)| id != "errors")
                .map(|(id, _)| id.as_str())
                .collect();
            assert_eq!(
                ordinary,
                if policy == "fail" {
                    vec!["first"]
                } else {
                    vec!["first", "last"]
                }
            );
            let failures: Vec<_> = state.seen.iter().filter(|(id, _)| id == "errors").collect();
            assert_eq!(failures.len(), usize::from(policy == "emit"));
            if policy == "emit" {
                assert_eq!(failures[0].1["node_id"], "middle");
            }
            assert_eq!(state.retention.usage(), retention::Usage::default());
            let events = host.events.lock().unwrap();
            assert!(events.iter().any(|e| e.node_id == "middle"
                && e.kind == EventKind::NodeError
                && e.detail.as_ref().unwrap()["policy"] == policy));
        }
    }
}

#[tokio::test]
async fn partial_provider_stream_skips_dependent_execution_but_delivers_error() {
    for policy in ["fail", "emit", "skip"] {
        let mut edges = vec![edge("stream", "source", "join", 0, "stream")];
        if policy == "emit" {
            edges.push(error_edge("source", "error"));
        }
        let (host, mut state) = fixture(
            vec![
                policy_node(
                    "source",
                    "Source",
                    policy,
                    json!({"values":["prefix"],"pump":true,"recoverable_provider":true}),
                ),
                node("join", "Join", json!({})),
                node("errors", "ErrorSink", json!({})),
            ],
            edges,
            "stream",
        );
        assert_eq!(run(&host, &mut state).await.is_ok(), policy != "fail");
        assert!(state.seen.contains(&("join".into(), json!("prefix"))));
        assert!(!state.calls.iter().any(|id| id == "join"));
        assert_eq!(
            state.seen.iter().filter(|(id, _)| id == "errors").count(),
            usize::from(policy == "emit")
        );
        assert!(!host
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.node_id == "source" && e.kind == EventKind::SignalEmit));
    }
}

#[tokio::test]
async fn control_outcomes_bypass_emit_and_skip() {
    for policy in ["emit", "skip"] {
        for reason in ["cancel", "capacity", "effect", "unclassified"] {
            let (host, mut state) = fixture(
                vec![
                    policy_node(
                        "source",
                        "Source",
                        policy,
                        json!({"failure_at":"execute","failure_reason":reason}),
                    ),
                    node("errors", "ErrorSink", json!({})),
                ],
                if policy == "emit" {
                    vec![error_edge("source", "error")]
                } else {
                    vec![]
                },
                "final_value",
            );
            assert!(run(&host, &mut state).await.is_err(), "{policy} {reason}");
            assert!(state.seen.is_empty());
        }
    }
}

#[tokio::test]
async fn failure_handling_survives_every_publication_boundary_or_requires_resolution() {
    use battersea_runtime::recovery::{Journal, RecoveryError};
    for policy in ["emit", "skip"] {
        let make = || {
            fixture(
                vec![
                    node("source", "Source", json!({"values":["prefix"]})),
                    policy_node("sink", "Sink", policy, json!({"failure_at":"sink"})),
                    node("last", "Sink", json!({})),
                    node("errors", "ErrorSink", json!({})),
                ],
                {
                    let mut edges = vec![
                        edge("a", "source", "sink", 0, "final_value"),
                        edge("b", "source", "last", 1, "final_value"),
                    ];
                    if policy == "emit" {
                        edges.push(error_edge("sink", "error"));
                    }
                    edges
                },
                "final_value",
            )
        };
        let (mut host, mut state) = make();
        let baseline = Arc::new(RecoveryStore::default());
        host.recovery_store = Some(baseline.clone());
        run(&host, &mut state).await.unwrap();
        let expected = state.seen.clone();
        let writes = baseline.writes.load(Ordering::SeqCst);
        for after in [false, true] {
            for boundary in 1..=writes {
                let (mut host, mut state) = make();
                let store = Arc::new(RecoveryStore::default());
                store.fail_write.store(boundary, Ordering::SeqCst);
                store.fail_after.store(after, Ordering::SeqCst);
                host.recovery_store = Some(store.clone());
                assert!(run(&host, &mut state).await.is_err());
                drop(state);
                match Journal::open(store, "test-run") {
                    Err(RecoveryError::Missing) => {}
                    Ok(journal) => match journal.require_resumable() {
                        Ok(()) => {
                            let revision = journal.record().revision;
                            let mut restored = host.restore_activation(&journal).unwrap();
                            host.resume_activation(
                                &mut restored,
                                journal,
                                revision,
                                CancellationToken::new(),
                            )
                            .await
                            .unwrap();
                            assert_eq!(restored.seen, expected, "{policy} {boundary} {after}");
                        }
                        Err(RecoveryError::UnresolvedEffects(_))
                        | Err(RecoveryError::CommitPending) => {}
                        other => panic!("{other:?}"),
                    },
                    Err(error) => panic!("{error}"),
                }
                let events = host.events.lock().unwrap();
                assert!(
                    events
                        .iter()
                        .filter(|e| e.node_id == "sink" && e.kind == EventKind::NodeError)
                        .count()
                        <= 1
                );
                assert!(
                    events
                        .iter()
                        .filter(|e| e.node_id == "errors" && e.kind == EventKind::TokenReceive)
                        .count()
                        <= 1
                );
                assert!(
                    events
                        .iter()
                        .filter(|e| e.node_id == "last" && e.kind == EventKind::TokenReceive)
                        .count()
                        <= 1
                );
                let ids: HashSet<_> = events
                    .iter()
                    .map(|e| (&e.run_id, e.attempt, e.sequence))
                    .collect();
                assert_eq!(ids.len(), events.len());
            }
        }
    }
}

#[tokio::test]
async fn snapshot_failure_uses_snapshot_error_port_and_stops_remaining_node_phases() {
    for policy in ["fail", "emit", "skip"] {
        let (host, mut state) = fixture(
            vec![
                node("source", "Source", json!({})),
                policy_node(
                    "snapshot",
                    "Source",
                    policy,
                    json!({"failure_at":"materialize"}),
                ),
                node("errors", "ErrorSink", json!({})),
            ],
            if policy == "emit" {
                vec![error_edge("snapshot", "snapshot_error")]
            } else {
                vec![]
            },
            "final_value",
        );
        state
            .definitions
            .get_mut("Source")
            .unwrap()
            .interfaces
            .clear();
        let token = CancellationToken::new();
        let result = host
            .materialize_snapshot_sources(&mut state, "source", &token)
            .await;
        assert_eq!(result.is_ok(), policy != "fail");
        if result.is_ok() {
            host.drain_flow_work(&mut state, &token).await.unwrap();
            assert_eq!(
                state.seen.iter().filter(|(id, _)| id == "errors").count(),
                usize::from(policy == "emit")
            );
            assert!(!state.calls.iter().any(|id| id == "snapshot"));
        }
    }
}

#[tokio::test]
async fn a_nested_failure_keeps_the_destination_policy_and_attribution() {
    let (host, mut state) = fixture(
        vec![
            policy_node("source", "Source", "emit", json!({"start_peer":"peer"})),
            node("peer", "Source", json!({"failure_at":"execute"})),
            node("errors", "ErrorSink", json!({})),
        ],
        vec![error_edge("source", "error")],
        "final_value",
    );
    assert!(run(&host, &mut state).await.is_err());
    assert!(state.seen.is_empty());
    let events = host.events.lock().unwrap();
    let failures: Vec<_> = events
        .iter()
        .filter(|e| e.kind == EventKind::NodeError)
        .collect();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].node_id, "peer");
}

#[tokio::test]
async fn callback_policies_cover_sources_inline_receives_actions_and_provider_completion() {
    for policy in ["fail", "emit", "skip"] {
        for (kind, operation, mode) in [
            ("Source", "execute", "final_value"),
            ("Join", "execute", "final_value"),
            ("Join", "receive", "stream"),
            ("Action", "action", "final_value"),
            ("Source", "provider_complete", "final_value"),
            ("Source", "provider_event", "final_value"),
            ("Source", "controller", "final_value"),
        ] {
            let failing_source = kind == "Source";
            let failed = if failing_source { "source" } else { "target" };
            let params = if operation == "provider_complete" || operation == "provider_event" {
                json!({"failure_at":operation,"pump":true})
            } else {
                json!({"failure_at":operation})
            };
            let mut nodes = vec![
                policy_node(failed, kind, policy, params),
                node("errors", "ErrorSink", json!({})),
            ];
            let mut edges = vec![];
            if !failing_source {
                nodes.push(node(
                    "source",
                    "Source",
                    json!({"values":["value"],"signal":true}),
                ));
                edges.push(if kind == "Action" {
                    signal("action", "source", "target", "go")
                } else {
                    edge("data", "source", "target", 0, mode)
                });
            }
            if policy == "emit" {
                edges.push(error_edge(failed, "error"));
            }
            let (host, mut state) = fixture(nodes, edges, mode);
            assert_eq!(
                run(&host, &mut state).await.is_ok(),
                policy != "fail",
                "{policy} {operation}"
            );
            assert_eq!(
                state.seen.iter().filter(|(id, _)| id == "errors").count(),
                usize::from(policy == "emit"),
                "{policy} {operation}"
            );
        }
    }
}

#[tokio::test]
async fn durable_driver_resumes_after_a_saved_result_without_reexecuting_the_source() {
    use battersea_runtime::recovery::Journal;
    let (mut host, mut original) = fixture(
        vec![
            node("source", "Source", json!({"values":["retained"]})),
            node("sink", "Sink", json!({})),
        ],
        vec![edge("output", "source", "sink", 0, "final_value")],
        "final_value",
    );
    let store = Arc::new(RecoveryStore::default());
    store.fault.store(1, Ordering::SeqCst);
    host.recovery_store = Some(store.clone());
    let error = run(&host, &mut original).await.unwrap_err();
    assert_eq!(
        error.reason_code(),
        Some(error_codes::EXECUTION_JOURNAL_STORAGE_FAILED)
    );
    drop(original);
    let journal = Journal::open(store, "test-run").unwrap();
    journal.require_resumable().unwrap();
    let revision = journal.record().revision;
    let mut restored = host.restore_activation(&journal).unwrap();
    host.resume_activation(&mut restored, journal, revision, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(restored.seen, vec![("sink".into(), json!("retained"))]);
    let starts = host
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event.node_id == "source" && event.kind == EventKind::NodeStart)
        .count();
    assert_eq!(
        starts, 1,
        "resuming must not dispatch the completed source again"
    );
    assert!(matches!(
        restored.recovery_record().unwrap().phase,
        battersea_runtime::recovery::RecoveryPhase::ExecutionComplete
    ));
}

#[tokio::test]
async fn a_dispatched_step_without_a_durable_result_blocks_reexecution() {
    use battersea_runtime::recovery::{Journal, RecoveryError};
    let (mut host, mut original) = fixture(
        vec![node("source", "Source", json!({"values":["effect"]}))],
        vec![],
        "final_value",
    );
    let store = Arc::new(RecoveryStore::default());
    store.fault.store(3, Ordering::SeqCst);
    host.recovery_store = Some(store.clone());
    let error = run(&host, &mut original).await.unwrap_err();
    assert_eq!(
        error.reason_code(),
        Some(error_codes::EXECUTION_JOURNAL_STORAGE_FAILED)
    );
    let journal = Journal::open(store, "test-run").unwrap();
    assert!(matches!(
        journal.require_resumable(),
        Err(RecoveryError::UnresolvedEffects(_))
    ));
    let revision = journal.record().revision;
    let mut restored = host.restore_activation(&journal).unwrap();
    let error = host
        .resume_activation(&mut restored, journal, revision, CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.reason_code(), Some(error_codes::UNRESOLVED_EFFECTS));
    let starts = host
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event.node_id == "source" && event.kind == EventKind::NodeStart)
        .count();
    assert_eq!(
        starts, 1,
        "an uncertain effect cannot be repeated by resume"
    );
}

#[tokio::test]
async fn explicit_not_applied_resolution_allows_a_step_that_never_dispatched() {
    use battersea_runtime::recovery::{EffectState, Journal};
    let (mut host, mut original) = fixture(
        vec![node("source", "Source", json!({}))],
        vec![],
        "final_value",
    );
    let store = Arc::new(RecoveryStore::default());
    store.fault.store(2, Ordering::SeqCst);
    host.recovery_store = Some(store.clone());
    assert!(run(&host, &mut original).await.is_err());
    assert!(original.calls.is_empty());
    let mut journal = Journal::open(store, "test-run").unwrap();
    let effect = journal
        .record()
        .effects
        .iter()
        .find(|(_, effect)| matches!(effect.state, EffectState::Intent))
        .unwrap()
        .0
        .clone();
    journal
        .resolve_not_applied(
            &effect,
            "fixture-operator",
            "The injected write error prevented dispatch.",
        )
        .unwrap();
    let revision = journal.record().revision;
    let mut restored = host.restore_activation(&journal).unwrap();
    host.resume_activation(&mut restored, journal, revision, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(
        restored.calls.iter().filter(|id| *id == "source").count(),
        1
    );
}

#[tokio::test]
async fn checkpoint_preserves_pending_fanout_order_and_completed_source() {
    let (host, mut original) = fixture(
        vec![
            node("source", "Source", json!({"values":["retained"]})),
            node("later", "Sink", json!({})),
            node("earlier", "Sink", json!({})),
        ],
        vec![
            edge("later-edge", "source", "later", 2, "final_value"),
            edge("earlier-edge", "source", "earlier", 1, "final_value"),
        ],
        "final_value",
    );
    let cancel = CancellationToken::new();
    host.execute_flow_node(&mut original, "source", None, &cancel)
        .await
        .unwrap();
    let before = original.retention.usage();
    let checkpoint =
        battersea_runtime::checkpoint::SchedulerCheckpoint::capture(&original.scheduler).unwrap();
    let bytes = serde_json::to_vec(&checkpoint).unwrap();
    drop(original);
    let checkpoint: battersea_runtime::checkpoint::SchedulerCheckpoint =
        serde_json::from_slice(&bytes).unwrap();
    let mut restored = Run {
        scheduler: checkpoint.restore().unwrap(),
        seen: vec![],
        calls: vec![],
        held: vec![],
        max_usage: Default::default(),
    };
    assert_eq!(restored.retention.usage(), before);
    assert!(restored.executed_nodes.contains("source"));
    host.drain_flow_work(&mut restored, &cancel).await.unwrap();
    assert_eq!(
        restored
            .seen
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        vec!["earlier", "later"]
    );
    assert!(!restored.calls.iter().any(|id| id == "source"));
    assert!(restored
        .seen
        .iter()
        .all(|(_, value)| value == &json!("retained")));
    assert_eq!(restored.retention.usage().bytes, 0);
}

#[tokio::test]
async fn a_live_provider_cannot_be_misrepresented_as_a_restorable_checkpoint() {
    let (host, mut original) = fixture(
        vec![node(
            "source",
            "Source",
            json!({"values":["pending"],"pump":true,"never_close":true}),
        )],
        vec![],
        "stream",
    );
    host.execute_flow_node(&mut original, "source", None, &CancellationToken::new())
        .await
        .unwrap();
    assert!(
        battersea_runtime::checkpoint::SchedulerCheckpoint::capture(&original.scheduler).is_err()
    );
}

#[test]
fn checkpoint_versions_and_invalid_graph_state_are_rejected() {
    let (_, original) = fixture(
        vec![node("source", "Source", json!({}))],
        vec![],
        "final_value",
    );
    let checkpoint =
        battersea_runtime::checkpoint::SchedulerCheckpoint::capture(&original.scheduler).unwrap();
    let mut value = serde_json::to_value(&checkpoint).unwrap();
    value["version"] = json!(u32::MAX);
    let unknown: battersea_runtime::checkpoint::SchedulerCheckpoint =
        serde_json::from_value(value).unwrap();
    assert!(unknown.restore().is_err());
    let mut value = serde_json::to_value(&checkpoint).unwrap();
    value["executed_nodes"] = json!(["missing-node"]);
    let corrupt: battersea_runtime::checkpoint::SchedulerCheckpoint =
        serde_json::from_value(value).unwrap();
    assert!(corrupt.restore().is_err());
}

#[tokio::test]
async fn fanout_preserves_declared_order_and_fifo_independent_of_edge_storage() {
    let values = json!(["one", "two", "three"]);
    let edges = vec![
        edge("last", "source", "last", 9, "stream"),
        edge("b", "source", "second", 2, "stream"),
        edge("a", "source", "first", 2, "stream"),
    ];
    let nodes = vec![
        node("source", "Source", json!({"values":values,"pump":true})),
        node("first", "Sink", json!({})),
        node("second", "Sink", json!({})),
        node("last", "Sink", json!({})),
    ];
    let (host, mut state) = fixture(nodes.clone(), edges.clone(), "stream");
    run(&host, &mut state).await.unwrap();
    let expected = values
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|value| ["first", "second", "last"].map(|id| (id.to_string(), value.clone())))
        .collect::<Vec<_>>();
    assert_eq!(state.seen, expected);
    let (host, mut permuted) = fixture(
        nodes.into_iter().rev().collect(),
        edges.into_iter().rev().collect(),
        "stream",
    );
    run(&host, &mut permuted).await.unwrap();
    assert_eq!(permuted.seen, state.seen);
    assert_eq!(state.retention.usage(), retention::Usage::default());
}
#[tokio::test]
async fn a_middle_destination_failure_stops_later_branches_without_redispatch() {
    let (host, mut state) = fixture(
        vec![
            node(
                "source",
                "Source",
                json!({"values":["payload"],"pump":true}),
            ),
            node("first", "Sink", json!({})),
            node("middle", "Sink", json!({"fail":true})),
            node("last", "Sink", json!({})),
        ],
        vec![
            edge("first", "source", "first", 0, "stream"),
            edge("middle", "source", "middle", 1, "stream"),
            edge("last", "source", "last", 2, "stream"),
        ],
        "stream",
    );
    assert!(run(&host, &mut state).await.is_err());
    assert_eq!(
        state
            .seen
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        vec!["first"]
    );
    assert!(!state.calls.iter().any(|id| id == "last"));
    assert_eq!(state.retention.usage(), retention::Usage::default());
}
#[tokio::test]
async fn streaming_inputs_consume_once_and_final_turn_waits_for_successful_close() {
    for values in [json!([]), json!(["one", "two"])] {
        let (host, mut state) = fixture(
            vec![
                node("source", "Source", json!({"values":values,"pump":true})),
                node("join", "Join", json!({})),
            ],
            vec![edge("stream", "source", "join", 0, "stream")],
            "stream",
        );
        run(&host, &mut state).await.unwrap();
        assert_eq!(
            state
                .seen
                .iter()
                .map(|(_, value)| value)
                .collect::<Vec<_>>(),
            values.as_array().unwrap().iter().collect::<Vec<_>>()
        );
        assert_eq!(
            state
                .calls
                .iter()
                .filter(|id| id.as_str() == "join")
                .count(),
            1
        );
        assert!(state.held.is_empty());
        assert!(state.input_tokens.is_empty());
    }
}
#[tokio::test]
async fn a_final_value_is_emitted_once_and_remains_charged_while_the_handler_holds_it() {
    let (host, mut state) = fixture(
        vec![
            node("source", "Source", json!({"values":["retained"]})),
            node("join", "Join", json!({})),
        ],
        vec![edge("final", "source", "join", 0, "final_value")],
        "final_value",
    );
    run(&host, &mut state).await.unwrap();
    assert_eq!(state.held.len(), 1);
    assert!(state.retention.usage().bytes > 0);
    let held = state.held.pop().unwrap();
    let clone = held.clone();
    drop(held);
    assert!(state.retention.usage().bytes > 0);
    drop(clone);
    assert_eq!(state.retention.usage(), retention::Usage::default());
    let (host, mut state) = fixture(
        vec![
            node("source", "Source", json!({"values":["first","second"]})),
            node("sink", "Sink", json!({})),
        ],
        vec![edge("final", "source", "sink", 0, "final_value")],
        "final_value",
    );
    let error = run(&host, &mut state).await.unwrap_err();
    assert!(error.message().contains("final value"));
}
#[tokio::test]
async fn full_lossless_mailbox_runs_consumers_and_lossy_mailbox_records_each_discard() {
    let values = json!([1, 2, 3, 4, 5]);
    let nodes = vec![
        node("source", "Source", json!({"values":values})),
        node("sink", "Sink", json!({})),
    ];
    let link = edge("queue", "source", "sink", 0, "stream");
    let (host, mut state) = fixture(nodes.clone(), vec![link.clone()], "stream");
    run(&host, &mut state).await.unwrap();
    assert_eq!(
        state.seen.iter().map(|(_, v)| v).collect::<Vec<_>>(),
        values.as_array().unwrap().iter().collect::<Vec<_>>()
    );
    let mut lossy = link;
    lossy["queue"]["policy"] = json!("drop_oldest");
    let (host, mut state) = fixture(nodes, vec![lossy], "stream");
    run(&host, &mut state).await.unwrap();
    assert_eq!(
        state.seen.last().map(|(_, v)| v),
        values.as_array().unwrap().last()
    );
    let events = host.events.lock().unwrap();
    let discarded = events
        .iter()
        .filter(|e| e.kind == EventKind::TokenDrop)
        .collect::<Vec<_>>();
    assert_eq!(
        discarded.len() + state.seen.len(),
        values.as_array().unwrap().len()
    );
    assert!(discarded
        .iter()
        .all(|e| e.detail.as_ref().unwrap()["encodedBytes"].as_u64().unwrap() > 0));
}
#[tokio::test]
async fn cancellation_preempts_a_blocked_consumer_and_stops_a_full_provider_mailbox() {
    let (host, mut state) = fixture(
        vec![
            node(
                "source",
                "Source",
                json!({"values":[1,2,3,4],"pump":true,"never_close":true}),
            ),
            node("sink", "Sink", json!({"block":true})),
        ],
        vec![edge("queue", "source", "sink", 0, "stream")],
        "stream",
    );
    state.flow.execution.limits.provider_queue.items = 1;
    let token = CancellationToken::new();
    let cancel = token.clone();
    let entered = host.entered.clone();
    let watcher = tokio::spawn(async move {
        entered.notified().await;
        tokio::task::yield_now().await;
        cancel.cancel();
    });
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        host.run_activation(&mut state, "source", HashMap::new(), token),
    )
    .await
    .unwrap()
    .unwrap_err();
    watcher.await.unwrap();
    assert_eq!(result.code(), "cancelled");
    assert_eq!(
        result.reason_code(),
        Some(error_codes::ACTIVATION_CANCELLED)
    );
    let polls = host.polled.load(Ordering::SeqCst);
    tokio::task::yield_now().await;
    assert_eq!(host.polled.load(Ordering::SeqCst), polls);
    assert!(polls < 4);
    assert_eq!(state.retention.usage(), retention::Usage::default());
}
#[tokio::test]
async fn an_instrument_relays_its_signal_and_is_never_run_as_a_node() {
    let (host, mut state) = fixture(
        vec![
            node(
                "source",
                "Source",
                json!({"values":["first"],"signal":true}),
            ),
            node("sink", "Sink", json!({})),
            node("tap", "Tap", json!({"forward":true})),
            node("action", "Action", json!({})),
        ],
        vec![
            edge("data", "source", "sink", 0, "final_value"),
            signal("into-tap", "source", "tap", "go"),
            signal("out-of-tap", "tap", "action", "go"),
        ],
        "final_value",
    );
    run(&host, &mut state).await.unwrap();
    assert_eq!(
        state.calls.iter().filter(|id| *id == "tap").count(),
        1,
        "the tap runs once, for the signal on its action port"
    );
    assert!(
        state.calls.iter().any(|id| id == "action"),
        "the tap relays its signal"
    );
}

#[tokio::test]
async fn queued_signals_follow_data_and_signal_cycles_are_refused() {
    let (host, mut state) = fixture(
        vec![
            node(
                "source",
                "Source",
                json!({"values":["first"],"signal":true}),
            ),
            node("sink", "Sink", json!({})),
            node("action", "Action", json!({})),
        ],
        vec![
            edge("data", "source", "sink", 0, "final_value"),
            signal("signal", "source", "action", "go"),
        ],
        "final_value",
    );
    run(&host, &mut state).await.unwrap();
    let sink = state.calls.iter().position(|id| id == "sink").unwrap();
    let action = state.calls.iter().position(|id| id == "action").unwrap();
    assert!(sink < action);
    let (host, mut state) = fixture(
        vec![
            node("source", "Source", json!({"values":[],"signal":true})),
            node("a", "Action", json!({"forward":true})),
            node("b", "Action", json!({"forward":true})),
        ],
        vec![
            signal("start", "source", "a", "go"),
            signal("a-b", "a", "b", "go"),
            signal("b-a", "b", "a", "back"),
        ],
        "final_value",
    );
    let error = run(&host, &mut state).await.unwrap_err();
    assert_eq!(error.reason_code(), Some(error_codes::SIGNAL_CYCLE));
}
#[tokio::test]
async fn preflight_cancellation_happens_before_provider_dispatch() {
    let (host, mut state) = fixture(
        vec![
            node(
                "source",
                "Source",
                json!({"values":["never dispatched"],"pump":true,"preflight":true}),
            ),
            node("cancel", "Action", json!({"cancel":true})),
        ],
        vec![signal("cancel", "source", "cancel", "go")],
        "stream",
    );
    let error = run(&host, &mut state).await.unwrap_err();
    assert_eq!(error.code(), "cancelled");
    assert_eq!(error.reason_code(), Some(error_codes::ACTIVATION_CANCELLED));
    assert_eq!(host.polled.load(Ordering::SeqCst), 0);
    assert!(!state.calls.iter().any(|id| id == "source"));
}
#[test]
fn node_retention_enforces_limits_and_releases_replaced_values() {
    let value = json!({"text":"retained parser contents"});
    let bytes = serde_json::to_vec(&value).unwrap().len();
    let budget = RetentionBudget::new(10, bytes * 3, bytes);
    let first = budget.retain("parser", value.clone()).unwrap();
    assert!(budget.retain("parser", value.clone()).is_err());
    let other = budget.retain("other", value.clone()).unwrap();
    drop(first);
    let replacement = budget.retain("parser", value).unwrap();
    drop(other);
    drop(replacement);
    assert_eq!(budget.usage(), retention::Usage::default());
}

#[tokio::test]
async fn invalid_or_oversize_fanout_is_rejected_before_any_destination_or_observer() {
    for mismatch in [true, false] {
        let nodes = vec![
            node("source", "Source", json!({"values":["a bounded payload"]})),
            node("first", "Sink", json!({})),
            node("last", "Join", json!({})),
        ];
        let mut edges = vec![
            edge("first", "source", "first", 0, "stream"),
            edge("last", "source", "last", 1, "stream"),
        ];
        if !mismatch {
            edges[1]["queue"]["max_event_bytes"] = json!(1);
        }
        let (host, mut state) = fixture(nodes, edges, "stream");
        if mismatch {
            state.definitions.get_mut("Join").unwrap().input_ports[0].token_type = "other".into();
        }
        let refused = if mismatch {
            error_codes::INCOMPATIBLE_EDGE_TARGET
        } else {
            error_codes::PAYLOAD_TOO_LARGE
        };
        let error = run(&host, &mut state).await.unwrap_err();
        assert_eq!(error.reason_code(), Some(refused));
        assert!(state.seen.is_empty());
        assert!(!host
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.kind == EventKind::TokenEmit));
        assert_eq!(state.retention.usage(), retention::Usage::default());
    }
}

#[tokio::test]
async fn fanout_that_cannot_be_reserved_reports_capacity_deadlock_without_partial_delivery() {
    let (host, mut state) = fixture(
        vec![
            node("source", "Source", json!({"values":["one event"]})),
            node("a", "Sink", json!({})),
            node("b", "Sink", json!({})),
        ],
        vec![
            edge("a", "source", "a", 0, "stream"),
            edge("b", "source", "b", 1, "stream"),
        ],
        "stream",
    );
    let mut flow = state.flow.clone();
    flow.execution.limits.pending_events = 1;
    flow.execution.limits.provider_queue.items = 1;
    state.scheduler =
        SchedulerState::new("capacity".into(), &flow, state.definitions.clone()).unwrap();
    let error = run(&host, &mut state).await.unwrap_err();
    assert_eq!(error.reason_code(), Some(error_codes::CAPACITY_DEADLOCK));
    assert!(state.seen.is_empty());
    assert_eq!(state.retention.usage(), retention::Usage::default());
}

#[tokio::test]
async fn preflight_disable_prevents_dispatch_and_removes_the_disabled_dependency() {
    let (host, mut state) = fixture(
        vec![
            node(
                "source",
                "Source",
                json!({"values":["disabled"],"pump":true,"preflight":true}),
            ),
            node("join", "Join", json!({})),
        ],
        vec![
            signal("disable", "source", "source", "disable"),
            edge("stream", "source", "join", 0, "stream"),
        ],
        "stream",
    );
    run(&host, &mut state).await.unwrap();
    assert_eq!(host.polled.load(Ordering::SeqCst), 0);
    assert_eq!(
        state
            .calls
            .iter()
            .filter(|id| id.as_str() == "join")
            .count(),
        1
    );
}

#[tokio::test]
async fn provider_failure_does_not_satisfy_stream_closure() {
    let (host, mut state) = fixture(
        vec![
            node(
                "source",
                "Source",
                json!({"values":["delta"],"pump":true,"provider_fail":true}),
            ),
            node("join", "Join", json!({})),
        ],
        vec![edge("stream", "source", "join", 0, "stream")],
        "stream",
    );
    assert!(run(&host, &mut state).await.is_err());
    assert_eq!(state.seen.len(), 1);
    assert!(!state.calls.iter().any(|id| id == "join"));
    assert_eq!(state.retention.usage(), retention::Usage::default());
}

#[tokio::test]
async fn an_idle_provider_does_not_block_an_independent_ready_provider() {
    let (host, mut state) = fixture(
        vec![
            node(
                "source",
                "Source",
                json!({"values":[],"pump":true,"never_close":true,"start_peer":"peer"}),
            ),
            node(
                "peer",
                "Source",
                json!({"values":["ready peer"],"pump":true}),
            ),
            node("sink", "Sink", json!({"cancel":true})),
        ],
        vec![edge("peer", "peer", "sink", 0, "stream")],
        "stream",
    );
    let error = tokio::time::timeout(std::time::Duration::from_secs(5), run(&host, &mut state))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code(), "cancelled");
    assert_eq!(state.seen.len(), 1);
    assert!(host
        .events
        .lock()
        .unwrap()
        .iter()
        .any(|e| e.kind == EventKind::ProviderEvent && e.node_id == "peer"));
}

#[tokio::test]
async fn capacity_pressure_cannot_expose_later_fanout_branches_inside_an_earlier_destination() {
    let (host, mut state) = fixture(
        vec![
            node("source", "Source", json!({"values":["payload"]})),
            node("first", "Sink", json!({"forward":true})),
            node("last", "Sink", json!({})),
            node("collector", "Join", json!({})),
        ],
        vec![
            edge("first", "source", "first", 0, "stream"),
            edge("last", "source", "last", 1, "stream"),
        ],
        "stream",
    );
    let mut definitions = state.definitions.clone();
    definitions.get_mut("Sink").unwrap().output_ports.push(serde_json::from_value(json!({"name":"out","kind":"output","token_type":"text","mode":"stream","phase":"execution"})).unwrap());
    let mut flow = state.flow.clone();
    flow.edges.push(
        serde_json::from_value(edge("downstream", "first", "collector", 0, "stream")).unwrap(),
    );
    flow.execution.limits.pending_events = 2;
    flow.execution.limits.provider_queue.items = 1;
    state.scheduler = SchedulerState::new("capacity".into(), &flow, definitions).unwrap();
    let error = run(&host, &mut state).await.unwrap_err();
    assert!(error.message().contains("Capacity deadlock"));
    assert!(!state.calls.iter().any(|id| id == "last"));
    assert_eq!(state.retention.usage(), retention::Usage::default());
}

#[tokio::test]
async fn a_busy_provider_cannot_starve_a_ready_peer() {
    let (host, mut state) = fixture(
        vec![
            node(
                "source",
                "Source",
                json!({"values":["busy"],"pump":true,"repeat":true,"start_peer":"z-peer"}),
            ),
            node("z-peer", "Source", json!({"values":["peer"],"pump":true})),
            node("busy", "Sink", json!({})),
            node("cancel", "Sink", json!({"cancel":true})),
        ],
        vec![
            edge("busy", "source", "busy", 0, "stream"),
            edge("peer", "z-peer", "cancel", 0, "stream"),
        ],
        "stream",
    );
    let error = tokio::time::timeout(std::time::Duration::from_secs(5), run(&host, &mut state))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code(), "cancelled");
    assert!(state.seen.iter().any(|(id, _)| id == "cancel"));
    assert!(state.max_usage.items <= state.flow.execution.limits.pending_events as usize);
    assert!(state.max_usage.bytes <= state.flow.execution.limits.retained_bytes as usize);
}

#[tokio::test]
async fn asynchronous_snapshot_dependencies_precede_authored_source_priority() {
    for reverse_storage in [false, true] {
        let payload = json!("upstream snapshot");
        let mut nodes = vec![
            node("source", "Action", json!({})),
            node(
                "dependent",
                "Source",
                json!({"require_input":payload, "values":[]}),
            ),
            node(
                "upstream",
                "Source",
                json!({"values":[payload], "pump":true}),
            ),
        ];
        if reverse_storage {
            nodes.reverse();
        }
        let (host, mut state) = fixture(nodes, vec![], "final_value");
        let mut definitions = state.definitions.clone();
        let source = definitions.get_mut("Source").unwrap();
        source.interfaces.clear();
        source.output_ports[0].phase = FlowPortPhase::Snapshot;
        source.input_ports.push(serde_json::from_value(json!({"name":"in","kind":"input","token_type":"text","mode":"final_value","phase":"snapshot"})).unwrap());
        let mut flow = state.flow.clone();
        flow.execution.source_order = vec!["dependent".into(), "upstream".into()];
        flow.edges.push(
            serde_json::from_value(edge("snapshot", "upstream", "dependent", 0, "final_value"))
                .unwrap(),
        );
        state.scheduler = SchedulerState::new("snapshots".into(), &flow, definitions).unwrap();
        run(&host, &mut state).await.unwrap();
        assert_eq!(state.calls, ["source", "upstream", "dependent"]);
        assert_eq!(state.retention.usage(), retention::Usage::default());
    }
}

#[tokio::test]
async fn hybrid_snapshot_and_execution_outputs_close_only_in_their_own_phase() {
    let snapshot = json!("snapshot payload");
    let execution = json!("execution payload");
    let (host, mut state) = fixture(
        vec![
            node("source", "Source", json!({"values":["delta"], "pump":true})),
            node(
                "hybrid",
                "Join",
                json!({"snapshot":snapshot, "execution":execution}),
            ),
            node("sink", "Sink", json!({})),
        ],
        vec![],
        "stream",
    );
    let mut definitions = state.definitions.clone();
    let hybrid = definitions.get_mut("Join").unwrap();
    hybrid.kind = FlowNodeClass::Hybrid;
    hybrid.output_ports = serde_json::from_value(json!([
        {"name":"snapshot","kind":"output","token_type":"text","mode":"final_value","phase":"snapshot"},
        {"name":"out","kind":"output","token_type":"text","mode":"stream","phase":"execution"}
    ])).unwrap();
    let mut snapshot_sink = definitions["Sink"].clone();
    snapshot_sink.class_name = "SnapshotSink".into();
    snapshot_sink.input_ports[0].mode = FlowPortMode::FinalValue;
    definitions.insert(snapshot_sink.class_name.clone(), snapshot_sink);
    let mut flow = state.flow.clone();
    flow.nodes
        .push(serde_json::from_value(node("snapshot-sink", "SnapshotSink", json!({}))).unwrap());
    flow.execution.source_order.push("hybrid".into());
    flow.edges = vec![
        edge("input", "source", "hybrid", 0, "stream"),
        edge("output", "hybrid", "sink", 0, "stream"),
        {
            let mut edge = edge("snapshot", "hybrid", "snapshot-sink", 0, "final_value");
            edge["source_port"] = json!("snapshot");
            edge
        },
    ]
    .into_iter()
    .map(|v| serde_json::from_value(v).unwrap())
    .collect();
    state.scheduler = SchedulerState::new("hybrid".into(), &flow, definitions).unwrap();
    run(&host, &mut state).await.unwrap();
    assert_eq!(
        state
            .seen
            .iter()
            .find(|(id, _)| id == "snapshot-sink")
            .unwrap()
            .1,
        snapshot
    );
    assert_eq!(
        state.seen.iter().find(|(id, _)| id == "sink").unwrap().1,
        execution
    );
    assert!(
        state.calls.iter().position(|id| id == "hybrid:snapshot")
            < state.calls.iter().position(|id| id == "hybrid")
    );
    assert_eq!(
        state
            .calls
            .iter()
            .filter(|id| id.as_str() == "hybrid")
            .count(),
        1
    );
    assert_eq!(state.retention.usage(), retention::Usage::default());
}

#[tokio::test]
async fn every_driver_publication_boundary_stops_or_resumes_without_repeating_effects() {
    use battersea_runtime::recovery::{Journal, RecoveryError, RecoveryPhase};
    let make = || {
        fixture(
            vec![
                node(
                    "source",
                    "Source",
                    json!({"values":["retained"], "start_peer":"optional-source"}),
                ),
                node("optional-source", "Source", json!({})),
                node("optional-join", "Join", json!({})),
                node("optional-sink", "Sink", json!({})),
                node("sink", "Sink", json!({})),
            ],
            vec![
                edge("output", "source", "sink", 0, "final_value"),
                edge(
                    "optional-input",
                    "optional-source",
                    "optional-join",
                    0,
                    "final_value",
                ),
                edge(
                    "optional-output",
                    "optional-join",
                    "optional-sink",
                    0,
                    "final_value",
                ),
            ],
            "final_value",
        )
    };
    let (mut host, mut state) = make();
    let baseline = Arc::new(RecoveryStore::default());
    host.recovery_store = Some(baseline.clone());
    run(&host, &mut state).await.unwrap();
    let writes = baseline.writes.load(Ordering::SeqCst);
    assert!(writes > 1);
    for after in [false, true] {
        for boundary in 1..=writes {
            let (mut host, mut state) = make();
            let store = Arc::new(RecoveryStore::default());
            store.fail_write.store(boundary, Ordering::SeqCst);
            store.fail_after.store(after, Ordering::SeqCst);
            host.recovery_store = Some(store.clone());
            assert!(
                run(&host, &mut state).await.is_err(),
                "boundary {boundary}, after {after}"
            );
            drop(state);
            match Journal::open(store, "test-run") {
                Err(RecoveryError::Missing) => {}
                Ok(journal) => match journal.require_resumable() {
                    Ok(()) => {
                        let revision = journal.record().revision;
                        let mut resumed = host.restore_activation(&journal).unwrap();
                        host.resume_activation(
                            &mut resumed,
                            journal,
                            revision,
                            CancellationToken::new(),
                        )
                        .await
                        .unwrap();
                        assert!(matches!(
                            resumed.recovery_record().unwrap().phase,
                            RecoveryPhase::ExecutionComplete
                        ));
                    }
                    Err(RecoveryError::UnresolvedEffects(_))
                    | Err(RecoveryError::CommitPending) => {}
                    other => panic!("unexpected recovery result: {other:?}"),
                },
                Err(error) => panic!("unexpected journal error: {error}"),
            }
            let events = host.events.lock().unwrap();
            let identities: HashSet<_> = events
                .iter()
                .map(|event| (&event.run_id, event.attempt, event.sequence))
                .collect();
            assert_eq!(
                identities.len(),
                events.len(),
                "resumed observations keep distinct identities"
            );
            assert!(
                events
                    .iter()
                    .filter(|event| event.node_id == "source" && event.kind == EventKind::NodeStart)
                    .count()
                    <= 1
            );
            assert!(events.iter().filter(|event| event.node_id == "sink" && event.kind == EventKind::TokenReceive).count() <= 1);
        }
    }
}

#[tokio::test]
async fn incompatible_handler_state_is_rejected_without_dispatch_or_publication() {
    use battersea_runtime::recovery::Journal;
    let (mut host, mut original) = fixture(
        vec![node("source", "Source", json!({}))],
        vec![],
        "final_value",
    );
    let store = Arc::new(RecoveryStore::default());
    store.fault.store(2, Ordering::SeqCst);
    host.recovery_store = Some(store.clone());
    assert!(run(&host, &mut original).await.is_err());
    {
        let mut stored = store.record.lock().unwrap();
        stored.as_mut().unwrap().checkpoint.value["handler_versions"]["test.node"] = json!(2);
    }
    let before = serde_json::to_vec(store.record.lock().unwrap().as_ref().unwrap()).unwrap();
    let journal = Journal::open(store.clone(), "test-run").unwrap();
    let Err(error) = host.restore_activation(&journal) else {
        panic!("a changed handler version cannot restore");
    };
    assert_eq!(
        error.reason_code(),
        Some(error_codes::INCOMPATIBLE_CHECKPOINT)
    );
    assert_eq!(
        serde_json::to_vec(store.record.lock().unwrap().as_ref().unwrap()).unwrap(),
        before
    );
    assert!(original.calls.is_empty());
}

#[tokio::test]
async fn absent_snapshot_dependencies_close_streams_successfully() {
    use battersea_runtime::checkpoint::SchedulerCheckpoint;
    let (host, mut state) = fixture(
        vec![
            node("source", "Source", json!({})),
            node("upstream", "Source", json!({"values":[]})),
            node("middle", "Join", json!({})),
            node("consumer", "Join", json!({})),
        ],
        vec![],
        "stream",
    );
    let mut defs = state.definitions.clone();
    let mut upstream = defs["Source"].clone();
    upstream.class_name = "Snapshot".into();
    upstream.interfaces.clear();
    upstream.output_ports[0].phase = FlowPortPhase::Snapshot;
    upstream.output_ports[0].mode = FlowPortMode::FinalValue;
    let mut middle = defs["Join"].clone();
    middle.class_name = "Hybrid".into();
    middle.kind = FlowNodeClass::Hybrid;
    middle.input_ports[0].phase = FlowPortPhase::Snapshot;
    middle.input_ports[0].mode = FlowPortMode::FinalValue;
    defs.insert(upstream.class_name.clone(), upstream);
    defs.insert(middle.class_name.clone(), middle);
    let mut flow = state.flow.clone();
    flow.nodes
        .iter_mut()
        .find(|n| n.id == "upstream")
        .unwrap()
        .definition_name = "Snapshot".into();
    flow.nodes
        .iter_mut()
        .find(|n| n.id == "middle")
        .unwrap()
        .definition_name = "Hybrid".into();
    flow.execution.source_order = vec!["source".into(), "middle".into(), "upstream".into()];
    flow.edges = vec![
        edge("snapshot", "upstream", "middle", 0, "final_value"),
        edge("empty", "middle", "consumer", 0, "stream"),
    ]
    .into_iter()
    .map(|e| serde_json::from_value(e).unwrap())
    .collect();
    state.scheduler = SchedulerState::new("absent-snapshot".into(), &flow, defs).unwrap();
    run(&host, &mut state).await.unwrap();
    assert!(!state
        .calls
        .iter()
        .any(|id| id == "middle" || id == "middle:snapshot"));
    assert!(state.calls.iter().any(|id| id == "consumer"));
    let saved = serde_json::to_value(SchedulerCheckpoint::capture(&state).unwrap()).unwrap();
    assert_eq!(saved["failed_outputs"], json!([]));
    assert_eq!(saved["failed_inputs"], json!([]));
    assert!(serde_json::from_value::<SchedulerCheckpoint>(saved.clone())
        .unwrap()
        .restore()
        .is_ok());
    let mut missing = saved;
    missing["dependency_skips"] = json!({});
    assert!(serde_json::from_value::<SchedulerCheckpoint>(missing)
        .unwrap()
        .restore()
        .is_err());
}

#[tokio::test]
async fn runtime_delivery_refusals_record_one_destination_error() {
    for policy in ["fail", "emit", "skip"] {
        let (host, mut state) = fixture(
            vec![
                node("source", "Source", json!({"values":["second"]})),
                policy_node("join", "Join", policy, json!({})),
            ],
            vec![edge("data", "source", "join", 0, "final_value")],
            "final_value",
        );
        let edge = state.flow.edges[0].clone();
        let value = state
            .retention
            .retain(
                "join",
                Token {
                    token_type: "text".into(),
                    value: json!("first"),
                },
            )
            .unwrap();
        host.deliver_flow_token(&mut state, &edge, value, &CancellationToken::new())
            .await
            .unwrap();
        let error = run(&host, &mut state).await.unwrap_err();
        assert_eq!(
            error.reason_code(),
            Some(error_codes::DUPLICATE_FINAL_VALUE)
        );
        let events = host.events.lock().unwrap();
        let errors = events
            .iter()
            .filter(|e| e.kind == EventKind::NodeError)
            .collect::<Vec<_>>();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].node_id, "join");
        assert_eq!(
            errors[0].detail.as_ref().unwrap()["error"]["code"],
            error_codes::DUPLICATE_FINAL_VALUE
        );
    }
}

#[tokio::test]
async fn absent_final_values_settle_branches_without_discarding_independent_results() {
    for pumped in [false, true] {
        let response = json!("retained response");
        let mut other_input = edge("other", "response", "join", 1, "final_value");
        other_input["target_port"] = json!("unused");
        let (host, mut state) = fixture(
            vec![
                node(
                    "source",
                    "Source",
                    json!({"values": [], "pump": pumped, "start_peer": "response"}),
                ),
                node("response", "Source", json!({"values": [response]})),
                node("optional", "Join", json!({})),
                node("join", "Join", json!({})),
                node("empty-sink", "Sink", json!({})),
                node("response-sink", "Sink", json!({})),
            ],
            vec![
                edge("absent", "source", "optional", 0, "final_value"),
                edge("propagated", "optional", "join", 0, "final_value"),
                other_input,
                edge("empty", "join", "empty-sink", 0, "final_value"),
                edge("result", "response", "response-sink", 0, "final_value"),
            ],
            "final_value",
        );
        run(&host, &mut state)
            .await
            .expect("absence is successful closure, not a stuck flow");
        assert_eq!(state.seen, vec![("response-sink".into(), response)]);
        for id in ["optional", "join", "empty-sink"] {
            assert!(
                !state.calls.iter().any(|called| called == id),
                "no handler may receive an invented value"
            );
            assert!(
                state.executed_nodes.contains(id),
                "absence must settle downstream branches"
            );
        }
        for id in ["optional", "join"] {
            assert!(state
                .signal_sources_settled
                .contains(&(id.into(), "done".into())));
            assert!(state
                .signal_sources_settled
                .contains(&(id.into(), "post_activate".into())));
        }
        assert!(state.input_tokens.is_empty());
        assert_eq!(state.retention.usage(), retention::Usage::default());
        let events = host.events.lock().unwrap();
        let skipped = events
            .iter()
            .filter(|event| event.kind == EventKind::NodeSkipped)
            .map(|event| event.node_id.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(skipped, HashSet::from(["optional", "join"]));
        assert!(!events
            .iter()
            .any(|event| event.kind == EventKind::SignalEmit
                && skipped.contains(event.node_id.as_str())));
    }
}

#[tokio::test]
async fn an_unresolved_dependency_cycle_still_fails_instead_of_becoming_an_absent_branch() {
    let (host, mut state) = fixture(
        vec![
            node("source", "Source", json!({})),
            node("left", "Join", json!({})),
            node("right", "Join", json!({})),
        ],
        vec![
            edge("left-right", "left", "right", 0, "final_value"),
            edge("right-left", "right", "left", 0, "final_value"),
        ],
        "final_value",
    );
    let error = run(&host, &mut state)
        .await
        .expect_err("an open dependency is not an absent final value");
    assert_eq!(error.code(), "invalid_request");
    assert_eq!(error.reason_code(), Some(error_codes::UNSATISFIED_INPUTS));
    assert!(!host
        .events
        .lock()
        .unwrap()
        .iter()
        .any(|event| event.kind == EventKind::NodeSkipped));
}

/// The issues generic validation reports for `flow` against the fixture definitions.
fn validation_issues(flow: &FlowDocument) -> Vec<FlowValidationIssue> {
    let mut registry = battersea_flow::registry::Registry::default();
    registry.register_token_type("text", json!(true)).unwrap();
    registry.register_handler("test.node").unwrap();
    battersea_flow::validation::validate_document(flow, &definitions("final_value"), &registry)
        .issues
}
fn names(issue: &FlowValidationIssue, code: &str, node_id: &str, edge_id: &str) -> bool {
    issue.code == code
        && issue.node_id.as_deref() == Some(node_id)
        && issue.edge_id.as_deref() == Some(edge_id)
}

#[test]
fn the_scheduler_refuses_an_edge_shape_with_the_issues_validation_reports() {
    let sources = || {
        vec![
            node("first", "Source", json!({})),
            node("second", "Source", json!({})),
        ]
    };
    let with = |extra: Vec<Value>| sources().into_iter().chain(extra).collect::<Vec<_>>();
    let shapes = [
        (
            with(vec![node("sink", "Sink", json!({}))]),
            vec![
                edge("first-in", "first", "sink", 0, "final_value"),
                edge("second-in", "second", "sink", 1, "final_value"),
            ],
            vec![(issue_codes::DUPLICATE_INPUT_EDGE, "sink", "second-in")],
        ),
        (
            with(vec![
                node("a", "Action", json!({})),
                node("b", "Action", json!({})),
            ]),
            vec![
                signal("to-a", "first", "a", "go"),
                signal("to-b", "first", "b", "go"),
            ],
            vec![(issue_codes::DUPLICATE_SIGNAL_EDGE, "first", "to-b")],
        ),
        (
            with(vec![node("action", "Action", json!({}))]),
            vec![
                signal("first-go", "first", "action", "go"),
                signal("second-go", "second", "action", "go"),
            ],
            vec![(issue_codes::DUPLICATE_ACTION_EDGE, "action", "second-go")],
        ),
        (
            with(vec![
                node("sink", "Sink", json!({})),
                node("action", "Action", json!({})),
            ]),
            vec![
                edge("first-in", "first", "sink", 0, "final_value"),
                edge("second-in", "second", "sink", 1, "final_value"),
                signal("first-go", "first", "action", "go"),
                signal("second-go", "second", "action", "go"),
            ],
            vec![
                (issue_codes::DUPLICATE_INPUT_EDGE, "sink", "second-in"),
                (issue_codes::DUPLICATE_ACTION_EDGE, "action", "second-go"),
            ],
        ),
    ];
    for (nodes, edges, expected) in shapes {
        let flow = document(nodes, edges);
        let issues =
            SchedulerState::new("refused".into(), &flow, definitions("final_value")).unwrap_err();
        let validated = validation_issues(&flow);
        for (code, node_id, edge_id) in expected {
            assert!(
                issues
                    .iter()
                    .any(|issue| names(issue, code, node_id, edge_id)),
                "expected {code} for {node_id} and {edge_id}: {issues:?}"
            );
        }
        for issue in &issues {
            assert!(
                validated.iter().any(|reported| reported.code == issue.code
                    && reported.node_id == issue.node_id
                    && reported.edge_id == issue.edge_id),
                "validation does not report {issue:?}: {validated:?}"
            );
        }
    }

    let fanned_out = document(
        with(vec![
            node("sink", "Sink", json!({})),
            node("join", "Join", json!({})),
            node("action", "Action", json!({})),
        ]),
        vec![
            edge("to-sink", "first", "sink", 0, "final_value"),
            edge("to-join", "first", "join", 1, "final_value"),
            signal("go", "first", "action", "go"),
            signal("back", "second", "action", "back"),
        ],
    );
    assert!(validation_issues(&fanned_out).is_empty());
    SchedulerState::new("accepted".into(), &fanned_out, definitions("final_value")).unwrap();
}

#[test]
fn the_scheduler_refuses_a_flow_outside_its_execution_contract_with_its_issue_code() {
    let flow = document(
        vec![
            node("source", "Source", json!({})),
            node("action", "Action", json!({})),
        ],
        vec![signal("go", "source", "action", "go")],
    );
    SchedulerState::new("accepted".into(), &flow, definitions("final_value")).unwrap();

    let mut queued = flow.clone();
    queued.edges[0].queue = Some(FlowQueueLimits::default());
    let mut upgraded = flow;
    upgraded.version += 1;
    for (refused, code) in [
        (queued, issue_codes::INVALID_EXECUTION_CONTRACT),
        (upgraded, issue_codes::UNSUPPORTED_DOCUMENT_VERSION),
    ] {
        let issues = SchedulerState::new("refused".into(), &refused, definitions("final_value"))
            .unwrap_err();
        assert!(issues.iter().any(|issue| issue.code == code), "{issues:?}");
        assert!(validation_issues(&refused)
            .iter()
            .any(|issue| issue.code == code));
    }
}

#[tokio::test]
async fn entry_points_a_handler_does_not_implement_are_refused() {
    let (host, mut state) = fixture(
        vec![node("source", "Source", json!({}))],
        vec![],
        "final_value",
    );
    let node = state.nodes_by_id["source"].clone();
    let definition = state.definitions["Source"].clone();
    let handler = host.handlers().get("test.node").unwrap();
    let controller = handler
        .execute_controller_action(&host, &mut state, &node, &definition, "refresh")
        .await
        .unwrap_err();
    let event = handler
        .receive_provider_event(
            &host,
            &mut state,
            &node,
            &definition,
            &json!({}),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    for error in [controller, event] {
        assert_eq!(
            error.reason_code(),
            Some(error_codes::UNSUPPORTED_HANDLER_OPERATION)
        );
    }
}

#[tokio::test]
async fn execution_names_the_node_definition_or_input_it_cannot_find() {
    let (host, mut state) = fixture(
        vec![
            node("source", "Source", json!({})),
            node("sink", "Sink", json!({})),
        ],
        vec![edge("in", "source", "sink", 0, "final_value")],
        "final_value",
    );
    let token = CancellationToken::new();
    let missing = host
        .execute_flow_node(&mut state, "missing", None, &token)
        .await
        .unwrap_err();
    assert_eq!(missing.reason_code(), Some(error_codes::UNKNOWN_NODE));
    let Err(input) = host.take_flow_input_token(&mut state, "sink", "in") else {
        panic!("no value was delivered");
    };
    assert_eq!(input.reason_code(), Some(error_codes::MISSING_INPUT_VALUE));
    state.definitions.remove("Source");
    let definition = host
        .execute_flow_node(&mut state, "source", None, &token)
        .await
        .unwrap_err();
    assert_eq!(
        definition.reason_code(),
        Some(error_codes::UNKNOWN_DEFINITION)
    );
}

#[tokio::test]
async fn emission_is_refused_on_an_unknown_mistyped_or_closed_output() {
    let (host, mut state) = fixture(
        vec![node("source", "Source", json!({}))],
        vec![],
        "final_value",
    );
    let token = CancellationToken::new();
    let value = |token_type: &str| Token {
        token_type: token_type.into(),
        value: json!("emitted"),
    };
    for (port, token_type, reason) in [
        ("missing", "text", error_codes::UNKNOWN_OUTPUT_PORT),
        ("out", "number", error_codes::INCOMPATIBLE_TOKEN_TYPE),
    ] {
        let error = host
            .emit_flow_token(&mut state, "source", port, value(token_type), &token)
            .await
            .unwrap_err();
        assert_eq!(error.reason_code(), Some(reason), "{port}");
    }
    host.emit_flow_token(&mut state, "source", "out", value("text"), &token)
        .await
        .unwrap();
    let closed = host
        .emit_flow_token(&mut state, "source", "out", value("text"), &token)
        .await
        .unwrap_err();
    assert_eq!(closed.reason_code(), Some(error_codes::CLOSED_OUTPUT));
}

#[tokio::test]
async fn a_final_value_input_accepts_one_value() {
    let (host, mut state) = fixture(
        vec![
            node("source", "Source", json!({})),
            node("join", "Join", json!({})),
        ],
        vec![edge("in", "source", "join", 0, "final_value")],
        "final_value",
    );
    let edge = state.flow.edges[0].clone();
    let token = CancellationToken::new();
    let value = |state: &Run| {
        state
            .retention
            .retain(
                "join",
                Token {
                    token_type: "text".into(),
                    value: json!("delivered"),
                },
            )
            .unwrap()
    };
    let first = value(&state);
    host.deliver_flow_token(&mut state, &edge, first, &token)
        .await
        .unwrap();
    let second = value(&state);
    let error = host
        .deliver_flow_token(&mut state, &edge, second, &token)
        .await
        .unwrap_err();
    assert_eq!(
        error.reason_code(),
        Some(error_codes::DUPLICATE_FINAL_VALUE)
    );
}

#[tokio::test]
async fn only_an_output_capable_node_can_be_disabled() {
    let (host, mut state) = fixture(
        vec![
            node("source", "Source", json!({})),
            node("action", "Action", json!({})),
        ],
        vec![],
        "final_value",
    );
    let disable = battersea_flow::catalog::FLOW_ACTION_DISABLE;
    let action = state.nodes_by_id["action"].clone();
    let error = host
        .execute_flow_action(&mut state, &action, disable)
        .await
        .unwrap_err();
    assert_eq!(
        error.reason_code(),
        Some(error_codes::UNSUPPORTED_ACTION_PORT)
    );
    let source = state.nodes_by_id["source"].clone();
    host.execute_flow_action(&mut state, &source, disable)
        .await
        .unwrap();
    assert!(state.disabled_output_nodes.contains("source"));
}

#[tokio::test]
async fn a_node_runs_one_provider_at_a_time() {
    let (host, mut state) = fixture(
        vec![node(
            "source",
            "Source",
            json!({"values":["first"],"pump":true,"never_close":true,"restart":true}),
        )],
        vec![],
        "stream",
    );
    let error = run(&host, &mut state).await.unwrap_err();
    assert_eq!(error.reason_code(), Some(error_codes::DUPLICATE_PROVIDER));
}

#[tokio::test]
async fn a_panicking_provider_interrupts_the_activation() {
    let (host, mut state) = fixture(
        vec![node(
            "source",
            "Source",
            json!({"values":[],"pump":true,"panic":true}),
        )],
        vec![],
        "stream",
    );
    let error = run(&host, &mut state).await.unwrap_err();
    assert_eq!(error.code(), "interrupted");
    assert_eq!(error.reason_code(), Some(error_codes::PROVIDER_STOPPED));
    assert_eq!(state.retention.usage(), retention::Usage::default());
}

#[tokio::test]
async fn resume_requires_durable_storage() {
    use battersea_runtime::recovery::Journal;
    let (mut host, mut original) = fixture(
        vec![
            node("source", "Source", json!({"values":["retained"]})),
            node("sink", "Sink", json!({})),
        ],
        vec![edge("output", "source", "sink", 0, "final_value")],
        "final_value",
    );
    let store = Arc::new(RecoveryStore::default());
    store.fault.store(1, Ordering::SeqCst);
    host.recovery_store = Some(store.clone());
    run(&host, &mut original).await.unwrap_err();
    let journal = Journal::open(store, "test-run").unwrap();
    host.recovery_store = None;
    let Err(error) = host.restore_activation(&journal) else {
        panic!("resume without storage cannot restore");
    };
    assert_eq!(
        error.reason_code(),
        Some(error_codes::DURABLE_STORAGE_REQUIRED)
    );
}

#[tokio::test]
async fn a_host_state_that_cannot_be_checkpointed_leaves_the_run_requiring_recovery() {
    let (mut host, mut state) = fixture(
        vec![
            node("source", "Source", json!({"values":["held"]})),
            node("join", "Join", json!({})),
        ],
        vec![edge("in", "source", "join", 0, "final_value")],
        "final_value",
    );
    host.recovery_store = Some(Arc::new(RecoveryStore::default()));
    let error = run(&host, &mut state).await.unwrap_err();
    assert_eq!(error.code(), "interrupted", "the default recovery class");
    assert_eq!(
        error.reason_code(),
        Some(error_codes::EXECUTION_INTERRUPTED)
    );
}

#[test]
fn every_journal_and_pump_failure_is_named_by_a_declared_code() {
    use battersea_runtime::{pump::PumpError, recovery::RecoveryError};
    let journal = [
        RecoveryError::Missing,
        RecoveryError::Conflict,
        RecoveryError::Incompatible("version".into()),
        RecoveryError::Invalid("operation".into()),
        RecoveryError::Storage("write".into()),
        RecoveryError::UnresolvedEffects(vec!["effect".into()]),
        RecoveryError::CommitPending,
        RecoveryError::Terminal,
        RecoveryError::Poisoned,
    ]
    .map(|error| error.code());
    let pump = [
        PumpError::InvalidLimits,
        PumpError::PayloadTooLarge { limit: 1 },
        PumpError::Serialization("value".into()),
        PumpError::Capacity("budget".into()),
        PumpError::Cancelled,
        PumpError::ProducerPanicked,
        PumpError::ProducerStopped,
    ]
    .map(|error| error.code());
    for code in journal.iter().chain(&pump) {
        assert!(error_codes::ALL.contains(code), "{code}");
    }
    assert_eq!(
        journal.iter().collect::<HashSet<_>>().len(),
        journal.len(),
        "each journal failure has its own code"
    );
}
