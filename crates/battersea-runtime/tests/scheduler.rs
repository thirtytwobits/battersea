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
struct Error(&'static str, String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.0, self.1)
    }
}
impl std::error::Error for Error {}
impl ExecutionError for Error {
    fn invalid_request(s: impl Into<String>) -> Self {
        Self("invalid_request", s.into())
    }
    fn internal(s: impl Into<String>) -> Self {
        Self("internal", s.into())
    }
    fn cancelled(s: impl Into<String>) -> Self {
        Self("cancelled", s.into())
    }
    fn interrupted(s: impl Into<String>) -> Self {
        Self("interrupted", s.into())
    }
    fn code(&self) -> &str {
        self.0
    }
    fn message(&self) -> &str {
        &self.1
    }
}
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
                } else {
                    host.start_provider(run, &node.id, source).await?;
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
        assert!(
            !run.input_tokens
                .contains_key(&(node.id.clone(), port.into())),
            "stream delta must not also be queued for final execution"
        );
        run.seen.push((node.id.clone(), value.value.clone()));
        Ok(())
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
            return Err(Error::invalid_request("Unknown fixture state version."));
        }
        Ok(Run {
            scheduler,
            seen: serde_json::from_value(state.value["seen"].clone())
                .map_err(|error| Error::invalid_request(error.to_string()))?,
            calls: serde_json::from_value(state.value["calls"].clone())
                .map_err(|error| Error::invalid_request(error.to_string()))?,
            held: vec![],
            max_usage: Default::default(),
        })
    }
    fn validate_activation(&self, run: &Run) -> Result<(), Error> {
        validate_execution_contract(&run.flow, &run.definitions).map_err(Error::invalid_request)
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
fn fixture(nodes: Vec<Value>, edges: Vec<Value>, mode: &str) -> (Host, Run) {
    let definitions=[
        json!({"class_name":"Source","kind":"source","short_description":"source","long_description":"source","handler_id":"test.node","interfaces":["IFlowNodeActivate"],"output_ports":[{"name":"out","kind":"output","token_type":"text","mode":mode,"phase":"execution"}],"signal_ports":[{"name":"done"}],"action_ports":[{"name":"disable"}]}),
        json!({"class_name":"Sink","kind":"sink","short_description":"sink","long_description":"sink","handler_id":"test.node","input_ports":[{"name":"in","kind":"input","token_type":"text","mode":mode,"phase":"execution"}],"action_ports":[{"name":"go"},{"name":"back"}],"signal_ports":[{"name":"done"}]}),
        json!({"class_name":"Join","kind":"inline","short_description":"join","long_description":"join","handler_id":"test.node","input_ports":[{"name":"in","kind":"input","token_type":"text","mode":mode,"phase":"execution"},{"name":"unused","kind":"input","token_type":"text","mode":"final_value","phase":"execution"}]}),
        json!({"class_name":"Action","kind":"control","short_description":"action","long_description":"action","handler_id":"test.node","action_ports":[{"name":"go"},{"name":"back"}],"signal_ports":[{"name":"done"}]}),
    ].into_iter().map(|v| {let d:FlowNodeDefinition=serde_json::from_value(v).unwrap();(d.class_name.clone(),d)}).collect();
    let sources = nodes
        .iter()
        .filter(|n| n["definition_name"] == "Source")
        .map(|n| n["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let flow:FlowDocument=serde_json::from_value(json!({"version":battersea_flow::document::FLOW_DOCUMENT_VERSION,"flow_key":"test","title":"test","execution":{"source_order":sources,"limits":FlowExecutionLimits::default()},"nodes":nodes,"edges":edges})).unwrap();
    let scheduler = SchedulerState::new("test-run".into(), &flow, definitions).unwrap();
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
    assert!(run(&host, &mut original).await.is_err());
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
    assert!(run(&host, &mut original).await.is_err());
    let journal = Journal::open(store, "test-run").unwrap();
    assert!(matches!(
        journal.require_resumable(),
        Err(RecoveryError::UnresolvedEffects(_))
    ));
    let revision = journal.record().revision;
    let mut restored = host.restore_activation(&journal).unwrap();
    assert!(host
        .resume_activation(&mut restored, journal, revision, CancellationToken::new())
        .await
        .is_err());
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
    let polls = host.polled.load(Ordering::SeqCst);
    tokio::task::yield_now().await;
    assert_eq!(host.polled.load(Ordering::SeqCst), polls);
    assert!(polls < 4);
    assert_eq!(state.retention.usage(), retention::Usage::default());
}
#[tokio::test]
async fn queued_signals_follow_data_and_signal_cycles_report_the_causal_path() {
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
    assert!(error.message().contains("Signal cycle"));
    assert!(error.message().contains("a") && error.message().contains("b"));
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
    assert_eq!(
        run(&host, &mut state).await.unwrap_err().code(),
        "cancelled"
    );
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
        assert!(run(&host, &mut state).await.is_err());
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
    assert!(error.message().contains("Capacity deadlock"));
    assert!(error.message().contains("edge"));
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
                node("source", "Source", json!({"values":["retained"]})),
                node("sink", "Sink", json!({})),
            ],
            vec![edge("output", "source", "sink", 0, "final_value")],
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
    assert!(host.restore_activation(&journal).is_err());
    assert_eq!(
        serde_json::to_vec(store.record.lock().unwrap().as_ref().unwrap()).unwrap(),
        before
    );
    assert!(original.calls.is_empty());
}
