//! Distinguishing cases from contracts/kits.md: names, model-facing text, the reference
//! host and the conformance harness.
use async_trait::async_trait;
use battersea_flow::{
    registry::{ParameterType, Registry},
    FlowNode, FlowNodeDefinition, FlowParameterDataType, FlowParameterDefinition,
    FlowParameterEditor, FlowParameterEditorKind,
};
use battersea_kit::conformance::{self, Case, Divergence, Fixture, PortAccess, Suite, EPOCH_MS};
use battersea_kit::{Clock, Kit, KitError, ManualClock, ReferenceHost, ReferenceState};
use battersea_runtime::{
    EventKind, ExecutionError, ExecutionEvent, ExecutionHost, NodeHandler, RegistryBuilder,
    Retained, RunPhase, SchedulerState, Token,
};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

const MANIFEST: &str = r#"
formatter_blocks:
  greeting:
    bindings: [name]
    template:
      kind: text
      text: "Hello {name}"
  farewell:
    bindings: [name]
    template:
      kind: text
      text: "Goodbye {name}"
node_definitions:
  - class_name: ExampleSource
    handler_id: example.source
    short_description: Source
    long_description: Source
    kind: source
    interfaces: [IFlowNodeActivate]
    output_ports:
      - name: text
        kind: output
        token_type: example.text
        mode: final_value
        phase: execution
        formatter:
          kind: prompt_template
          default:
            ref: greeting
          variants:
            shout:
              template:
                kind: text
                text: "HELLO"
  - class_name: ExampleSink
    handler_id: example.sink
    short_description: Sink
    long_description: Sink
    kind: sink
    input_ports:
      - name: text
        kind: input
        token_type: example.text
        mode: final_value
        phase: execution
"#;
const TOKEN_TYPES: &[&str] = &["example.text"];
const KIT: Kit = Kit {
    name: "example",
    manifest: MANIFEST,
    token_types: TOKEN_TYPES,
};

fn kit(manifest: &'static str) -> Kit {
    Kit { manifest, ..KIT }
}
fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

#[test]
fn check_refuses_each_name_outside_the_namespace() {
    KIT.check().unwrap();
    assert!(matches!(
        Kit {
            name: "Example",
            ..KIT
        }
        .check(),
        Err(KitError::Name(_))
    ));
    assert!(matches!(
        Kit {
            token_types: &["other.text"],
            ..KIT
        }
        .check(),
        Err(KitError::RegisteredTokenType { .. })
    ));
    assert!(matches!(
        kit(leak(MANIFEST.replace("example.sink", "other.sink"))).check(),
        Err(KitError::HandlerId { .. })
    ));
    assert!(matches!(
        kit(leak(MANIFEST.replace("ExampleSink", "Sink"))).check(),
        Err(KitError::ClassName { .. })
    ));
    assert!(matches!(
        kit(leak(MANIFEST.replacen(
            "token_type: example.text",
            "token_type: other.text",
            1
        )))
        .check(),
        Err(KitError::ReferencedTokenType { .. })
    ));
    assert!(matches!(
        kit(leak(MANIFEST.replacen(
            "token_type: example.text",
            "token_type: example.other",
            1
        )))
        .check(),
        Err(KitError::UnregisteredTokenType { .. })
    ));
}

#[test]
fn check_accepts_the_shared_vocabulary_only_from_the_generic_kit() {
    let shared = leak(
        MANIFEST
            .replace("example.text", "prompt.fragment")
            .replace("ExampleS", "S"),
    );
    let generic = Kit {
        name: battersea_kit::GENERIC_KIT,
        manifest: leak(shared.replace("example.", "battersea.")),
        token_types: battersea_kit::SHARED_TOKEN_TYPES,
    };
    generic.check().unwrap();
    let other = Kit {
        token_types: &["prompt.fragment"],
        ..kit(shared)
    };
    assert!(matches!(
        other.check(),
        Err(KitError::RegisteredTokenType { .. })
    ));
    let referencing = Kit {
        token_types: &[],
        manifest: leak(
            shared
                .replace("Source\n", "ExampleSource\n")
                .replace("Sink\n", "ExampleSink\n"),
        ),
        ..KIT
    };
    referencing.check().unwrap();
}

