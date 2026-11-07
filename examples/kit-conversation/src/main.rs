//! A separately compiled application host for the conversation kit, keeping each transcript
//! in a file. It runs the kit's conformance suite against those files.
use async_trait::async_trait;
use battersea_flow::{registry::Registry, *};
#[cfg(test)]
use battersea_kit::conformance::Fixture;
use battersea_kit::{Clock, ManualClock};
use battersea_kit_conversation::{
    codes,
    compaction::{Policy, PolicyEvaluation},
    conformance::{Probe, ScriptedResponse, Snapshot, TranscriptSnapshot, TRANSCRIPT},
    reference::ScriptedEngine,
    CallRecord, CompactionJob, JobId, JobState, Memory, Port, Reservation, Revision, RunState,
    SpendEstimate, Summary, ToolPlan, TranscriptId, Turn, TurnId,
};
use battersea_model::{EngineAdapter, EngineTokenUsage};
use battersea_runtime::*;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

/// The application's own refusal codes, beside the runtime's `error_codes`.
mod app_codes {
    pub const INVALID_PARAMETER_WRITE: &str = "invalid_parameter_write";
    pub const INVALID_FLOW: &str = "invalid_flow";
    pub const ALREADY_ACCEPTED: &str = "already_accepted";
    pub const STORAGE: &str = "storage";
}
#[derive(Debug)]
struct Error {
    code: &'static str,
    reason: Option<&'static str>,
    message: String,
}
impl Error {
    fn new(code: &'static str, reason: Option<&'static str>, message: impl Into<String>) -> Self {
        Self {
            code,
            reason,
            message: message.into(),
        }
    }
    fn storage(error: impl std::fmt::Display) -> Self {
        Self::invalid_request(app_codes::STORAGE, error.to_string())
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}
impl ExecutionError for Error {
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
        self.code
    }
    fn reason_code(&self) -> Option<&str> {
        self.reason
    }
    fn message(&self) -> &str {
        &self.message
    }
}

/// The application's run state: the scheduler's and the kit's.
struct Run {
    scheduler: SchedulerState,
    kit: RunState<Value>,
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

/// A transcript as its file holds it.
#[derive(Default)]
struct Transcript {
    turns: Vec<Turn<Value>>,
    summary: Option<Summary<Value>>,
}

/// A host whose transcripts are files under `dir`, one per transcript, named by the
/// transcript. Jobs, the spend ledger and the engine's script stay in memory.
struct FileHost {
    dir: PathBuf,
    clock: Arc<ManualClock>,
    handlers: HandlerRegistry<Self>,
    phases: Mutex<HashMap<String, RunPhase>>,
    events: Mutex<Vec<ExecutionEvent>>,
    engine: Arc<ScriptedEngine>,
    jobs: Mutex<BTreeMap<JobId, CompactionJob<Value>>>,
    ledger: Mutex<Vec<(Reservation, EngineTokenUsage)>>,
    policy: Policy,
    next: AtomicU64,
}
static HOSTS: AtomicU64 = AtomicU64::new(0);
impl FileHost {
    /// A host over a fresh directory under the system's temporary directory.
    fn new(clock: Arc<ManualClock>) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "kit-conversation-{}-{}",
            std::process::id(),
            HOSTS.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        Self {
            dir,
            engine: Arc::new(ScriptedEngine::new(clock.clone())),
            clock,
            handlers: RegistryBuilder::default().build(),
            phases: Mutex::default(),
            events: Mutex::default(),
            jobs: Mutex::default(),
            ledger: Mutex::default(),
            policy: Policy::default(),
            next: AtomicU64::new(0),
        }
    }
    fn next(&self, kind: &str) -> String {
        format!("{kind}-{}", self.next.fetch_add(1, Ordering::SeqCst) + 1)
    }
    /// Keeps generated ids clear of a seeded `id` of the generated form.
    fn reserve(&self, id: &str) {
        if let Some(number) = id
            .rsplit_once('-')
            .and_then(|(_, number)| number.parse::<u64>().ok())
        {
            self.next.fetch_max(number, Ordering::SeqCst);
        }
    }
    fn path(&self, transcript: &TranscriptId) -> PathBuf {
        self.dir.join(format!("{transcript}.json"))
    }
    fn read(&self, transcript: &TranscriptId) -> Result<Transcript, Error> {
        let path = self.path(transcript);
        if !path.is_file() {
            return Err(Error::invalid_request(
                codes::UNKNOWN_TRANSCRIPT,
                format!("Transcript {transcript} has no file."),
            ));
        }
        let text = std::fs::read_to_string(path).map_err(Error::storage)?;
        let value: Value = serde_json::from_str(&text).map_err(Error::storage)?;
        Ok(Transcript {
            turns: serde_json::from_value(value["turns"].clone()).map_err(Error::storage)?,
            summary: serde_json::from_value(value["summary"].clone()).map_err(Error::storage)?,
        })
    }
    fn write(&self, transcript: &TranscriptId, held: &Transcript) -> Result<(), Error> {
        let value = json!({"turns": held.turns, "summary": held.summary});
        std::fs::write(
            self.path(transcript),
            serde_json::to_vec_pretty(&value).map_err(Error::storage)?,
        )
        .map_err(Error::storage)
    }
    /// Every transcript file, by name.
    fn transcripts(&self) -> Result<BTreeMap<TranscriptId, Transcript>, Error> {
        let mut held = BTreeMap::new();
        for entry in std::fs::read_dir(&self.dir).map_err(Error::storage)? {
            let path = entry.map_err(Error::storage)?.path();
            if let Some(name) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".json"))
            {
                let id = TranscriptId::from(name);
                held.insert(id.clone(), self.read(&id)?);
            }
        }
        Ok(held)
    }
    fn registry() -> Registry {
        let mut registry = Registry::default();
        battersea_kit_conversation::conformance::register_for_suite(&mut registry).unwrap();
        registry
    }
}
impl Drop for FileHost {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[async_trait]
impl ExecutionHost for FileHost {
    type State = Run;
    type Error = Error;
    fn controller_activation_effects(
        &self,
        _run: &mut Run,
        _node: &str,
        _values: &HashMap<String, Value>,
    ) {
    }
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
        Self::registry()
            .convert_automation_value(parameter, value)
            .map_err(|e| Error::invalid_request(app_codes::INVALID_PARAMETER_WRITE, e.to_string()))
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
        Err(Error::invalid_request(
            error_codes::UNSUPPORTED_HANDLER_OPERATION,
            "This host has no logic nodes.",
        ))
    }
    fn logic_gate_label(&self, _node: &FlowNode, definition: &FlowNodeDefinition) -> String {
        definition.class_name.clone()
    }
}

