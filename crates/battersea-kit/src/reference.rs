//! Copyright (c) Scott A Dixon
//!
//! An in-memory execution host on a [`ManualClock`]: a kit's own test host and the oracle
//! its conformance suite compares against. It makes no durable commitment and has no
//! checkpoints.
use async_trait::async_trait;
use battersea_flow::{
    execution::validate_execution_contract, registry::Registry, FlowActionPortDefinition, FlowNode,
    FlowNodeDefinition, FlowParameterDefinition, FlowSignalPortDefinition,
};
use battersea_model::ManualClock;
use battersea_runtime::{
    error_codes, ActivationHost, ExecutionError, ExecutionEvent, ExecutionHost, HandlerRegistry,
    RunPhase, SchedulerState,
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex, PoisonError},
};

/// The reference host's own refusal codes, beside the runtime's `error_codes`.
pub mod codes {
    /// A parameter write the datatype registry refuses.
    pub const INVALID_PARAMETER_WRITE: &str = "invalid_parameter_write";
    /// A second acceptance of the same run.
    pub const ALREADY_ACCEPTED: &str = "already_accepted";
    /// A logic node on a host built without a logic evaluator.
    pub const NO_LOGIC_EVALUATOR: &str = "no_logic_evaluator";
}

/// The reference host's error: an outcome class, the refusal reason and a message.
#[derive(Debug, Clone)]
pub struct ReferenceError {
    class: &'static str,
    reason: Option<&'static str>,
    message: String,
}
impl ReferenceError {
    fn new(class: &'static str, reason: Option<&'static str>, message: impl Into<String>) -> Self {
        Self {
            class,
            reason,
            message: message.into(),
        }
    }
}
impl std::fmt::Display for ReferenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.class, self.message)
    }
}
impl std::error::Error for ReferenceError {}
impl ExecutionError for ReferenceError {
    fn invalid_request(reason: &'static str, message: impl Into<String>) -> Self {
        Self::new("invalid_request", Some(reason), message)
    }
    fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", None, message)
    }
    fn cancelled(reason: &'static str, message: impl Into<String>) -> Self {
        Self::new("cancelled", Some(reason), message)
    }
    fn interrupted(reason: &'static str, message: impl Into<String>) -> Self {
        Self::new("interrupted", Some(reason), message)
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

/// The reference host's run state: the scheduler state and the kit's own state `X`.
pub struct ReferenceState<X> {
    scheduler: SchedulerState,
    pub kit: X,
}
impl<X> ReferenceState<X> {
    pub fn new(scheduler: SchedulerState, kit: X) -> Self {
        Self { scheduler, kit }
    }
}
impl<X> Deref for ReferenceState<X> {
    type Target = SchedulerState;
    fn deref(&self) -> &SchedulerState {
        &self.scheduler
    }
}
impl<X> DerefMut for ReferenceState<X> {
    fn deref_mut(&mut self) -> &mut SchedulerState {
        &mut self.scheduler
    }
}

/// Computes a logic node's signal outputs; the generic kit supplies one for its gates.
pub type LogicOutputs = dyn Fn(
        &FlowNode,
        &FlowNodeDefinition,
        &[FlowActionPortDefinition],
        &[FlowSignalPortDefinition],
        &HashSet<String>,
        bool,
    ) -> Result<Vec<String>, String>
    + Send
    + Sync;

/// An in-memory [`ExecutionHost`] whose state carries the kit's `X` and whose clock is a
/// [`ManualClock`]. It records execution events and run phases.
pub struct ReferenceHost<X: Send + Sync + 'static> {
    handlers: HandlerRegistry<Self>,
    clock: Arc<ManualClock>,
    logic: Option<Arc<LogicOutputs>>,
    events: Mutex<Vec<ExecutionEvent>>,
    phases: Mutex<HashMap<String, RunPhase>>,
    _kit: PhantomData<fn() -> X>,
}