#[test]
fn model_facing_text_changes_with_a_block_a_reference_or_an_inline_template() {
    let text = KIT.model_facing_text().unwrap();
    assert_eq!(
        text.keys().collect::<Vec<_>>(),
        [
            "ExampleSource.text.default",
            "ExampleSource.text.variants.shout",
            "formatter_blocks.farewell",
            "formatter_blocks.greeting"
        ]
    );
    assert_eq!(
        text["ExampleSource.text.default"],
        text["formatter_blocks.greeting"]
    );
    let block = kit(leak(MANIFEST.replace("Hello {name}", "Good day {name}")))
        .model_facing_text()
        .unwrap();
    assert_ne!(
        text["formatter_blocks.greeting"],
        block["formatter_blocks.greeting"]
    );
    assert_ne!(
        text["ExampleSource.text.default"],
        block["ExampleSource.text.default"]
    );
    assert_eq!(
        text["ExampleSource.text.variants.shout"],
        block["ExampleSource.text.variants.shout"]
    );
    let reference = kit(leak(MANIFEST.replace("ref: greeting", "ref: farewell")))
        .model_facing_text()
        .unwrap();
    assert_eq!(
        reference["ExampleSource.text.default"],
        text["formatter_blocks.farewell"]
    );
    assert_eq!(
        reference["formatter_blocks.greeting"],
        text["formatter_blocks.greeting"]
    );
    let inline = kit(leak(MANIFEST.replace("HELLO", "HEY")))
        .model_facing_text()
        .unwrap();
    assert_ne!(
        text["ExampleSource.text.variants.shout"],
        inline["ExampleSource.text.variants.shout"]
    );
    assert!(matches!(
        kit(leak(MANIFEST.replace("ref: greeting", "ref: missing"))).model_facing_text(),
        Err(KitError::Manifest { .. })
    ));
}

#[test]
fn model_facing_text_keeps_the_default_template_apart_from_a_variant_named_default() {
    let manifest = leak(MANIFEST.replace("shout:", "default:"));
    kit(manifest).check().unwrap();
    let text = kit(manifest).model_facing_text().unwrap();
    assert_eq!(
        text["ExampleSource.text.default"],
        text["formatter_blocks.greeting"]
    );
    assert_ne!(
        text["ExampleSource.text.variants.default"],
        text["ExampleSource.text.default"]
    );
    let changed = kit(leak(manifest.replace("Hello {name}", "Good day {name}")))
        .model_facing_text()
        .unwrap();
    assert_ne!(
        changed["ExampleSource.text.default"],
        text["ExampleSource.text.default"]
    );
    assert_eq!(
        changed["ExampleSource.text.variants.default"],
        text["ExampleSource.text.variants.default"]
    );
}

struct Source;
struct Sink {
    fail_on: Option<&'static str>,
    message: &'static str,
}
/// A sink that waits on the host's clock before accepting each token, within `timeout` when
/// one is set.
struct SleepySink {
    wait: Duration,
    timeout: Option<Duration>,
}
#[async_trait]
impl<H: ExecutionHost> NodeHandler<H> for Source {
    fn handler_id(&self) -> &'static str {
        "example.source"
    }
    async fn execute_node(
        &self,
        host: &H,
        runtime: &mut H::State,
        node: &FlowNode,
        _definition: &FlowNodeDefinition,
        values: Option<&HashMap<String, Value>>,
        token: &CancellationToken,
    ) -> Result<(), H::Error> {
        let value = values
            .and_then(|values| values.get("text"))
            .cloned()
            .unwrap_or(Value::Null);
        host.emit_flow_token(
            runtime,
            &node.id,
            "text",
            Token {
                token_type: "example.text".into(),
                value,
            },
            token,
        )
        .await
    }
}
#[async_trait]
impl<H: ExecutionHost> NodeHandler<H> for Sink {
    fn handler_id(&self) -> &'static str {
        "example.sink"
    }
    async fn execute_sink(
        &self,
        _host: &H,
        runtime: &mut H::State,
        _node: &FlowNode,
        _port: &str,
        value: Retained<Token>,
    ) -> Result<(), H::Error> {
        if self.fail_on.is_some_and(|run| runtime.run_id == run) {
            return Err(H::Error::internal(format!(
                "{} {:?}",
                self.message, value.value
            )));
        }
        Ok(())
    }
}
#[async_trait]
impl NodeHandler<ReferenceHost<Vec<String>>> for SleepySink {
    fn handler_id(&self) -> &'static str {
        "example.sink"
    }
    async fn execute_sink(
        &self,
        host: &ReferenceHost<Vec<String>>,
        _runtime: &mut ReferenceState<Vec<String>>,
        _node: &FlowNode,
        _port: &str,
        _value: Retained<Token>,
    ) -> Result<(), battersea_kit::ReferenceError> {
        let clock = host.clock().as_ref();
        let work = battersea_kit::clock::sleep(clock, self.wait);
        match self.timeout {
            Some(limit) => battersea_kit::clock::timeout(clock, limit, work)
                .await
                .map_err(|_| battersea_kit::ReferenceError::internal("the work timed out")),
            None => {
                work.await;
                Ok(())
            }
        }
    }
}

