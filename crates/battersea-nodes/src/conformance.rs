//! Copyright (c) Scott A Dixon
//!
//! The generic kit's conformance suite: probe nodes that activate and observe a flow, the
//! cases, the reference fixture and [`run`]. An application registers the probes beside the
//! generic handlers in its test host and runs the suite against it.
use async_trait::async_trait;
use battersea_flow::{FlowExecutionLimits, FlowNode, FlowNodeDefinition};
use battersea_kit::conformance::{self, Case, Failure, Fixture, Suite};
use battersea_kit::{ManualClock, ReferenceHost, ReferenceState};
use battersea_runtime::{
    ExecutionEvent, ExecutionHost, NodeHandler, RegistryBuilder, Retained, SchedulerState, Token,
};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};
use tokio_util::sync::CancellationToken;

/// The probe definitions the suite composes after the generic manifest.
pub const PROBE_MANIFEST: &str = include_str!("conformance.yaml");

/// Emits each activation value named after an output port, then `done` when asked.
pub struct ProbeSource;
/// Receives tokens and signals; the runtime's events record them.
pub struct ProbeSink;

#[async_trait]
impl<H: ExecutionHost> NodeHandler<H> for ProbeSource {
    fn handler_id(&self) -> &'static str {
        "battersea.conformance.source"
    }
    async fn execute_node(
        &self,
        host: &H,
        runtime: &mut H::State,
        node: &FlowNode,
        definition: &FlowNodeDefinition,
        values: Option<&HashMap<String, Value>>,
        token: &CancellationToken,
    ) -> Result<(), H::Error> {
        let values = values.cloned().unwrap_or_default();
        for port in &definition.output_ports {
            if let Some(value) = values.get(&port.name) {
                host.emit_flow_token(
                    runtime,
                    &node.id,
                    &port.name,
                    Token {
                        token_type: port.token_type.clone(),
                        value: value.clone(),
                    },
                    token,
                )
                .await?;
            }
        }
        if values.get("done") == Some(&Value::Bool(true)) {
            host.emit_flow_signal(runtime, node, "done").await?;
        }
        Ok(())
    }
}

#[async_trait]
impl<H: ExecutionHost> NodeHandler<H> for ProbeSink {
    fn handler_id(&self) -> &'static str {
        "battersea.conformance.sink"
    }
    async fn execute_sink(
        &self,
        _host: &H,
        _runtime: &mut H::State,
        _node: &FlowNode,
        _port: &str,
        _value: Retained<Token>,
    ) -> Result<(), H::Error> {
        Ok(())
    }
    async fn execute_action(
        &self,
        _host: &H,
        _runtime: &mut H::State,
        _node: &FlowNode,
        _definition: &FlowNodeDefinition,
        _port: &str,
    ) -> Result<(), H::Error> {
        Ok(())
    }
}

/// Install the probe handlers beside the generic handlers.
pub fn register_probe_handlers<H: ExecutionHost>(
    builder: &mut RegistryBuilder<H>,
) -> Result<(), String> {
    builder.register(ProbeSource)?;
    builder.register(ProbeSink)?;
    Ok(())
}

fn flow(key: &str, body: &str) -> String {
    let limits = serde_json::to_string(&FlowExecutionLimits::default()).expect("limits");
    format!(
        r#"flow 1 "{key}" {{
    version = 3;
    title = "{key}";
    execution = {{"source_order": ["source"], "limits": {limits}}};
    node "source" "ConformanceSource" {{"instance_name": "Source"}}
{body}
}}"#
    )
}

fn case(name: &str, body: &str, values: Value) -> Case {
    Case {
        name: name.into(),
        flow: flow(name, body),
        node: "source".into(),
        values: serde_json::from_value(values).expect("activation values"),
        steps: vec![],
        before: vec![],
    }
}