impl<X: Send + Sync + 'static> ReferenceHost<X> {
    pub fn new(handlers: HandlerRegistry<Self>, clock: Arc<ManualClock>) -> Self {
        Self {
            handlers,
            clock,
            logic: None,
            events: Mutex::default(),
            phases: Mutex::default(),
            _kit: PhantomData,
        }
    }
    /// Evaluates logic nodes with `logic`; without one the host refuses them.
    pub fn with_logic(
        mut self,
        logic: impl Fn(
                &FlowNode,
                &FlowNodeDefinition,
                &[FlowActionPortDefinition],
                &[FlowSignalPortDefinition],
                &HashSet<String>,
                bool,
            ) -> Result<Vec<String>, String>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.logic = Some(Arc::new(logic));
        self
    }
    pub fn clock(&self) -> &Arc<ManualClock> {
        &self.clock
    }
    /// A run state around `scheduler` carrying `kit`.
    pub fn state(&self, scheduler: SchedulerState, kit: X) -> ReferenceState<X> {
        ReferenceState::new(scheduler, kit)
    }
    /// Every execution event recorded, in order.
    pub fn events(&self) -> Vec<ExecutionEvent> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    /// The execution events recorded for `run_id`, in order.
    pub fn run_events(&self, run_id: &str) -> Vec<ExecutionEvent> {
        self.events()
            .into_iter()
            .filter(|event| event.run_id == run_id)
            .collect()
    }
    /// The last phase retained for `run_id`.
    pub fn phase(&self, run_id: &str) -> Option<RunPhase> {
        self.phases
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(run_id)
            .cloned()
    }
}

#[async_trait]
impl<X: Send + Sync + 'static> ExecutionHost for ReferenceHost<X> {
    type State = ReferenceState<X>;
    type Error = ReferenceError;
    fn controller_activation_effects(
        &self,
        _runtime: &mut Self::State,
        _node: &str,
        _values: &HashMap<String, Value>,
    ) {
    }
    fn handlers(&self) -> &HandlerRegistry<Self> {
        &self.handlers
    }
    async fn execution_event(&self, _runtime: &Self::State, event: ExecutionEvent) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
    async fn resolve_parameter_write(
        &self,
        _runtime: &Self::State,
        _node: &FlowNode,
        parameter: &FlowParameterDefinition,
        value: &Value,
    ) -> Result<Value, ReferenceError> {
        Registry::default()
            .convert_automation_value(parameter, value)
            .map_err(|error| {
                ReferenceError::invalid_request(codes::INVALID_PARAMETER_WRITE, error.to_string())
            })
    }
    async fn commit_parameter_write(
        &self,
        _runtime: &mut Self::State,
        _node: &FlowNode,
        _parameter: &FlowParameterDefinition,
        _value: &Value,
    ) -> Result<(), ReferenceError> {
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
    ) -> Result<Vec<String>, ReferenceError> {
        let Some(logic) = &self.logic else {
            return Err(ReferenceError::invalid_request(
                codes::NO_LOGIC_EVALUATOR,
                "The reference host was built without a logic evaluator.",
            ));
        };
        logic(node, definition, actions, signals, fired, enabled).map_err(|message| {
            ReferenceError::invalid_request(error_codes::UNSUPPORTED_HANDLER_OPERATION, message)
        })
    }
    fn logic_gate_label(&self, _node: &FlowNode, definition: &FlowNodeDefinition) -> String {
        definition.class_name.clone()
    }
}

#[async_trait]
impl<X: Send + Sync + 'static> ActivationHost for ReferenceHost<X> {
    fn validate_activation(&self, runtime: &Self::State) -> Result<(), ReferenceError> {
        validate_execution_contract(&runtime.flow, &runtime.definitions).map_err(|issue| {
            ReferenceError::invalid_request(error_codes::INVALID_EXECUTION_CONTRACT, issue.message)
        })
    }
    async fn accept_activation(
        &self,
        runtime: &Self::State,
        _node: &str,
    ) -> Result<(), ReferenceError> {
        let mut phases = self.phases.lock().unwrap_or_else(PoisonError::into_inner);
        if phases.contains_key(&runtime.run_id) {
            return Err(ReferenceError::invalid_request(
                codes::ALREADY_ACCEPTED,
                format!("Run {:?} was already accepted.", runtime.run_id),
            ));
        }
        phases.insert(runtime.run_id.clone(), RunPhase::Accepted);
        Ok(())
    }
    async fn preflight_activation(&self, _runtime: &mut Self::State) -> Result<(), ReferenceError> {
        Ok(())
    }
    async fn complete_execution(&self, _runtime: &mut Self::State) -> Result<(), ReferenceError> {
        Ok(())
    }
    async fn retain_execution_phase(
        &self,
        runtime: &Self::State,
        phase: RunPhase,
    ) -> Result<(), ReferenceError> {
        self.phases
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(runtime.run_id.clone(), phase);
        Ok(())
    }
}