fn register_token_types(registry: &mut Registry) -> Result<(), String> {
    registry
        .register_token_type("example.text", json!({"type": "string"}))
        .map_err(|error| error.to_string())
}

#[derive(Default)]
struct Example {
    fail_on: Option<&'static str>,
    message: &'static str,
    /// A sink that waits this long on the clock replaces the plain sink.
    wait: Option<Duration>,
    /// The sleepy sink's wait is bounded by this timeout.
    timeout: Option<Duration>,
    /// The kit state every run starts with.
    state: Vec<String>,
}
impl Fixture for Example {
    type Host = ReferenceHost<Vec<String>>;
    fn host(&self, clock: Arc<ManualClock>) -> Self::Host {
        let mut builder = RegistryBuilder::default();
        builder.register(Source).unwrap();
        match self.wait {
            Some(wait) => builder
                .register(SleepySink {
                    wait,
                    timeout: self.timeout,
                })
                .unwrap(),
            None => builder
                .register(Sink {
                    fail_on: self.fail_on,
                    message: self.message,
                })
                .unwrap(),
        }
        ReferenceHost::new(builder.build(), clock)
    }
    fn state(&self, host: &Self::Host, scheduler: SchedulerState) -> ReferenceState<Vec<String>> {
        host.state(scheduler, self.state.clone())
    }
    fn events(&self, host: &Self::Host, run_id: &str) -> Vec<ExecutionEvent> {
        host.run_events(run_id)
    }
}
/// The example kit's port access: no operations; the kit state is the observable state.
struct Access;
impl PortAccess<ReferenceHost<Vec<String>>, ()> for Access {
    async fn prepare(
        &self,
        _host: &ReferenceHost<Vec<String>>,
        _state: &mut ReferenceState<Vec<String>>,
        _operation: &(),
    ) -> Result<(), String> {
        Ok(())
    }
    async fn observe(
        &self,
        _host: &ReferenceHost<Vec<String>>,
        state: &ReferenceState<Vec<String>>,
    ) -> Result<Value, String> {
        Ok(json!(state.kit))
    }
}

fn flow(key: &str) -> String {
    let limits = serde_json::to_string(&battersea_flow::FlowExecutionLimits::default()).unwrap();
    format!(
        r#"flow 1 "{key}" {{
    version = 3;
    title = "{key}";
    execution = {{"source_order": ["source"], "limits": {limits}}};
    node "source" "ExampleSource" {{"instance_name": "Source"}}
    node "sink" "ExampleSink" {{"instance_name": "Sink"}}
    edge "delivery" "source"."text" -> "sink"."text" {{"order": 0}}
}}"#
    )
}

fn case(name: &str, text: &str) -> Case {
    Case {
        name: name.into(),
        flow: flow(name),
        node: "source".into(),
        values: HashMap::from([("text".to_string(), json!(text))]),
        steps: vec![],
        before: vec![],
    }
}

fn suite_of(cases: Vec<Case>) -> Suite {
    Suite {
        kit: &KIT,
        manifests: vec![("example", MANIFEST)],
        register_token_types,
        cases,
    }
}
fn suite() -> Suite {
    suite_of(vec![case("first", "one"), case("second", "two")])
}

/// The scheduler state of the example flow `name` on `host`.
fn scheduler(host: &ReferenceHost<Vec<String>>, name: &str) -> SchedulerState {
    let mut registry = Registry::default();
    register_token_types(&mut registry).unwrap();
    for id in host.handlers().ids() {
        registry.register_handler(id).unwrap();
    }
    let catalog = battersea_flow::catalog::Catalog::from_manifest(MANIFEST, registry).unwrap();
    let catalog = host.handlers().bind_catalog(catalog).unwrap();
    let flow = battersea_flow::dsl::parse_flow_dsl(&flow(name)).unwrap();
    SchedulerState::new(name.into(), &flow, catalog.definitions().clone()).unwrap()
}