#[async_trait]
impl ActivationHost for FileHost {
    fn validate_activation(&self, run: &Run) -> Result<(), Error> {
        let mut registry = Self::registry();
        for id in self.handlers.ids() {
            registry.register_handler(id).unwrap();
        }
        let validation =
            battersea_flow::validation::validate_document(&run.flow, &run.definitions, &registry);
        if validation.valid {
            Ok(())
        } else {
            Err(Error::invalid_request(
                app_codes::INVALID_FLOW,
                format!("{:?}", validation.issues),
            ))
        }
    }
    async fn accept_activation(&self, run: &Run, _node: &str) -> Result<(), Error> {
        let mut phases = self.phases.lock().unwrap();
        if phases.contains_key(&run.run_id) {
            return Err(Error::invalid_request(
                app_codes::ALREADY_ACCEPTED,
                "already accepted",
            ));
        }
        phases.insert(run.run_id.clone(), RunPhase::Accepted);
        Ok(())
    }
    async fn preflight_activation(&self, _run: &mut Run) -> Result<(), Error> {
        Ok(())
    }
    /// Commits the run's staged turns to the transcript's file.
    async fn complete_execution(&self, run: &mut Run) -> Result<(), Error> {
        let staged = std::mem::take(&mut run.kit.staged);
        if staged.is_empty() {
            return Ok(());
        }
        let transcript = run.kit.transcript.clone();
        let mut held = match self.read(&transcript) {
            Ok(held) => held,
            Err(error) if error.reason == Some(codes::UNKNOWN_TRANSCRIPT) => Transcript::default(),
            Err(error) => return Err(error),
        };
        held.turns.extend(staged);
        self.write(&transcript, &held)
    }
    async fn retain_execution_phase(&self, run: &Run, phase: RunPhase) -> Result<(), Error> {
        self.phases
            .lock()
            .unwrap()
            .insert(run.run_id.clone(), phase);
        Ok(())
    }
}

