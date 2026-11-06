//! Distinguishing cases from contracts/kits.md: names, model-facing text, the reference
//! host and the conformance harness.
use async_trait::async_trait;
use battersea_flow::{registry::Registry, FlowNode, FlowNodeDefinition};
use battersea_kit::conformance::{self, Case, Divergence, Fixture, Suite, EPOCH_MS};
use battersea_kit::{Clock, Kit, KitError, ManualClock, ReferenceHost, ReferenceState};
use battersea_runtime::{
    EventKind, ExecutionError, ExecutionEvent, ExecutionHost, NodeHandler, RegistryBuilder,
    Retained, RunPhase, SchedulerState, Token,
};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};
use tokio_util::sync::CancellationToken;

const MANIFEST: &str = r#"
formatter_blocks:
  greeting:
    bindings: [name]
    template:
      kind: text
      text: "Hello {name}"
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
fn model_facing_text_changes_with_a_formatter_block_and_keeps_inline_variants() {
    let text = KIT.model_facing_text().unwrap();
    assert_eq!(
        text.keys().collect::<Vec<_>>(),
        ["ExampleSource.text.shout", "formatter_blocks.greeting"]
    );
    let changed = kit(leak(MANIFEST.replace("Hello {name}", "Good day {name}")))
        .model_facing_text()
        .unwrap();
    assert_ne!(
        text["formatter_blocks.greeting"],
        changed["formatter_blocks.greeting"]
    );
    assert_eq!(
        text["ExampleSource.text.shout"],
        changed["ExampleSource.text.shout"]
    );
}

struct Source;
struct Sink {
    fail_on: Option<&'static str>,
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
            return Err(H::Error::internal(format!("refused {:?}", value.value)));
        }
        Ok(())
    }
}

fn register_token_types(registry: &mut Registry) -> Result<(), String> {
    registry
        .register_token_type("example.text", json!({"type": "string"}))
        .map_err(|error| error.to_string())
}

struct Example {
    fail_on: Option<&'static str>,
}
impl Fixture for Example {
    type Host = ReferenceHost<Vec<String>>;
    fn host(&self, clock: Arc<ManualClock>) -> Self::Host {
        let mut builder = RegistryBuilder::default();
        builder.register(Source).unwrap();
        builder
            .register(Sink {
                fail_on: self.fail_on,
            })
            .unwrap();
        ReferenceHost::new(builder.build(), clock)
    }
    fn state(&self, host: &Self::Host, scheduler: SchedulerState) -> ReferenceState<Vec<String>> {
        host.state(scheduler, vec![])
    }
    fn events(&self, host: &Self::Host, run_id: &str) -> Vec<ExecutionEvent> {
        host.run_events(run_id)
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

fn suite() -> Suite {
    Suite {
        kit: &KIT,
        manifests: vec![("example", MANIFEST)],
        register_token_types,
        cases: vec![case("first", "one"), case("second", "two")],
    }
}

#[tokio::test]
async fn the_reference_host_runs_a_flow_on_its_manual_clock() {
    let fixture = Example { fail_on: None };
    let played = conformance::play(
        &suite(),
        &fixture,
        &case("alone", "value"),
        async |_, _, _: &()| Ok(()),
    )
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
    let mut registry = Registry::default();
    register_token_types(&mut registry).unwrap();
    for id in host.handlers().ids() {
        registry.register_handler(id).unwrap();
    }
    let catalog = battersea_flow::catalog::Catalog::from_manifest(MANIFEST, registry).unwrap();
    let catalog = host.handlers().bind_catalog(catalog).unwrap();
    let flow = battersea_flow::dsl::parse_flow_dsl(&flow("direct")).unwrap();
    let scheduler =
        SchedulerState::new("direct".into(), &flow, catalog.definitions().clone()).unwrap();
    let mut state = host.state(scheduler, vec!["kit state".into()]);
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
    let reference = Example { fail_on: None };
    let prepare =
        async |_: &ReferenceHost<Vec<String>>, _: &mut ReferenceState<Vec<String>>, _: &()| Ok(());
    conformance::run(
        &suite,
        &reference,
        &Example { fail_on: None },
        prepare,
        prepare,
    )
    .await
    .unwrap();
    let failure = conformance::run(
        &suite,
        &reference,
        &Example {
            fail_on: Some("second"),
        },
        prepare,
        prepare,
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