#[tokio::test]
async fn the_reference_host_converts_automation_writes_with_the_kits_registry() {
    let mut registry = Registry::default();
    registry
        .register_parameter_type(
            "example.shout",
            ParameterType::new(json!({"type": "string"}), None, |_| Ok(()))
                .with_automation_conversion(|value| {
                    Ok(json!(value.as_str().unwrap_or_default().to_uppercase()))
                }),
        )
        .unwrap();
    let clock = Arc::new(ManualClock::new(EPOCH_MS));
    let bare = Example::default().host(clock.clone());
    let kit = Example::default().host(clock).with_registry(registry);
    let parameter = FlowParameterDefinition {
        name: "volume".into(),
        datatype: FlowParameterDataType::named("example.shout"),
        editor: FlowParameterEditor {
            kind: FlowParameterEditorKind::String,
            min: None,
            max: None,
            values: vec![],
            source: None,
            default_value: None,
        },
        controller: None,
        short_description: None,
        long_description: None,
    };
    let state = kit.state(scheduler(&kit, "writes"), vec![]);
    let node = state.flow.nodes[0].clone();
    assert_eq!(
        kit.resolve_parameter_write(&state, &node, &parameter, &json!("loud"))
            .await
            .unwrap(),
        json!("LOUD")
    );
    let refused = bare
        .resolve_parameter_write(&state, &node, &parameter, &json!("loud"))
        .await
        .unwrap_err();
    assert_eq!(
        refused.reason_code(),
        Some(battersea_kit::reference::codes::INVALID_PARAMETER_WRITE)
    );
    assert!(refused.message().contains("Unknown datatype"), "{refused}");
}

