//! Copyright (c) Scott A Dixon
//!
//! The port on the kit's reference host: transcripts in memory, a scripted engine and a
//! spend ledger, on the host's manual clock. A run's staged turns commit when its activation
//! completes, without durability.
use crate::{
    codes,
    compaction::{Policy, PolicyEvaluation},
    CallRecord, CompactionJob, JobId, JobState, Memory, Port, Reservation, Revision, RunState,
    SpendEstimate, Summary, ToolPlan, TranscriptId, Turn, TurnId,
};
use async_trait::async_trait;
use battersea_kit::{ManualClock, ReferenceError, ReferenceHost, ReferenceState};
use battersea_model::{
    utc_now, Clock, EngineAdapter, EngineAdapterDebugContext, EngineAdapterRequestError,
    EngineLocalToolCall, EngineTextStream, EngineTextStreamEvent, EngineTextStreamRequest,
    EngineTokenUsage, ErrorKind, StopReason, ToolCallEvidence, ToolCallOutcome,
};
use battersea_runtime::{ExecutionError, HandlerRegistry};
use futures_util::stream;
use serde_json::Value;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

/// The reference host's extension: any JSON.
pub type Extension = Value;

/// A transcript the reference host holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Transcript {
    pub turns: Vec<Turn<Extension>>,
    pub summary: Option<Summary<Extension>>,
}

/// What the reference host holds across runs.
pub struct Store {
    pub transcripts: BTreeMap<TranscriptId, Transcript>,
    pub jobs: BTreeMap<JobId, CompactionJob<Extension>>,
    /// Settled reservations with the usage that settled them.
    pub ledger: Vec<(Reservation, EngineTokenUsage)>,
    /// Tokens the host still lets runs reserve; `None` is unbounded.
    pub budget: Option<u64>,
    pub policy: Policy,
    pub tools: ToolPlan,
    pub engine: Arc<ScriptedEngine>,
    next: u64,
}
impl Store {
    fn next(&mut self) -> u64 {
        self.next += 1;
        self.next
    }
    /// Keeps generated ids clear of a seeded `id` of the generated form.
    fn reserve(&mut self, id: &str) {
        if let Some(number) = numbered(id) {
            self.next = self.next.max(number);
        }
    }
}

/// The number of an id of the generated form `KIND-N`.
fn numbered(id: &str) -> Option<u64> {
    id.rsplit_once('-')
        .and_then(|(_, number)| number.parse().ok())
}

/// The kit's state on the reference host: a store shared by every run on it and the run's
/// own [`RunState`].
pub struct State {
    store: Arc<Mutex<Store>>,
    pub run: RunState<Extension>,
}
impl State {
    /// A run on `transcript` with a store of its own, whose engine reads `clock`.
    pub fn new(transcript: impl Into<TranscriptId>, clock: Arc<dyn Clock>) -> Self {
        Self {
            store: Arc::new(Mutex::new(Store {
                transcripts: BTreeMap::new(),
                jobs: BTreeMap::new(),
                ledger: vec![],
                budget: None,
                policy: Policy::default(),
                tools: ToolPlan::default(),
                engine: Arc::new(ScriptedEngine::new(clock)),
                next: 0,
            })),
            run: RunState::new(transcript.into()),
        }
    }
    /// Another run on `transcript`, sharing this state's store.
    pub fn another_run(&self, transcript: impl Into<TranscriptId>) -> Self {
        Self {
            store: self.store.clone(),
            run: RunState::new(transcript.into()),
        }
    }
    /// The store, locked.
    pub fn store(&self) -> MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// The scripted engine every call on this store reaches.
    pub fn engine(&self) -> Arc<ScriptedEngine> {
        self.store().engine.clone()
    }
}

/// Commits the run's staged turns to its transcript: the reference host's completion hook.
pub fn commit(state: &mut ReferenceState<State>) -> Result<(), String> {
    let staged = std::mem::take(&mut state.kit.run.staged);
    let transcript = state.kit.run.transcript.clone();
    state
        .kit
        .store()
        .transcripts
        .entry(transcript)
        .or_default()
        .turns
        .extend(staged);
    Ok(())
}

/// A reference host whose port is this module's, committing staged turns on completion.
pub fn host(
    handlers: HandlerRegistry<ReferenceHost<State>>,
    clock: Arc<ManualClock>,
) -> ReferenceHost<State> {
    ReferenceHost::new(handlers, clock).with_completion(commit)
}

fn refuse(reason: &'static str, message: String) -> ReferenceError {
    ReferenceError::invalid_request(reason, message)
}