#[async_trait]
impl Port for FileHost {
    type Extension = Value;
    fn run_state(run: &mut Run) -> &mut RunState<Value> {
        &mut run.kit
    }
    async fn turns(
        &self,
        _run: &Run,
        transcript: &TranscriptId,
    ) -> Result<Vec<Turn<Value>>, Error> {
        Ok(self.read(transcript)?.turns)
    }
    async fn stage(&self, run: &mut Run, mut turn: Turn<Value>) -> Result<TurnId, Error> {
        let id = TurnId::from(self.next("turn"));
        turn.id = Some(id.clone());
        turn.created_at = self.clock.unix_ms();
        run.kit.staged.push(turn);
        Ok(id)
    }
    async fn revise(&self, run: &mut Run, revision: Revision<Value>) -> Result<(), Error> {
        let transcript = run.kit.transcript.clone();
        let mut held = self.read(&transcript)?;
        if let Some(turn) = held
            .turns
            .iter_mut()
            .find(|turn| turn.id.as_ref() == Some(&revision.turn))
        {
            turn.content = revision.content;
            turn.revision += 1;
            return self.write(&transcript, &held);
        }
        Err(Error::invalid_request(
            codes::UNKNOWN_TURN,
            format!(
                "Transcript {transcript} does not hold turn {}.",
                revision.turn
            ),
        ))
    }
    async fn memory(&self, _run: &Run, transcript: &TranscriptId) -> Result<Memory<Value>, Error> {
        let held = self.read(transcript)?;
        let after = held
            .summary
            .as_ref()
            .and_then(|summary| {
                held.turns
                    .iter()
                    .position(|turn| turn.id.as_ref() == Some(&summary.through))
            })
            .map_or(0, |position| position + 1);
        Ok(Memory {
            transcript: transcript.clone(),
            summary: held.summary,
            turns: held.turns[after..].to_vec(),
        })
    }
    async fn write_summary(&self, run: &mut Run, summary: Summary<Value>) -> Result<(), Error> {
        let transcript = run.kit.transcript.clone();
        let mut held = self.read(&transcript)?;
        held.summary = Some(summary);
        self.write(&transcript, &held)
    }
    fn engine(&self, _run: &Run, _node: &str) -> Result<Arc<dyn EngineAdapter>, Error> {
        Ok(self.engine.clone())
    }
    fn tool_plan(&self, _run: &Run, _node: &str) -> Result<ToolPlan, Error> {
        Ok(ToolPlan::default())
    }
    async fn reserve(
        &self,
        run: &mut Run,
        node: &str,
        estimate: SpendEstimate,
    ) -> Result<Reservation, Error> {
        let reservation = Reservation {
            id: self.next("reservation"),
            node: node.into(),
            estimate,
        };
        run.kit.reservations.push(reservation.clone());
        Ok(reservation)
    }
    async fn settle(
        &self,
        run: &mut Run,
        reservation: Reservation,
        usage: EngineTokenUsage,
    ) -> Result<(), Error> {
        let open = &mut run.kit.reservations;
        let Some(position) = open.iter().position(|held| held.id == reservation.id) else {
            return Err(Error::internal(format!(
                "Reservation {} is not open.",
                reservation.id
            )));
        };
        open.remove(position);
        self.ledger.lock().unwrap().push((reservation, usage));
        Ok(())
    }
    async fn record_call(&self, run: &mut Run, record: CallRecord) -> Result<(), Error> {
        run.kit.calls.push(record);
        Ok(())
    }
    fn policy(&self, _run: &Run, transcript: &TranscriptId) -> Result<Policy, Error> {
        self.read(transcript)?;
        Ok(self.policy.clone())
    }
    async fn request_compaction(
        &self,
        _run: &mut Run,
        evaluation: PolicyEvaluation,
    ) -> Result<JobId, Error> {
        let id = JobId::from(self.next("job"));
        self.jobs.lock().unwrap().insert(
            id.clone(),
            CompactionJob {
                id: id.clone(),
                transcript: evaluation.transcript.clone(),
                evaluation,
                state: JobState::Requested,
                extension: Value::Null,
            },
        );
        Ok(id)
    }
    async fn transition(&self, _run: &mut Run, job: &JobId, to: JobState) -> Result<(), Error> {
        match self.jobs.lock().unwrap().get_mut(job) {
            Some(held) => {
                held.state = to;
                Ok(())
            }
            None => Err(Error::invalid_request(
                codes::UNKNOWN_JOB,
                format!("Job {job} is unknown."),
            )),
        }
    }
    async fn deliver(&self, run: &mut Run, text: String) -> Result<(), Error> {
        run.kit.deliveries.push(text);
        Ok(())
    }
}

