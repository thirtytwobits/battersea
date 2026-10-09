//! A separately compiled application implementing nodes through the public SDK.
use async_trait::async_trait;
use battersea_flow::{catalog::Catalog, registry::Registry, *};
use battersea_runtime::*;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    ops::{Deref, DerefMut},
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
struct Error {
    code: &'static str,
    message: String,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}
impl ExecutionError for Error {
    fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_request",
            message: message.into(),
        }
    }
    fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "internal",
            message: message.into(),
        }
    }
    fn cancelled(message: impl Into<String>) -> Self {
        Self {
            code: "cancelled",
            message: message.into(),
        }
    }
    fn interrupted(message: impl Into<String>) -> Self {
        Self {
            code: "interrupted",
            message: message.into(),
        }
    }
    fn code(&self) -> &str {
        self.code
    }
    fn message(&self) -> &str {
        &self.message
    }
}
struct Run {
    scheduler: SchedulerState,
    output: Vec<Value>,
    order: Vec<&'static str>,
}
impl Deref for Run {
    type Target = SchedulerState;
    fn deref(&self) -> &Self::Target {
        &self.scheduler
    }
}
impl DerefMut for Run {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.scheduler
    }
}
struct Application {
    handlers: HandlerRegistry<Self>,
    phases: Mutex<HashMap<String, RunPhase>>,
    events: Mutex<Vec<ExecutionEvent>>,
    fail_accept: bool,
    fail_completion: bool,
}
struct Source;
struct Sink;
#[async_trait]
impl NodeHandler<Application> for Source {
    fn handler_id(&self) -> &'static str {
        "example.source"
    }
    async fn execute_node(
        &self,
        host: &Application,
        run: &mut Run,
        node: &FlowNode,
        _definition: &FlowNodeDefinition,
        values: Option<&HashMap<String, Value>>,
        token: &CancellationToken,
    ) -> Result<(), Error> {
        run.order.push("source");
        let value = values
            .and_then(|v| v.get("text"))
            .cloned()
            .unwrap_or(Value::Null);
        if value == json!("panic") {
            panic!("fixture unwinding panic");
        }
        host.emit_flow_token(
            run,
            &node.id,
            "text",
            Token {
                token_type: if value == json!("wrong-type") {
                    "invalid".into()
                } else {
                    "prompt.fragment".into()
                },
                value,
            },
            token,
        )
        .await
    }
}
#[async_trait]
impl NodeHandler<Application> for Sink {
    fn handler_id(&self) -> &'static str {
        "example.sink"
    }
    async fn execute_sink(
        &self,
        _host: &Application,
        run: &mut Run,
        _node: &FlowNode,
        _port: &str,
        token: Token,
    ) -> Result<(), Error> {
        run.order.push("sink");
        run.output.push(token.value);
        Ok(())
    }
}
#[async_trait]
impl ExecutionHost for Application {
    type State = Run;
    type Error = Error;
    fn handlers(&self) -> &HandlerRegistry<Self> {
        &self.handlers
    }
    async fn execution_event(&self, _run: &Run, event: ExecutionEvent) {
        self.events.lock().unwrap().push(event);
    }
    async fn resolve_parameter_write(
        &self,
        _run: &Run,
        _node: &FlowNode,
        parameter: &FlowParameterDefinition,
        value: &Value,
    ) -> Result<Value, Error> {
        Registry::default()
            .convert_automation_value(parameter, value)
            .map_err(|e| Error::invalid_request(e.to_string()))
    }
    async fn commit_parameter_write(
        &self,
        _run: &mut Run,
        _node: &FlowNode,
        _parameter: &FlowParameterDefinition,
        _value: &Value,
    ) -> Result<(), Error> {
        Ok(())
    }
    fn prepare_logic_outputs(
        &self,
        _node: &FlowNode,
        _definition: &FlowNodeDefinition,
        _actions: &[FlowActionPortDefinition],
        _signals: &[FlowSignalPortDefinition],
        _fired: &HashSet<String>,
        _enabled: bool,
    ) -> Result<Vec<String>, Error> {
        Err(Error::invalid_request("No logic nodes registered"))
    }
    fn logic_gate_label(&self, _node: &FlowNode, definition: &FlowNodeDefinition) -> String {
        definition.class_name.clone()
    }
}
#[async_trait]
impl ActivationHost for Application {
    fn validate_activation(&self, run: &Run) -> Result<(), Error> {
        let mut registry = Registry::default();
        battersea_nodes::register_token_types(&mut registry).unwrap();
        for id in self.handlers.ids() {
            registry.register_handler(id).unwrap();
        }

        let validation =
            battersea_flow::validation::validate_document(&run.flow, &run.definitions, &registry);
        if validation.valid {
            Ok(())
        } else {
            Err(Error::invalid_request(format!("{:?}", validation.issues)))
        }
    }