#[async_trait]
impl Port for ReferenceHost<State> {
    type Extension = Extension;
    fn run_state(state: &mut Self::State) -> &mut RunState<Extension> {
        &mut state.kit.run
    }
    async fn turns(
        &self,
        state: &Self::State,
        transcript: &TranscriptId,
    ) -> Result<Vec<Turn<Extension>>, ReferenceError> {
        state
            .kit
            .store()
            .transcripts
            .get(transcript)
            .map(|held| held.turns.clone())
            .ok_or_else(|| {
                refuse(
                    codes::UNKNOWN_TRANSCRIPT,
                    format!("Transcript {transcript} is not held."),
                )
            })
    }
    async fn stage(
        &self,
        state: &mut Self::State,
        mut turn: Turn<Extension>,
    ) -> Result<TurnId, ReferenceError> {
        let id = TurnId(format!("turn-{}", state.kit.store().next()));
        turn.id = Some(id.clone());
        turn.created_at = self.clock().unix_ms();
        state.kit.run.staged.push(turn);
        Ok(id)
    }
    async fn revise(
        &self,
        state: &mut Self::State,
        revision: Revision<Extension>,
    ) -> Result<(), ReferenceError> {
        let transcript = state.kit.run.transcript.clone();
        let mut store = state.kit.store();
        let held = store.transcripts.get_mut(&transcript).and_then(|held| {
            held.turns
                .iter_mut()
                .find(|turn| turn.id.as_ref() == Some(&revision.turn))
        });
        match held {
            Some(turn) => {
                turn.content = revision.content;
                turn.revision += 1;
                Ok(())
            }
            None => Err(refuse(
                codes::UNKNOWN_TURN,
                format!(
                    "Transcript {transcript} does not hold turn {}.",
                    revision.turn
                ),
            )),
        }
    }
    async fn memory(
        &self,
        state: &Self::State,
        transcript: &TranscriptId,
    ) -> Result<Memory<Extension>, ReferenceError> {
        let store = state.kit.store();
        let held = store.transcripts.get(transcript).ok_or_else(|| {
            refuse(
                codes::UNKNOWN_TRANSCRIPT,
                format!("Transcript {transcript} is not held."),
            )
        })?;
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
            summary: held.summary.clone(),
            turns: held.turns[after..].to_vec(),
        })
    }
    async fn write_summary(
        &self,
        state: &mut Self::State,
        summary: Summary<Extension>,
    ) -> Result<(), ReferenceError> {
        let transcript = state.kit.run.transcript.clone();
        let mut store = state.kit.store();
        let held = store.transcripts.get_mut(&transcript).ok_or_else(|| {
            refuse(
                codes::UNKNOWN_TRANSCRIPT,
                format!("Transcript {transcript} is not held."),
            )
        })?;
        held.summary = Some(summary);
        Ok(())
    }
    fn engine(
        &self,
        state: &Self::State,
        _node: &str,
    ) -> Result<Arc<dyn EngineAdapter>, ReferenceError> {
        Ok(state.kit.store().engine.clone())
    }
    fn tool_plan(&self, state: &Self::State, _node: &str) -> Result<ToolPlan, ReferenceError> {
        Ok(state.kit.store().tools.clone())
    }
    async fn reserve(
        &self,
        state: &mut Self::State,
        node: &str,
        estimate: SpendEstimate,
    ) -> Result<Reservation, ReferenceError> {
        let total = estimate.input_tokens + estimate.max_output_tokens.unwrap_or(0);
        let id = {
            let mut store = state.kit.store();
            if let Some(budget) = store.budget {
                if total > budget {
                    return Err(refuse(
                        codes::SPEND_REFUSED,
                        format!("{total} tokens exceed the {budget} the host allows."),
                    ));
                }
                store.budget = Some(budget - total);
            }
            format!("reservation-{}", store.next())
        };
        let reservation = Reservation {
            id,
            node: node.into(),
            estimate,
        };
        state.kit.run.reservations.push(reservation.clone());
        Ok(reservation)
    }
    async fn settle(
        &self,
        state: &mut Self::State,
        reservation: Reservation,
        usage: EngineTokenUsage,
    ) -> Result<(), ReferenceError> {
        let open = &mut state.kit.run.reservations;
        let Some(position) = open.iter().position(|held| held.id == reservation.id) else {
            return Err(ReferenceError::internal(format!(
                "Reservation {} is not open.",
                reservation.id
            )));
        };
        open.remove(position);
        state.kit.store().ledger.push((reservation, usage));
        Ok(())
    }
    async fn record_call(
        &self,
        state: &mut Self::State,
        record: CallRecord,
    ) -> Result<(), ReferenceError> {
        state.kit.run.calls.push(record);
        Ok(())
    }
    fn policy(
        &self,
        state: &Self::State,
        transcript: &TranscriptId,
    ) -> Result<Policy, ReferenceError> {
        let store = state.kit.store();
        if !store.transcripts.contains_key(transcript) {
            return Err(refuse(
                codes::UNKNOWN_TRANSCRIPT,
                format!("Transcript {transcript} is not held."),
            ));
        }
        Ok(store.policy.clone())
    }
    async fn request_compaction(
        &self,
        state: &mut Self::State,
        evaluation: PolicyEvaluation,
    ) -> Result<JobId, ReferenceError> {
        let mut store = state.kit.store();
        let id = JobId(format!("job-{}", store.next()));
        store.jobs.insert(
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
    async fn transition(
        &self,
        state: &mut Self::State,
        job: &JobId,
        to: JobState,
    ) -> Result<(), ReferenceError> {
        match state.kit.store().jobs.get_mut(job) {
            Some(held) => {
                held.state = to;
                Ok(())
            }
            None => Err(refuse(
                codes::UNKNOWN_JOB,
                format!("Job {job} is not held."),
            )),
        }
    }
    async fn deliver(&self, state: &mut Self::State, text: String) -> Result<(), ReferenceError> {
        state.kit.run.deliveries.push(text);
        Ok(())
    }
}

/// One response a [`ScriptedEngine`] returns: text, or a batch of tool calls it runs
/// through the request's executor before taking the next response as the model's next turn.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptedResponse {
    Text(String),
    ToolCalls(Vec<EngineLocalToolCall>),
}