#[async_trait]
impl Probe for FileHost {
    fn extension(&self) -> Value {
        json!({"host": "files"})
    }
    async fn seed(
        &self,
        _run: &mut Run,
        transcript: TranscriptId,
        turns: Vec<Turn<Value>>,
        summary: Option<Summary<Value>>,
    ) -> Result<(), Error> {
        for id in turns.iter().filter_map(|turn| turn.id.as_ref()) {
            self.reserve(id.as_str());
        }
        if let Some(job) = summary.as_ref().and_then(|summary| summary.job.as_ref()) {
            self.reserve(job.as_str());
        }
        let turns = turns
            .into_iter()
            .map(|mut turn| {
                if turn.id.is_none() {
                    turn.id = Some(TurnId::from(self.next("turn")));
                }
                turn
            })
            .collect();
        self.write(&transcript, &Transcript { turns, summary })
    }
    async fn script(&self, _run: &mut Run, responses: Vec<ScriptedResponse>) -> Result<(), Error> {
        self.engine.extend(responses);
        Ok(())
    }
    async fn snapshot(&self, run: &Run) -> Result<Snapshot<Value>, Error> {
        Ok(Snapshot {
            transcripts: self
                .transcripts()?
                .into_iter()
                .map(|(id, held)| {
                    (
                        id,
                        TranscriptSnapshot {
                            turns: held.turns,
                            summary: held.summary,
                        },
                    )
                })
                .collect(),
            staged: run.kit.staged.clone(),
            jobs: self.jobs.lock().unwrap().values().cloned().collect(),
            calls: run.kit.calls.clone(),
            reservations: run.kit.reservations.clone(),
            deliveries: run.kit.deliveries.clone(),
        })
    }
}

/// The suite's view of this host.
#[cfg(test)]
struct FileFixture;
#[cfg(test)]
impl Fixture for FileFixture {
    type Host = FileHost;
    fn host(&self, clock: Arc<ManualClock>) -> FileHost {
        FileHost::new(clock)
    }
    fn state(&self, _host: &FileHost, scheduler: SchedulerState) -> Run {
        Run {
            scheduler,
            kit: RunState::new(TRANSCRIPT.into()),
        }
    }
    fn events(&self, host: &FileHost, run_id: &str) -> Vec<ExecutionEvent> {
        host.events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.run_id == run_id)
            .cloned()
            .collect()
    }
}

/// A run state around the kit's catalogue and a one-node flow, for the standalone run.
fn run(host: &FileHost, id: &str) -> Run {
    let mut registry = FileHost::registry();
    for definition in battersea_kit_conversation::KIT
        .definitions(&FileHost::registry())
        .unwrap()
    {
        registry.register_handler(&definition.handler_id).unwrap();
    }
    let catalog = battersea_flow::catalog::Catalog::from_manifest(
        battersea_kit_conversation::MANIFEST,
        registry,
    )
    .unwrap();
    let limits = serde_json::to_string(&FlowExecutionLimits::default()).unwrap();
    let flow = battersea_flow::dsl::parse_flow_dsl(&format!(
        r#"
flow 1 "{id}" {{
    version = 3;
    title = "Conversation";
    execution = {{"source_order":["prompt"],"limits":{limits}}};
    node "prompt" "ConversationStaticPrompt" {{"instance_name":"Prompt","parameter_values":{{"text":"hello"}}}}
}}
"#
    ))
    .unwrap();
    assert!(catalog.validate(&flow).valid);
    let _ = host;
    Run {
        scheduler: SchedulerState::new(id.into(), &flow, catalog.definitions().clone()).unwrap(),
        kit: RunState::new(TRANSCRIPT.into()),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let clock = Arc::new(ManualClock::new(1_700_000_000_000));
    let host = FileHost::new(clock);
    let mut state = run(&host, "standalone");
    host.seed(&mut state, TRANSCRIPT.into(), vec![], None)
        .await?;
    host.stage(
        &mut state,
        Turn::text(
            battersea_model::Role::User,
            "hello",
            json!({"host": "files"}),
        ),
    )
    .await?;
    host.complete_execution(&mut state).await?;
    let turns = host.turns(&state, &TRANSCRIPT.into()).await?;
    println!(
        "{} holds {} turn(s)",
        host.path(&TRANSCRIPT.into()).display(),
        turns.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_conversation_kit_suite_passes_on_this_host() {
        battersea_kit_conversation::conformance::run(&FileFixture)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn staged_turns_reach_the_file_on_completion_and_not_before() {
        let clock = Arc::new(ManualClock::new(1_700_000_000_000));
        let host = FileHost::new(clock);
        let mut state = run(&host, "files");
        host.seed(&mut state, TRANSCRIPT.into(), vec![], None)
            .await
            .unwrap();
        host.stage(
            &mut state,
            Turn::text(battersea_model::Role::User, "hello", json!({})),
        )
        .await
        .unwrap();
        assert!(host
            .turns(&state, &TRANSCRIPT.into())
            .await
            .unwrap()
            .is_empty());
        host.complete_execution(&mut state).await.unwrap();
        let turns = host.turns(&state, &TRANSCRIPT.into()).await.unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].created_at, Some(1_700_000_000_000));
        assert!(host.turns(&state, &"other".into()).await.is_err());
        assert!(host.path(&TRANSCRIPT.into()).is_file());
        let dir = host.dir.clone();
        drop(host);
        assert!(!dir.exists());
    }
}