    async fn accept_activation(&self, run: &Run, _node: &str) -> Result<(), Error> {
        if self.fail_accept {
            return Err(Error::internal("storage unavailable"));
        }
        let mut phases = self.phases.lock().unwrap();
        if phases.contains_key(&run.run_id) {
            return Err(Error::invalid_request("already accepted"));
        }
        phases.insert(run.run_id.clone(), RunPhase::Accepted);
        Ok(())
    }
    async fn preflight_activation(&self, run: &mut Run) -> Result<(), Error> {
        run.order.push("preflight");
        Ok(())
    }
    fn controller_activation_effects(
        &self,
        run: &mut Run,
        _node: &str,
        _values: &HashMap<String, Value>,
    ) {
        run.order.push("controller");
    }
    async fn complete_execution(&self, run: &mut Run) -> Result<(), Error> {
        run.order.push("complete");
        Ok(())
    }
    async fn retain_execution_phase(&self, run: &Run, phase: RunPhase) -> Result<(), Error> {
        if self.fail_completion {
            return Err(Error::internal("completion storage unavailable"));
        }
        self.phases
            .lock()
            .unwrap()
            .insert(run.run_id.clone(), phase);
        Ok(())
    }
}
fn application() -> Application {
    let mut builder = RegistryBuilder::default();
    builder.register(Source).unwrap();
    builder.register(Sink).unwrap();
    battersea_nodes::register_handlers(&mut builder).unwrap();
    Application {
        handlers: builder.build(),
        phases: Mutex::default(),
        events: Mutex::default(),
        fail_accept: false,
        fail_completion: false,
    }
}
fn run(id: &str) -> Run {
    let mut registry = Registry::default();
    battersea_nodes::register_token_types(&mut registry).unwrap();
    registry.register_handler("example.source").unwrap();
    registry.register_handler("example.sink").unwrap();
    for id in application().handlers.ids() {
        if id != "example.source" && id != "example.sink" {
            registry.register_handler(id).unwrap();
        }
    }

    let catalog = Catalog::from_manifests(&[("generic", battersea_nodes::MANIFEST), ("application", &json!({"node_definitions":[
        {"class_name":"Source","short_description":"Source","long_description":"Source","handler_id":"example.source","kind":"source","output_ports":[{"name":"text","kind":"output","token_type":"prompt.fragment"}]},
        {"class_name":"Sink","short_description":"Sink","long_description":"Sink","handler_id":"example.sink","kind":"sink","input_ports":[{"name":"text","kind":"input","token_type":"prompt.fragment"}]}
    ]}).to_string())], registry).unwrap();
    let flow = battersea_flow::document::load_document(&json!({"version":1,"flow_key":"example","title":"Example","nodes":[{"id":"source","definition_name":"Source","instance_name":"Source"},{"id":"sink","definition_name":"Sink","instance_name":"Sink"}],"edges":[{"id":"delivery","source_node_id":"source","source_port":"text","target_node_id":"sink","target_port":"text","kind":"token","order":0}]}).to_string()).unwrap();
    let catalog = application().handlers.bind_catalog(catalog).unwrap();
    assert!(catalog.validate(&flow).valid);
    Run {
        scheduler: SchedulerState::new(id.into(), &flow, catalog.definitions().clone()).unwrap(),
        output: vec![],
        order: vec![],
    }
}
fn fanout_run(id: &str) -> Run {
    let mut state = run(id);
    let mut flow = serde_json::to_value(&state.flow).unwrap();
    flow["nodes"].as_array_mut().unwrap().push(json!({"id":"copy", "definition_name":"Multiplexer", "instance_name":"Copy", "parameter_values":{"output_ports":2}}));
    flow["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"second", "definition_name":"Sink", "instance_name":"Second"}));
    flow["edges"] = json!([
        {"id":"input", "source_node_id":"source", "source_port":"text", "target_node_id":"copy", "target_port":"input", "kind":"token", "order":0},
        {"id":"first", "source_node_id":"copy", "source_port":"output-0", "target_node_id":"sink", "target_port":"text", "kind":"token", "order":0},
        {"id":"second", "source_node_id":"copy", "source_port":"output-1", "target_node_id":"second", "target_port":"text", "kind":"token", "order":1}
    ]);
    let flow = battersea_flow::document::load_document(&flow.to_string()).unwrap();
    state.scheduler = SchedulerState::new(id.into(), &flow, state.definitions.clone()).unwrap();
    state
}
async fn demonstrate_generic_fanout() -> Result<(), Error> {
    let app = application();
    let mut state = fanout_run("generic-fanout");
    let payload = json!("Application-owned content");
    app.run_activation(
        &mut state,
        "source",
        HashMap::from([("text".into(), payload.clone())]),
        CancellationToken::new(),
    )
    .await?;
    assert_eq!(state.output, vec![payload.clone(), payload]);
    Ok(())
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let app = application();
    let mut run = run("standalone");
    let payload = json!("Application-owned content");
    app.run_activation(
        &mut run,
        "source",
        HashMap::from([("text".into(), payload.clone())]),
        CancellationToken::new(),
    )
    .await?;
    assert_eq!(run.output, vec![payload]);
    demonstrate_generic_fanout().await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn generic_multiplexer_preserves_each_routed_payload() {
        demonstrate_generic_fanout().await.unwrap();
    }
    #[tokio::test]
    async fn a_separately_compiled_node_runs_without_a_product_session() {
        let app = application();
        let mut run = run("custom-node");
        let payload = json!("authored value");
        app.run_activation(
            &mut run,
            "source",
            HashMap::from([("text".into(), payload.clone())]),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(run.output, vec![payload]);
        assert_eq!(
            run.order,
            ["preflight", "source", "sink", "controller", "complete"]
        );
        assert!(matches!(
            app.phases.lock().unwrap().get(&run.run_id),
            Some(RunPhase::CompletionPending)
        ));
        let events = app.events.lock().unwrap();
        assert!(events
            .iter()
            .all(|e| e.run_id == run.run_id && e.flow_key == run.flow.flow_key));
        assert!(events.windows(2).all(|w| w[1].sequence > w[0].sequence));
        assert!(matches!(
            events.last().unwrap().kind,
            EventKind::ActivationExecutionFinished
        ));
    }
    #[test]
    fn duplicate_registration_cannot_replace_a_handler() {
        let mut builder = RegistryBuilder::<Application>::default();
        builder.register(Source).unwrap();
        assert!(builder.register(Source).is_err());
        assert!(builder.build().get("example.source").is_ok());
    }
    #[tokio::test]
    async fn missing_handlers_fail_before_acceptance() {
        let mut app = application();
        app.handlers = RegistryBuilder::default().build();
        let mut run = run("missing");
        assert!(app
            .run_activation(&mut run, "source", HashMap::new(), CancellationToken::new())
            .await
            .is_err());
        assert!(app.phases.lock().unwrap().is_empty());
        assert!(run.order.is_empty());
    }
    #[tokio::test]
    async fn failures_cancellation_and_panics_keep_the_accepted_identity() {
        for input in ["wrong-type", "panic", "cancel"] {
            let app = application();
            let mut run = run(input);
            let token = CancellationToken::new();
            if input == "cancel" {
                token.cancel();
            }
            assert!(app
                .run_activation(
                    &mut run,
                    "source",
                    HashMap::from([("text".into(), json!(input))]),
                    token
                )
                .await
                .is_err());
            let phases = app.phases.lock().unwrap();
            let Some(RunPhase::Terminal { outcome }) = phases.get(&run.run_id) else {
                panic!("terminal outcome required")
            };
            assert!(matches!(
                (input, outcome),
                ("wrong-type", Outcome::Failed { .. })
                    | ("panic", Outcome::Interrupted { .. })
                    | ("cancel", Outcome::Cancelled { .. })
            ));
            assert!(run.output.is_empty());
        }
    }
    #[tokio::test]
    async fn storage_failures_never_report_durable_success_or_repeat_execution() {
        let mut app = application();
        let mut state = run("storage");
        app.fail_accept = true;
        assert!(app
            .run_activation(
                &mut state,
                "source",
                HashMap::new(),
                CancellationToken::new()
            )
            .await
            .is_err());
        assert!(state.order.is_empty());
        app.fail_accept = false;
        app.fail_completion = true;
        assert!(app
            .run_activation(
                &mut state,
                "source",
                HashMap::new(),
                CancellationToken::new()
            )
            .await
            .is_err());
        assert!(matches!(
            app.phases.lock().unwrap().get(&state.run_id),
            Some(RunPhase::Accepted)
        ));
        let calls = state.output.len();
        assert!(app
            .run_activation(
                &mut state,
                "source",
                HashMap::new(),
                CancellationToken::new()
            )
            .await
            .is_err());
        assert_eq!(state.output.len(), calls);
    }
}