/// The cases: one per generic handler family.
pub fn cases() -> Vec<Case> {
    let fragment = json!({"text": "First paragraph.", "encoding": "markdown"});
    vec![
        case(
            "concatenate",
            r#"    node "join" "Concatenate" {"instance_name": "Join", "parameter_values": {"input_ports": 2}}
    node "sink" "ConformanceSink" {"instance_name": "Sink"}
    edge "first" "source"."fragment" -> "join"."input-0" {"order": 0}
    edge "second" "source"."other" -> "join"."input-1" {"order": 1}
    edge "out" "join"."output" -> "sink"."input" {"order": 0}"#,
            json!({"fragment": fragment, "other": "Second."}),
        ),
        case(
            "multiplexer",
            r#"    node "copy" "Multiplexer" {"instance_name": "Copy", "parameter_values": {"output_ports": 2}}
    node "left" "ConformanceSink" {"instance_name": "Left"}
    node "right" "ConformanceSink" {"instance_name": "Right"}
    edge "in" "source"."raw" -> "copy"."input" {"order": 0}
    edge "l" "copy"."output-0" -> "left"."input" {"order": 0}
    edge "r" "copy"."output-1" -> "right"."input" {"order": 1}"#,
            json!({"raw": "raw text"}),
        ),
        case(
            "and-gate",
            r#"    node "fan" "SignalMultiplexer" {"instance_name": "Fan", "parameter_values": {"output_ports": 2}}
    node "gate" "AndGate" {"instance_name": "Gate", "parameter_values": {"input_ports": 2}}
    node "sink" "ConformanceSink" {"instance_name": "Sink"}
    edge "data" "source"."fragment" -> "sink"."input" {"order": 0}
    edge "s0" "source"."done" ~> "fan"."input" {}
    edge "s1" "fan"."output-0" ~> "gate"."input-0" {}
    edge "s2" "fan"."output-1" ~> "gate"."input-1" {}
    edge "s3" "gate"."output" ~> "sink"."go" {}"#,
            json!({"fragment": fragment, "done": true}),
        ),
        case(
            "or-gate-merge",
            r#"    node "merge" "SignalDemultiplexer" {"instance_name": "Merge", "parameter_values": {"input_ports": 2}}
    node "gate" "OrGate" {"instance_name": "Gate", "parameter_values": {"input_ports": 2}}
    node "sink" "ConformanceSink" {"instance_name": "Sink"}
    edge "data" "source"."fragment" -> "sink"."input" {"order": 0}
    edge "s0" "source"."done" ~> "merge"."input-0" {}
    edge "s1" "merge"."output" ~> "gate"."input-0" {}
    edge "s2" "gate"."output" ~> "sink"."go" {}"#,
            json!({"fragment": fragment, "done": true}),
        ),
        case(
            "invert-gate",
            r#"    node "gate" "InvertGate" {"instance_name": "Gate"}
    node "sink" "ConformanceSink" {"instance_name": "Sink"}
    edge "data" "source"."fragment" -> "sink"."input" {"order": 0}
    edge "s0" "source"."done" ~> "gate"."input" {}
    edge "s1" "gate"."output" ~> "sink"."go" {}"#,
            json!({"fragment": fragment}),
        ),
    ]
}

/// The suite: the generic manifest, the probes and the cases.
pub fn suite() -> Suite {
    Suite {
        kit: &crate::KIT,
        manifests: vec![
            (crate::KIT.name, crate::MANIFEST),
            ("battersea.conformance", PROBE_MANIFEST),
        ],
        register_token_types: crate::register_token_types,
        cases: cases(),
    }
}

/// The reference host with the generic handlers, the probes and the logic evaluator.
pub fn reference_host(clock: Arc<ManualClock>) -> ReferenceHost<()> {
    let mut builder = RegistryBuilder::default();
    crate::register_handlers(&mut builder).expect("generic handlers");
    register_probe_handlers(&mut builder).expect("probe handlers");
    ReferenceHost::new(builder.build(), clock).with_logic(
        |node, definition, actions, signals, fired, enabled| {
            crate::logic::prepare_logic_outputs(node, definition, actions, signals, fired, enabled)
                .map_err(|error| error.to_string())
        },
    )
}

/// The kit's own fixture: [`reference_host`] with no kit state.
pub struct ReferenceFixture;
impl Fixture for ReferenceFixture {
    type Host = ReferenceHost<()>;
    fn host(&self, clock: Arc<ManualClock>) -> Self::Host {
        reference_host(clock)
    }
    fn state(&self, host: &Self::Host, scheduler: SchedulerState) -> ReferenceState<()> {
        host.state(scheduler, ())
    }
    fn events(&self, host: &Self::Host, run_id: &str) -> Vec<ExecutionEvent> {
        host.run_events(run_id)
    }
}

/// Plays every case on the reference host and on the fixture's host and reports the first
/// divergence.
pub async fn run<F: Fixture>(fixture: &F) -> Result<(), Failure> {
    conformance::run(
        &suite(),
        &ReferenceFixture,
        fixture,
        async |_, _, _: &()| Ok(()),
        async |_, _, _: &()| Ok(()),
    )
    .await
}