/// An [`EngineAdapter`] that returns scripted responses in order and counts a request's
/// input tokens as the words of its messages.
pub struct ScriptedEngine {
    clock: Arc<dyn Clock>,
    script: Mutex<VecDeque<ScriptedResponse>>,
}
impl ScriptedEngine {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            script: Mutex::default(),
        }
    }
    /// Appends responses to the script.
    pub fn extend(&self, responses: impl IntoIterator<Item = ScriptedResponse>) {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(responses);
    }
    /// The responses not yet returned.
    pub fn remaining(&self) -> usize {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
    fn take(&self) -> Option<ScriptedResponse> {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
    }
    /// The words of the request's messages: the scripted engine's token count.
    pub fn words(request: &EngineTextStreamRequest) -> u64 {
        request
            .shared
            .messages
            .iter()
            .map(|message| message.text_content().split_whitespace().count() as u64)
            .sum()
    }
    fn now(&self) -> String {
        utc_now(self.clock.as_ref()).to_rfc3339()
    }
}

#[async_trait]
impl EngineAdapter for ScriptedEngine {
    fn describe_debug_context(&self) -> EngineAdapterDebugContext {
        EngineAdapterDebugContext {
            provider: "reference".into(),
            backend: "scripted".into(),
            model: "scripted".into(),
            sdk: "battersea-kit-conversation".into(),
            base_url: None,
            timeout_ms: None,
            max_retries: None,
        }
    }
    async fn count_text_stream_input_tokens(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<u64, EngineAdapterRequestError> {
        Ok(Self::words(&request))
    }
    async fn stream_text(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<EngineTextStream, EngineAdapterRequestError> {
        let input = Self::words(&request);
        // Events already produced, tool evidence among them, reach the caller before an
        // error ends the stream.
        let mut events: Vec<Result<EngineTextStreamEvent, EngineAdapterRequestError>> = vec![];
        let mut round = 0u32;
        loop {
            let Some(response) = self.take() else {
                let error = EngineAdapterRequestError::new(
                    "reference",
                    "The script has no response left.",
                    ErrorKind::Mock,
                );
                if events.is_empty() {
                    return Err(error);
                }
                events.push(Err(error));
                break;
            };
            events.push(Ok(EngineTextStreamEvent::MessageStart {
                turn_index: round,
            }));
            match response {
                ScriptedResponse::Text(text) => {
                    let output = text.split_whitespace().count() as u64;
                    events.push(Ok(EngineTextStreamEvent::TextDelta { text }));
                    events.push(Ok(EngineTextStreamEvent::MessageStop {
                        reason: StopReason::Complete,
                    }));
                    events.push(Ok(EngineTextStreamEvent::TokenUsage {
                        usage: EngineTokenUsage {
                            turn_index: round,
                            input_tokens: Some(input),
                            output_tokens: Some(output),
                            total_tokens: Some(input + output),
                            ..EngineTokenUsage::default()
                        },
                    }));
                    break;
                }
                ScriptedResponse::ToolCalls(calls) => {
                    if round as usize >= request.max_tool_rounds {
                        let error = EngineAdapterRequestError::new(
                            "reference",
                            "The script asks for more tool rounds than the request allows.",
                            ErrorKind::InvalidRequest,
                        );
                        if events.len() == 1 {
                            return Err(error);
                        }
                        events.push(Err(error));
                        break;
                    }
                    for call in calls {
                        events.push(Ok(EngineTextStreamEvent::ToolCallStarted {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            arguments: call.arguments.clone(),
                            started_at: self.now(),
                        }));
                        let result = match &request.local_tool_executor {
                            Some(executor) => executor.call(call.clone()).await,
                            None => Err(EngineAdapterRequestError::new(
                                "reference",
                                format!("Tool {:?} has no executor.", call.name),
                                ErrorKind::InvalidRequest,
                            )),
                        };
                        // An executor's own evidence stands; a failure without any is
                        // uncertain, as the model crate's tool loop records it.
                        let evidence = match &result {
                            Ok(_) => ToolCallEvidence {
                                call_id: call.id.clone(),
                                tool_name: call.name.clone(),
                                outcome: ToolCallOutcome::Completed,
                                request_id: None,
                            },
                            Err(error) => {
                                error.tool_call.as_deref().cloned().unwrap_or_else(|| {
                                    ToolCallEvidence {
                                        call_id: call.id.clone(),
                                        tool_name: call.name.clone(),
                                        outcome: ToolCallOutcome::Uncertain,
                                        request_id: error.request_id.as_deref().map(String::from),
                                    }
                                })
                            }
                        };
                        events.push(Ok(match result {
                            Ok(result) => EngineTextStreamEvent::ToolCallCompleted {
                                id: call.id,
                                name: call.name,
                                ok: !result.is_error(),
                                result: result.provider_content().unwrap_or(Value::Null),
                                error_message: None,
                                error_classification: None,
                                tool_result: Some(Box::new(result)),
                                tool_call: Some(evidence),
                                finished_at: self.now(),
                            },
                            Err(error) => EngineTextStreamEvent::ToolCallCompleted {
                                id: call.id,
                                name: call.name,
                                ok: false,
                                result: Value::Null,
                                error_message: Some(error.message.to_string()),
                                error_classification: Some(
                                    error.classification.as_str().to_string(),
                                ),
                                tool_result: None,
                                tool_call: Some(evidence),
                                finished_at: self.now(),
                            },
                        }));
                    }
                    events.push(Ok(EngineTextStreamEvent::MessageStop {
                        reason: StopReason::ToolUse,
                    }));
                    round += 1;
                }
            }
        }
        Ok(Box::pin(stream::iter(events)))
    }
}

#[cfg(feature = "conformance")]
#[async_trait]
impl crate::conformance::Probe for ReferenceHost<State> {
    fn extension(&self) -> Extension {
        Value::Null
    }
    async fn seed(
        &self,
        state: &mut Self::State,
        transcript: TranscriptId,
        turns: Vec<Turn<Extension>>,
        summary: Option<Summary<Extension>>,
    ) -> Result<(), ReferenceError> {
        let mut store = state.kit.store();
        for id in turns.iter().filter_map(|turn| turn.id.as_ref()) {
            store.reserve(id.as_str());
        }
        if let Some(job) = summary.as_ref().and_then(|summary| summary.job.as_ref()) {
            store.reserve(job.as_str());
        }
        let turns = turns
            .into_iter()
            .map(|mut turn| {
                if turn.id.is_none() {
                    turn.id = Some(TurnId(format!("turn-{}", store.next())));
                }
                turn
            })
            .collect();
        store
            .transcripts
            .insert(transcript, Transcript { turns, summary });
        Ok(())
    }
    async fn script(
        &self,
        state: &mut Self::State,
        responses: Vec<ScriptedResponse>,
    ) -> Result<(), ReferenceError> {
        state.kit.engine().extend(responses);
        Ok(())
    }
    async fn snapshot(
        &self,
        state: &Self::State,
    ) -> Result<crate::conformance::Snapshot<Extension>, ReferenceError> {
        let store = state.kit.store();
        Ok(crate::conformance::Snapshot {
            transcripts: store
                .transcripts
                .iter()
                .map(|(id, held)| {
                    (
                        id.clone(),
                        crate::conformance::TranscriptSnapshot {
                            turns: held.turns.clone(),
                            summary: held.summary.clone(),
                        },
                    )
                })
                .collect(),
            staged: state.kit.run.staged.clone(),
            jobs: store.jobs.values().cloned().collect(),
            calls: state.kit.run.calls.clone(),
            reservations: state.kit.run.reservations.clone(),
            deliveries: state.kit.run.deliveries.clone(),
        })
    }
}