#[tokio::test]
async fn the_reference_host_runs_a_flow_on_its_manual_clock() {
    let fixture = Example::default();
    let played = conformance::play(&suite(), &fixture, &case("alone", "value"), &Access)
        .await
        .unwrap();
    assert_eq!(played.outcome, conformance::Outcome::Completed);
    let kinds = played
        .events
        .iter()
        .map(|event| event["kind"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds.first().map(String::as_str),
        Some("activation_accepted")
    );
    assert_eq!(
        kinds.last().map(String::as_str),
        Some("activation_execution_finished")
    );
    assert!(played
        .events
        .iter()
        .any(|event| event["kind"] == "token_receive" && event["detail"]["value"] == "value"));
    let clock = Arc::new(ManualClock::new(EPOCH_MS));
    let host = fixture.host(clock.clone());
    let mut state = host.state(scheduler(&host, "direct"), vec!["kit state".into()]);
    battersea_runtime::ActivationHost::run_activation(
        &host,
        &mut state,
        "source",
        HashMap::from([("text".to_string(), json!("direct"))]),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(state.kit, ["kit state"]);
    assert!(matches!(
        host.phase("direct"),
        Some(RunPhase::CompletionPending)
    ));
    assert!(matches!(
        host.run_events("direct").last().map(|event| event.kind),
        Some(EventKind::ActivationExecutionFinished)
    ));
    assert_eq!(clock.unix_ms(), Some(EPOCH_MS));
    assert!(host.checkpoint_host_state(&state).is_err());
}

#[tokio::test]
async fn the_harness_passes_an_equal_host_and_names_the_first_diverging_case() {
    let suite = suite();
    let reference = Example::default();
    conformance::run(&suite, &reference, &Example::default(), &Access)
        .await
        .unwrap();
    let failure = conformance::run(
        &suite,
        &reference,
        &Example {
            fail_on: Some("second"),
            message: "refused",
            ..Example::default()
        },
        &Access,
    )
    .await
    .unwrap_err();
    assert_eq!(failure.case, "second");
    assert!(
        matches!(&*failure.divergence, Divergence::Outcome { candidate: conformance::Outcome::Failed { code, .. }, .. } if code == "internal"),
        "{failure}"
    );
    assert!(failure.to_string().starts_with("case \"second\""));
}

#[tokio::test]
async fn the_harness_compares_the_ports_observable_state() {
    let failure = conformance::run(
        &suite(),
        &Example::default(),
        &Example {
            state: vec!["a write the port acknowledged but lost".into()],
            ..Example::default()
        },
        &Access,
    )
    .await
    .unwrap_err();
    assert_eq!(failure.case, "first");
    assert!(
        matches!(&*failure.divergence, Divergence::State { reference, .. } if *reference == json!([])),
        "{failure}"
    );
}

#[tokio::test]
async fn the_harness_ignores_the_hosts_own_error_wording() {
    let suite = suite_of(vec![case("refused", "value")]);
    let failing = |message| Example {
        fail_on: Some("refused"),
        message,
        ..Example::default()
    };
    conformance::run(
        &suite,
        &failing("the reference says"),
        &failing("the candidate says"),
        &Access,
    )
    .await
    .unwrap();
    let played = conformance::play(&suite, &failing("wording"), &suite.cases[0], &Access)
        .await
        .unwrap();
    let error = played
        .events
        .iter()
        .find(|event| event["kind"] == "node_error")
        .expect("a failure record");
    assert!(error["summary"].as_str().unwrap().starts_with("wording"));
    let stripped = conformance::comparable(error);
    assert_eq!(stripped["summary"], "");
    assert_eq!(stripped["detail"]["error"]["message"], "");
    assert_eq!(
        stripped["detail"]["error"]["code"],
        error["detail"]["error"]["code"]
    );
    let finished = played.events.last().unwrap();
    assert_eq!(finished["kind"], "activation_execution_finished");
    assert_eq!(
        conformance::comparable(finished)["detail"]["outcome"]["message"],
        ""
    );
}

#[tokio::test]
async fn a_step_completes_a_wait_that_races_a_longer_timeout() {
    let (work, limit) = (Duration::from_secs(1), Duration::from_secs(5));
    let racing = |wait| Example {
        wait: Some(wait),
        timeout: Some(limit),
        ..Example::default()
    };
    let stepped = suite_of(vec![Case {
        steps: vec![work],
        ..case("raced", "value")
    }]);
    let played = conformance::play(&stepped, &racing(work), &stepped.cases[0], &Access)
        .await
        .unwrap();
    assert_eq!(played.outcome, conformance::Outcome::Completed);
    assert_eq!(played.unused_steps, 0);
    conformance::run(&stepped, &racing(work), &racing(work), &Access)
        .await
        .unwrap();
    let late = suite_of(vec![Case {
        steps: vec![limit],
        ..case("late", "value")
    }]);
    let played = conformance::play(&late, &racing(limit * 2), &late.cases[0], &Access)
        .await
        .unwrap();
    assert!(
        matches!(&played.outcome, conformance::Outcome::Failed { code, .. } if code == "internal"),
        "{:?}",
        played.outcome
    );
    assert_eq!(played.unused_steps, 0);
}

#[tokio::test]
async fn a_wait_the_case_does_not_step_fails_on_the_clock() {
    let second = Duration::from_secs(1);
    let stepped = suite_of(vec![Case {
        steps: vec![second],
        ..case("timed", "value")
    }]);
    let waits = |wait| Example {
        wait: Some(wait),
        ..Example::default()
    };
    conformance::run(&stepped, &waits(second), &waits(second), &Access)
        .await
        .unwrap();
    let failure = conformance::run(&stepped, &waits(second), &waits(second * 2), &Access)
        .await
        .unwrap_err();
    assert!(
        matches!(*failure.divergence, Divergence::WaitBeyondSteps { side: conformance::Side::Candidate, remaining } if remaining == second),
        "{failure}"
    );
    let unstepped = suite_of(vec![case("untimed", "value")]);
    let failure = conformance::run(&unstepped, &waits(second), &waits(second), &Access)
        .await
        .unwrap_err();
    assert!(
        matches!(
            *failure.divergence,
            Divergence::WaitBeyondSteps {
                side: conformance::Side::Reference,
                ..
            }
        ),
        "{failure}"
    );
    let failure = conformance::run(&stepped, &Example::default(), &Example::default(), &Access)
        .await
        .unwrap_err();
    assert!(
        matches!(
            *failure.divergence,
            Divergence::UnusedClockSteps {
                side: conformance::Side::Reference,
                remaining: 1
            }
        ),
        "{failure}"
    );
}
