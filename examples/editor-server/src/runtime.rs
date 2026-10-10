//! Application-owned execution, storage and custom nodes.
use async_trait::async_trait;
use battersea_flow::{catalog::Catalog, registry::Registry, *};
use battersea_runtime::*;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub struct Error {
    pub code: String,
    pub message: String,
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
            code: "invalid_request".into(),
            message: message.into(),
        }
    }
    fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "internal".into(),
            message: message.into(),
        }
    }
    fn cancelled(message: impl Into<String>) -> Self {
        Self {
            code: "cancelled".into(),
            message: message.into(),
        }
    }
    fn interrupted(message: impl Into<String>) -> Self {
        Self {
            code: "interrupted".into(),
            message: message.into(),
        }
    }
    fn code(&self) -> &str {
        &self.code
    }
    fn message(&self) -> &str {
        &self.message
    }
}
pub struct Run {
    scheduler: SchedulerState,
    pub output: Vec<Retained<Value>>,
    pub tap: Mutex<battersea_pianola::PianolaTap>,
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
pub struct Application {
    pub handlers: HandlerRegistry<Self>,
    pub catalog: Catalog,
    pub root: PathBuf,
    pub view: Mutex<battersea_telemetry::view::View>,
    pub observation_error: Mutex<Option<String>>,
    exporter: Option<battersea_telemetry::otlp::Exporter>,
    pub acceptance: Mutex<HashMap<String, tokio::sync::oneshot::Sender<Result<(), String>>>>,
}
struct TextSource;
struct Output;
#[async_trait]
impl NodeHandler<Application> for TextSource {
    fn handler_id(&self) -> &'static str {
        "example.text"
    }
    async fn execute_node(
        &self,
        host: &Application,
        run: &mut Run,
        node: &FlowNode,
        _: &FlowNodeDefinition,
        values: Option<&HashMap<String, Value>>,
        _token: &CancellationToken,
    ) -> Result<(), Error> {
        let text = values
            .and_then(|v| v.get("text"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::invalid_request("text is required"))?;
        let delay = node
            .parameter_values
            .get("delay_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let text = text.to_uppercase();
        host.start_provider(
            run,
            &node.id,
            futures_util::stream::once(async move {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                ProviderEvent::Token {
                    port: "text".into(),
                    value: Token {
                        token_type: "prompt.fragment".into(),
                        value: json!(text),
                    },
                }
            }),
        )
        .await
    }
}
#[async_trait]
impl NodeHandler<Application> for Output {
    fn handler_id(&self) -> &'static str {
        "example.output"
    }
    async fn execute_sink(
        &self,
        _: &Application,
        run: &mut Run,
        _: &FlowNode,
        _: &str,
        token: Retained<Token>,
    ) -> Result<(), Error> {
        run.output.push(
            run.retention
                .retain("output", token.value.clone())
                .map_err(Error::invalid_request)?,
        );
        Ok(())
    }
}
impl Application {
    pub fn new(root: PathBuf) -> Result<Self, Error> {
        let mut builder = RegistryBuilder::default();
        builder.register(TextSource).map_err(Error::internal)?;
        builder.register(Output).map_err(Error::internal)?;
        battersea_nodes::register_handlers(&mut builder).map_err(Error::internal)?;
        let handlers = builder.build();
        let mut registry = Registry::default();
        battersea_nodes::register_token_types(&mut registry).map_err(Error::internal)?;
        for id in handlers.ids() {
            registry
                .register_handler(id)
                .map_err(|e| Error::internal(e.to_string()))?;
        }
        let manifest = std::fs::read_to_string(root.join("nodes.json"))
            .map_err(|e| Error::internal(e.to_string()))?;
        let catalog = Catalog::from_manifests(
            &[
                ("generic", battersea_nodes::MANIFEST),
                ("application", &manifest),
            ],
            registry,
        )
        .map_err(|e| Error::internal(e.to_string()))?;
        handlers
            .validate(catalog.definitions().values())
            .map_err(Error::internal)?;
        Ok(Self {
            handlers,
            catalog,
            root,
            view: Mutex::new(
                battersea_telemetry::view::View::new(
                    uuid::Uuid::new_v4().to_string(),
                    battersea_telemetry::view::Retention {
                        max_records: 4096,
                        max_record_bytes: 8 * 1024 * 1024,
                        max_delta_bytes: 8 * 1024 * 1024,
                        max_deltas: 8192,
                        max_age_ms: 3_600_000,
                    },
                )
                .map_err(|e| Error::internal(e.to_string()))?,
            ),
            observation_error: Mutex::default(),
            exporter: std::env::var("BATTERSEA_OTLP_ENDPOINT")
                .ok()
                .map(|endpoint| {
                    battersea_telemetry::otlp::Exporter::new(
                        &endpoint,
                        std::time::Duration::from_secs(5),
                        1024 * 1024,
                    )
                    .map_err(|e| Error::invalid_request(e.to_string()))
                })
                .transpose()?,
            acceptance: Mutex::default(),
        })
    }
    pub async fn publish_retained_outcome(&self, run: &Run) -> Result<(), Error> {
        let bytes = std::fs::read(self.root.join("runs").join(format!("{}.json", run.run_id)))
            .map_err(|e| Error::internal(e.to_string()))?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|e| Error::internal(e.to_string()))?;
        let phase: RunPhase = serde_json::from_value(value["phase"].clone())
            .map_err(|e| Error::internal(e.to_string()))?;
        let event = ExecutionEvent {
            attempt: 0,
            run_id: run.run_id.clone(),
            flow_key: run.flow.flow_key.clone(),
            node_id: String::new(),
            sequence: 0,
            kind: EventKind::ActivationExecutionFinished,
            summary: String::new(),
            detail: Some(serde_json::to_value(phase).map_err(|e| Error::internal(e.to_string()))?),
        };
        self.execution_event(run, event).await;
        {
            use battersea_telemetry::view::{active, State, Status};
            let mut view = self.view.lock().unwrap();
            let status = view
                .get(&battersea_telemetry::otlp::activation_resource_id(
                    &run.run_id,
                ))
                .and_then(|r| match &r.state {
                    State::Activation { status } => Some(status.clone()),
                    _ => None,
                });
            if let Some(status) = status.filter(|status| !active(status)) {
                let children = view
                    .records()
                    .filter(|r| {
                        r.context.activation_id == run.run_id
                            && matches!(&r.state, State::Node { status, .. } if active(status))
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                for mut child in children {
                    if let State::Node {
                        status: child_status,
                        ..
                    } = &mut child.state
                    {
                        *child_status = if status == Status::Succeeded {
                            status.clone()
                        } else {
                            Status::Interrupted
                        };
                    }
                    if let Err(error) = view.record(child, |_| Ok(())) {
                        *self.observation_error.lock().unwrap() = Some(error.to_string());
                    }
                }
            }
        }
        if let Some(exporter) = &self.exporter {
            let (records, in_flight) = {
                let view = self.view.lock().unwrap();
                (
                    view.records()
                        .filter(|record| record.context.activation_id == run.run_id)
                        .cloned()
                        .collect::<Vec<_>>(),
                    view.in_flight_requests() as u64,
                )
            };
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            match battersea_telemetry::otlp::Batch::from_records(
                "battersea-example",
                &records,
                in_flight,
                now,
            ) {
                Ok(batch) => {
                    if let Err(error) = exporter.export(&batch).await {
                        *self.observation_error.lock().unwrap() = Some(error.to_string());
                    }
                }
                Err(error) => *self.observation_error.lock().unwrap() = Some(error.to_string()),
            }
        }
        Ok(())
    }
    pub fn run(&self, id: String, flow: &FlowDocument) -> Result<Run, Error> {
        Ok(Run {
            scheduler: SchedulerState::new(id, flow, self.catalog.definitions().clone())
                .map_err(Error::invalid_request)?,
            output: vec![],
            tap: Mutex::new(battersea_pianola::PianolaTap::new(
                vec![battersea_pianola::PianolaTapTarget {
                    flow_key: flow.flow_key.clone(),
                    node_id: None,
                    port: "text".into(),
                }],
                16 * 1024 * 1024,
            )),
        })
    }
    pub fn write_record(&self, id: &str, value: &Value, create: bool) -> Result<(), Error> {
        use std::io::Write;
        let path = self.root.join("runs").join(format!("{id}.json"));
        if create {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|e| Error::internal(e.to_string()))?;
            file.write_all(value.to_string().as_bytes())
                .and_then(|_| file.sync_all())
                .map_err(|e| Error::internal(e.to_string()))
        } else {
            let temp = path.with_extension("tmp");
            let mut file =
                std::fs::File::create(&temp).map_err(|e| Error::internal(e.to_string()))?;
            file.write_all(value.to_string().as_bytes())
                .and_then(|_| file.sync_all())
                .and_then(|_| std::fs::rename(temp, path))
                .map_err(|e| Error::internal(e.to_string()))
        }
    }
}
#[async_trait]
impl ExecutionHost for Application {
    type State = Run;
    type Error = Error;
    fn controller_activation_effects(&self, _: &mut Run, _: &str, _: &HashMap<String, Value>) {}
    fn handlers(&self) -> &HandlerRegistry<Self> {
        &self.handlers
    }
    async fn execution_event(&self, _: &Run, event: ExecutionEvent) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("wall clock")
            .as_millis() as u64;
        if let Err(error) =
            self.view
                .lock()
                .unwrap()
                .observe_execution(&event, None, now, None, |_| Ok(()))
        {
            *self.observation_error.lock().unwrap() = Some(error.to_string());
        }
    }
    fn observe_token(&self, run: &Run, node: &str, port: &str, token: &Token) {
        run.tap.lock().unwrap().capture(
            &run.flow.flow_key,
            node,
            port,
            &token.token_type,
            &token.value,
        );
    }
    async fn resolve_parameter_write(
        &self,
        _: &Run,
        _: &FlowNode,
        parameter: &FlowParameterDefinition,
        value: &Value,
    ) -> Result<Value, Error> {
        Registry::default()
            .convert_automation_value(parameter, value)
            .map_err(|e| Error::invalid_request(e.to_string()))
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
        node: &FlowNode,
        definition: &FlowNodeDefinition,
        actions: &[FlowActionPortDefinition],
        signals: &[FlowSignalPortDefinition],
        fired: &HashSet<String>,
        enabled: bool,
    ) -> Result<Vec<String>, Error> {
        battersea_nodes::logic::prepare_logic_outputs(
            node, definition, actions, signals, fired, enabled,
        )
        .map_err(|error| Error::invalid_request(error.to_string()))
    }
    fn logic_gate_label(&self, _: &FlowNode, definition: &FlowNodeDefinition) -> String {
        definition.class_name.clone()
    }
}
#[async_trait]
impl ActivationHost for Application {
    fn validate_activation(&self, run: &Run) -> Result<(), Error> {
        let result = self.catalog.validate(&run.flow);
        if result.valid {
            Ok(())
        } else {
            Err(Error::invalid_request(format!("{:?}", result.issues)))
        }
    }
    async fn accept_activation(&self, run: &Run, _: &str) -> Result<(), Error> {
        self.write_record(
            &run.run_id,
            &json!({"id":run.run_id,"flow":run.flow,"phase":"accepted"}),
            true,
        )?;
        if let Some(sender) = self.acceptance.lock().unwrap().remove(&run.run_id) {
            let _ = sender.send(Ok(()));
        }
        Ok(())
    }
    async fn preflight_activation(&self, _: &mut Run) -> Result<(), Error> {
        Ok(())
    }
    async fn complete_execution(&self, _: &mut Run) -> Result<(), Error> {
        Ok(())
    }
    async fn retain_execution_phase(&self, run: &Run, phase: RunPhase) -> Result<(), Error> {
        let phase = match phase {
            RunPhase::CompletionPending => RunPhase::Terminal {
                outcome: Outcome::Succeeded,
            },
            other => other,
        };
        let tap = run.tap.lock().unwrap();
        if let Some(error) = tap.error() {
            return Err(Error::invalid_request(error));
        }
        let capture: Vec<_> = tap
            .emissions_from(0)
            .into_iter()
            .map(|e| json!({"node_id":e.node_id,"port":e.port,"value":e.value,"ordinal":e.ordinal}))
            .collect();
        self.write_record(&run.run_id, &json!({"id":run.run_id,"flow":run.flow,"phase":phase,"output":run.output,"capture":capture}), false)?;
        Ok(())
    }
}
