//! Copyright (c) Scott A Dixon
//!
//! The kit's conformance suite: plays its cases on the reference host and on an
//! application's host, reached through [`Port`] and [`Probe`] alone.
use crate::{
    reference, register_handlers, register_token_types, CallRecord, CompactionJob, JobId, JobState,
    Options, PolicyEvaluation, Port, Reservation, Summary, ToolPlan, TranscriptId, Turn, KIT,
    MANIFEST,
};
use async_trait::async_trait;
use battersea_flow::{registry::Registry, FlowExecutionLimits, FlowQueueLimits};
pub use battersea_kit::conformance::{Divergence, Failure, Fixture, Outcome, Played, Side};
use battersea_kit::{
    conformance::{Case, PortAccess, Suite},
    ManualClock, ReferenceHost, ReferenceState,
};
use battersea_model::{
    EngineAdapterRequestError, EngineLocalToolCall, EngineLocalToolDefinition,
    EngineLocalToolExecutor, EngineLocalToolResult, Role,
};
use battersea_runtime::{ExecutionError, ExecutionEvent, RegistryBuilder, SchedulerState};
pub use reference::ScriptedResponse;
use serde::Serialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

/// What the suite needs of a host beside its port: seeding, scripting and observation.
#[async_trait]
pub trait Probe: Port {
    /// The extension the suite's seeded records carry.
    fn extension(&self) -> Self::Extension;
    /// Creates `transcript` holding `turns` committed and `summary` as its memory. A turn
    /// without an id is assigned one.
    async fn seed(
        &self,
        state: &mut Self::State,
        transcript: TranscriptId,
        turns: Vec<Turn<Self::Extension>>,
        summary: Option<Summary<Self::Extension>>,
    ) -> Result<(), Self::Error>;
    /// Creates `job` as the port holds it, under its id.
    async fn seed_job(
        &self,
        state: &mut Self::State,
        job: CompactionJob<Self::Extension>,
    ) -> Result<(), Self::Error>;
    /// Installs the responses the engine returns, in order.
    async fn script(
        &self,
        state: &mut Self::State,
        responses: Vec<ScriptedResponse>,
    ) -> Result<(), Self::Error>;
    /// Offers `plan` to every chat node of the run.
    async fn offer_tools(&self, state: &mut Self::State, plan: ToolPlan)
        -> Result<(), Self::Error>;
    /// The observable state.
    async fn snapshot(&self, state: &Self::State)
        -> Result<Snapshot<Self::Extension>, Self::Error>;
}

/// A transcript as a host holds it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TranscriptSnapshot<X> {
    pub turns: Vec<Turn<X>>,
    pub summary: Option<Summary<X>>,
}

/// A host's observable state: its transcripts, the run's staged turns, its jobs and the
/// run's call records, open reservations and deliveries.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Snapshot<X> {
    pub transcripts: BTreeMap<TranscriptId, TranscriptSnapshot<X>>,
    pub staged: Vec<Turn<X>>,
    pub jobs: Vec<CompactionJob<X>>,
    pub calls: Vec<CallRecord>,
    pub reservations: Vec<Reservation>,
    pub deliveries: Vec<String>,
}
/// Ids of one kind, each replaced by its order of appearance.
#[derive(Default)]
struct Aliases(BTreeMap<String, String>);
impl Aliases {
    /// Numbers `id` if it is new.
    fn number(&mut self, kind: &str, id: &str) {
        let next = self.0.len();
        self.0
            .entry(id.to_string())
            .or_insert_with(|| format!("{kind}#{next}"));
    }
    /// The alias of `id`, or `id` itself where it was never numbered.
    fn alias(&self, id: &str) -> String {
        self.0.get(id).cloned().unwrap_or_else(|| id.to_string())
    }
    /// Numbers `id` if it is new and returns its alias.
    fn next(&mut self, kind: &str, id: &str) -> String {
        self.number(kind, id);
        self.alias(id)
    }
}

/// The run's events as the suite compares them: in every kit record a token carries, the
/// extension is removed and each turn and job id replaced by its order of appearance.
pub fn comparable_events(events: &[Value]) -> Vec<Value> {
    let mut turns = Aliases::default();
    let mut jobs = Aliases::default();
    let mut record = |value: &mut Value, token_type: &str| {
        let mut turn = |turn: &mut Value| {
            if let Some(record) = turn.as_object_mut() {
                record.remove("extension");
                if let Some(id) = record.get("id").and_then(Value::as_str) {
                    let alias = turns.next("turn", id);
                    record.insert("id".into(), json!(alias));
                }
            }
        };
        match token_type {
            "conversation.turn" => turn(value),
            "conversation.turns" => {
                for item in value.as_array_mut().into_iter().flatten() {
                    turn(item);
                }
            }
            "conversation.summary" | "conversation.revision" => {
                if let Some(record) = value.as_object_mut() {
                    record.remove("extension");
                    for field in ["through", "turn"] {
                        if let Some(id) = record.get(field).and_then(Value::as_str) {
                            let alias = turns.next("turn", id);
                            record.insert(field.into(), json!(alias));
                        }
                    }
                    if let Some(id) = record.get("job").and_then(Value::as_str) {
                        let alias = jobs.next("job", id);
                        record.insert("job".into(), json!(alias));
                    }
                }
            }
            _ => {}
        }
    };
    events
        .iter()
        .map(|event| {
            let mut event = event.clone();
            let token_type = event["detail"]["tokenType"].as_str().map(str::to_owned);
            if let (Some(token_type), Some(value)) =
                (token_type, event.pointer_mut("/detail/value"))
            {
                record(value, &token_type);
            }
            event
        })
        .collect()
}

impl<X> Snapshot<X> {
    /// The snapshot as the suite compares it: the kit records' extensions removed, and
    /// every turn, job and reservation id replaced by its order of appearance, since each
    /// host names them. Content and payloads are left as they are.
    pub fn comparable(&self) -> Value {
        let mut turns = Aliases::default();
        let mut jobs = Aliases::default();
        let mut reservations = Aliases::default();
        for turn in self
            .transcripts
            .values()
            .flat_map(|transcript| &transcript.turns)
            .chain(&self.staged)
        {
            if let Some(id) = &turn.id {
                turns.number("turn", id.as_str());
            }
        }
        for job in &self.jobs {
            jobs.number("job", job.id.as_str());
        }
        for reservation in self.reservations.iter().chain(
            self.calls
                .iter()
                .filter_map(|call| call.reservation.as_ref()),
        ) {
            reservations.number("reservation", &reservation.id);
        }
        let turn = |turn: &Turn<X>| {
            json!({
                "id": turn.id.as_ref().map(|id| turns.alias(id.as_str())),
                "role": turn.role,
                "content": turn.content,
                "created_at": turn.created_at,
                "revision": turn.revision,
            })
        };
        let summary = |summary: &Summary<X>| {
            json!({
                "text": summary.text,
                "through": turns.alias(summary.through.as_str()),
                "job": summary.job.as_ref().map(|job| jobs.alias(job.as_str())),
            })
        };
        let reservation = |reservation: &Reservation| {
            json!({
                "id": reservations.alias(&reservation.id),
                "node": reservation.node,
                "estimate": reservation.estimate,
            })
        };
        json!({
            "transcripts": self
                .transcripts
                .iter()
                .map(|(id, transcript)| {
                    (
                        id.to_string(),
                        json!({
                            "turns": transcript.turns.iter().map(turn).collect::<Vec<_>>(),
                            "summary": transcript.summary.as_ref().map(summary),
                        }),
                    )
                })
                .collect::<serde_json::Map<_, _>>(),
            "staged": self.staged.iter().map(turn).collect::<Vec<_>>(),
            "jobs": self
                .jobs
                .iter()
                .map(|job| {
                    json!({
                        "id": jobs.alias(job.id.as_str()),
                        "transcript": job.transcript,
                        "evaluation": job.evaluation,
                        "state": job.state,
                    })
                })
                .collect::<Vec<_>>(),
            "calls": self
                .calls
                .iter()
                .map(|call| {
                    json!({
                        "node": call.node,
                        "status": call.status,
                        "reservation": call.reservation.as_ref().map(reservation),
                        "started_at": call.started_at,
                        "finished_at": call.finished_at,
                    })
                })
                .collect::<Vec<_>>(),
            "reservations": self.reservations.iter().map(reservation).collect::<Vec<_>>(),
            "deliveries": self.deliveries,
        })
    }
}

/// A port operation a case performs before its activation.
#[derive(Debug, Clone)]
pub enum Op {
    /// Creates a transcript with committed turns and a summary.
    Seed {
        transcript: TranscriptId,
        turns: Vec<Turn<()>>,
        summary: Option<Summary<()>>,
    },
    /// Creates a compaction job.
    Job { job: CompactionJob<()> },
    /// Installs the engine's responses.
    Script { responses: Vec<ScriptedResponse> },
    /// Offers tools, each answered by [`Echo`].
    Tools { plan: ToolPlan },
}

/// The suite's tool executor: answers every call with its arguments.
pub struct Echo;
#[async_trait]
impl EngineLocalToolExecutor for Echo {
    async fn call(
        &self,
        call: EngineLocalToolCall,
    ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
        Ok(EngineLocalToolResult::json(call.arguments))
    }
}

/// The kit's port access: applies [`Op`]s through [`Probe`] and observes its snapshot.
pub struct Access;
impl<H: Probe> PortAccess<H, Op> for Access {
    async fn prepare(&self, host: &H, state: &mut H::State, operation: &Op) -> Result<(), String> {
        let message = |error: H::Error| error.message().to_string();
        match operation {
            Op::Seed {
                transcript,
                turns,
                summary,
            } => {
                let turns = turns
                    .iter()
                    .map(|turn| Turn {
                        id: turn.id.clone(),
                        role: turn.role,
                        content: turn.content.clone(),
                        created_at: turn.created_at,
                        revision: turn.revision,
                        extension: host.extension(),
                    })
                    .collect();
                let summary = summary.as_ref().map(|summary| Summary {
                    text: summary.text.clone(),
                    through: summary.through.clone(),
                    job: summary.job.clone(),
                    extension: host.extension(),
                });
                host.seed(state, transcript.clone(), turns, summary)
                    .await
                    .map_err(message)
            }
            Op::Job { job } => {
                let job = CompactionJob {
                    id: job.id.clone(),
                    transcript: job.transcript.clone(),
                    evaluation: job.evaluation.clone(),
                    state: job.state,
                    extension: host.extension(),
                };
                host.seed_job(state, job).await.map_err(message)
            }
            Op::Script { responses } => {
                host.script(state, responses.clone()).await.map_err(message)
            }
            Op::Tools { plan } => host.offer_tools(state, plan.clone()).await.map_err(message),
        }
    }
    async fn observe(&self, host: &H, state: &H::State) -> Result<Value, String> {
        Ok(host
            .snapshot(state)
            .await
            .map_err(|error| error.message().to_string())?
            .comparable())
    }
}

/// The transcript every case's run is on.
pub const TRANSCRIPT: &str = "conformance";

/// Registers the kit's token types with the extension schema `true`, and the shared
/// vocabulary the kit references, as the generic kit registers it.
pub fn register_for_suite(registry: &mut Registry) -> Result<(), String> {
    for name in battersea_kit::SHARED_TOKEN_TYPES {
        registry
            .register_token_type(name, json!(true))
            .map_err(|error| error.to_string())?;
    }
    register_token_types(
        registry,
        &Options {
            extension_schema: json!(true),
        },
    )
}

/// A flow of the kit's nodes: `sources` in materialisation order, `body` its nodes and edges.
/// `{queue}` in `body` is the queue a streaming edge declares.
fn flow(key: &str, sources: &[&str], body: &str) -> String {
    let limits = serde_json::to_string(&FlowExecutionLimits::default()).expect("limits");
    let sources = serde_json::to_string(sources).expect("sources");
    let queue = serde_json::to_string(&FlowQueueLimits::default()).expect("queue");
    let body = body.replace("{queue}", &queue);
    format!(
        r#"flow 1 "{key}" {{
    version = 3;
    title = "{key}";
    execution = {{"source_order": {sources}, "limits": {limits}}};
{body}
}}"#
    )
}

/// A case activating `node` with `values`.
fn case(name: &str, sources: &[&str], body: &str, node: &str, values: Value) -> Case<Op> {
    Case {
        name: name.into(),
        flow: flow(name, sources, body),
        node: node.into(),
        values: serde_json::from_value(values).expect("activation values"),
        steps: vec![],
        cancel: false,
        before: vec![],
    }
}

fn turn(id: Option<&str>, role: Role, text: &str) -> Turn<()> {
    Turn {
        id: id.map(Into::into),
        ..Turn::text(role, text, ())
    }
}

/// Seeds the case's transcript with `turns` and `summary`.
fn seed(turns: Vec<Turn<()>>, summary: Option<Summary<()>>) -> Op {
    Op::Seed {
        transcript: TRANSCRIPT.into(),
        turns,
        summary,
    }
}

fn script(responses: Vec<ScriptedResponse>) -> Op {
    Op::Script { responses }
}

/// The prompt source activated with `prompt` and a sink delivering its text.
const PROMPT: &str = r#"    node "prompt" "ConversationPrompt" {"instance_name": "Prompt"}"#;

/// The kit's cases.
pub fn cases() -> Vec<Case<Op>> {
    let two_turns = || {
        vec![
            turn(None, Role::User, "Where is the cat?"),
            turn(None, Role::Assistant, "On the mat."),
        ]
    };
    let mut prompt = case(
        "prompt",
        &["prompt", "static"],
        &format!(
            r#"{PROMPT}
    node "static" "ConversationStaticPrompt" {{"instance_name": "Static", "parameter_values": {{"text": "Be brief."}}}}
    node "tap" "ConversationSignalTap" {{"instance_name": "Tap"}}
    node "out" "ConversationOutput" {{"instance_name": "Out"}}
    node "static_out" "ConversationOutput" {{"instance_name": "Static out"}}
    node "stage" "ConversationTranscriptOutput" {{"instance_name": "Stage"}}
    edge "e1" "prompt"."text" -> "tap"."input" {{"order": 0}}
    edge "e2" "tap"."output" -> "out"."text" {{"order": 0}}
    edge "e3" "static"."text" -> "static_out"."text" {{"order": 0}}
    edge "e4" "prompt"."turn" -> "stage"."turns" {{"order": 0}}"#
        ),
        "prompt",
        json!({"prompt": "Hello there"}),
    );
    prompt.before = vec![seed(vec![], None)];

    let mut transcript = case(
        "transcript",
        &["prompt", "transcript"],
        &format!(
            r#"{PROMPT}
    node "transcript" "ConversationTranscript" {{"instance_name": "Transcript"}}
    node "out" "ConversationOutput" {{"instance_name": "Out"}}
    edge "e1" "transcript"."text" -> "out"."text" {{"order": 0}}"#
        ),
        "prompt",
        json!({"prompt": "unused"}),
    );
    transcript.before = vec![seed(two_turns(), None)];

    let mut memory = case(
        "memory-budget",
        &["prompt", "memory"],
        &format!(
            r#"{PROMPT}
    node "memory" "ConversationMemory" {{"instance_name": "Memory", "parameter_values": {{"budget": 3}}}}
    node "out" "ConversationOutput" {{"instance_name": "Out"}}
    edge "e1" "memory"."context" -> "out"."text" {{"order": 0}}"#
        ),
        "prompt",
        json!({"prompt": "unused"}),
    );
    let mut calling = turn(Some("t3"), Role::Assistant, "Let me check the weather");
    calling
        .content
        .push(battersea_model::ContentBlock::ToolCall {
            id: "call-1".into(),
            name: "weather".into(),
            arguments: json!({"city": "London"}),
        });
    let mut answering = turn(Some("t4"), Role::Tool, "");
    answering.content = vec![battersea_model::ContentBlock::ToolResult {
        id: "call-1".into(),
        content: json!({"forecast": "rain"}),
    }];
    memory.before = vec![seed(
        vec![
            turn(Some("t1"), Role::User, "Where is the cat?"),
            turn(Some("t2"), Role::Assistant, "On the mat."),
            calling,
            answering,
            turn(Some("t5"), Role::User, "short reply"),
        ],
        Some(Summary {
            text: "The cat was found.".into(),
            through: "t2".into(),
            job: None,
            extension: (),
        }),
    )];

    let chat_flow = |chat_parameters: &str, retain: u64| {
        format!(
            r#"{PROMPT}
    node "system" "ConversationStaticPrompt" {{"instance_name": "System", "parameter_values": {{"text": "Be brief."}}}}
    node "memory" "ConversationMemory" {{"instance_name": "Memory"}}
    node "chat" "ConversationChat" {{"instance_name": "Chat", "parameter_values": {chat_parameters}}}
    node "buffer" "ConversationBuffer" {{"instance_name": "Buffer", "parameter_values": {{"retain": {retain}}}}}
    node "out" "ConversationOutput" {{"instance_name": "Out"}}
    node "stage" "ConversationTranscriptOutput" {{"instance_name": "Stage"}}
    edge "e1" "system"."text" -> "chat"."system" {{"order": 0}}
    edge "e2" "memory"."turns" -> "chat"."history" {{"order": 1}}
    edge "e3" "prompt"."text" -> "chat"."prompt" {{"order": 2}}
    edge "e4" "chat"."text" -> "buffer"."stream" {{"order": 0, "queue": {{queue}}}}
    edge "e5" "buffer"."text" -> "out"."text" {{"order": 0}}
    edge "e6" "chat"."reply" -> "stage"."turns" {{"order": 0}}"#
        )
    };
    let chat_sources: &[&str] = &["prompt", "system", "memory"];
    let mut chat_stream = case(
        "chat-stream",
        chat_sources,
        &chat_flow(r#"{"budget": 100, "max_output_tokens": 50}"#, 1024),
        "prompt",
        json!({"prompt": "Where is the dog?"}),
    );
    chat_stream.before = vec![
        seed(two_turns(), None),
        script(vec![
            ScriptedResponse::Reasoning("Thinking about dogs.".into()),
            ScriptedResponse::Text("In the garden.".into()),
        ]),
    ];

    let mut chat_tools = case(
        "chat-tools",
        chat_sources,
        &chat_flow(r#"{"budget": 100, "max_tool_rounds": 2}"#, 1024),
        "prompt",
        json!({"prompt": "Echo this"}),
    );
    chat_tools.before = vec![
        seed(vec![], None),
        Op::Tools {
            plan: ToolPlan {
                definitions: vec![EngineLocalToolDefinition {
                    name: "echo".into(),
                    // ai-prompt-trojan-allow tool-description: the suite's fixture tool, offered to the scripted engine only.
                    description: "Returns its arguments.".into(),
                    input_schema: json!({"type": "object"}),
                }],
                executor: Some(Arc::new(Echo)),
                max_rounds: 0,
            },
        },
        script(vec![
            ScriptedResponse::ToolCalls(vec![EngineLocalToolCall {
                id: "call-1".into(),
                name: "echo".into(),
                arguments: json!({"text": "kept"}),
            }]),
            ScriptedResponse::Text("Echoed.".into()),
        ]),
    ];

    let mut over_budget = case(
        "chat-over-budget",
        chat_sources,
        &chat_flow(r#"{"budget": 2}"#, 1024),
        "prompt",
        json!({"prompt": "This prompt is far too long for the budget"}),
    );
    over_budget.before = vec![seed(vec![], None)];

    let mut cancelled = case(
        "chat-cancelled",
        chat_sources,
        &chat_flow(r#"{"budget": 100}"#, 1024),
        "prompt",
        json!({"prompt": "Take your time"}),
    );
    cancelled.cancel = true;
    cancelled.before = vec![
        seed(vec![], None),
        script(vec![
            ScriptedResponse::Wait(Duration::from_secs(5)),
            ScriptedResponse::Text("Too late.".into()),
        ]),
    ];

    let mut overflow = case(
        "buffer-overflow",
        chat_sources,
        &chat_flow(r#"{"budget": 100}"#, 4),
        "prompt",
        json!({"prompt": "Say a lot"}),
    );
    overflow.before = vec![
        seed(vec![], None),
        script(vec![ScriptedResponse::Text(
            "A reply longer than four bytes.".into(),
        )]),
    ];

    let mut stage = case(
        "stage",
        &["prompt", "memory"],
        &format!(
            r#"{PROMPT}
    node "memory" "ConversationMemory" {{"instance_name": "Memory"}}
    node "stage_prompt" "ConversationTranscriptOutput" {{"instance_name": "Stage prompt"}}
    node "stage_memory" "ConversationTranscriptOutput" {{"instance_name": "Stage memory"}}
    edge "e1" "memory"."turns" -> "stage_memory"."turns" {{"order": 0}}
    edge "e2" "prompt"."turn" -> "stage_prompt"."turns" {{"order": 0}}"#
        ),
        "prompt",
        json!({"prompt": "And again"}),
    );
    stage.before = vec![seed(two_turns(), None)];

    let mut compaction = case(
        "compaction-request",
        &["prompt"],
        &format!(
            r#"{PROMPT}
    node "over" "ConversationCompactionRequest" {{"instance_name": "Over", "parameter_values": {{"max_turns": 2}}}}
    node "under" "ConversationCompactionRequest" {{"instance_name": "Under", "parameter_values": {{"max_turns": 5}}}}
    edge "e1" "prompt"."turn" -> "over"."turns" {{"order": 0}}
    edge "e2" "prompt"."turn" -> "under"."turns" {{"order": 1}}"#
        ),
        "prompt",
        json!({"prompt": "One more"}),
    );
    compaction.before = vec![seed(two_turns(), None)];

    let mut memory_output = case(
        "memory-output",
        &["prompt", "memory"],
        &format!(
            r#"{PROMPT}
    node "memory" "ConversationMemory" {{"instance_name": "Memory"}}
    node "parse" "ConversationCompactionParser" {{"instance_name": "Parse"}}
    node "write" "ConversationMemoryOutput" {{"instance_name": "Write"}}
    edge "e1" "memory"."turns" -> "parse"."turns" {{"order": 0}}
    edge "e2" "prompt"."text" -> "parse"."text" {{"order": 1}}
    edge "e3" "parse"."summary" -> "write"."summary" {{"order": 0}}"#
        ),
        "prompt",
        json!({"prompt": "The cat is on the mat.", "job": "job-1"}),
    );
    memory_output.before = vec![
        seed(two_turns(), None),
        Op::Job {
            job: CompactionJob {
                id: JobId::from("job-1"),
                transcript: TRANSCRIPT.into(),
                evaluation: PolicyEvaluation {
                    transcript: TRANSCRIPT.into(),
                    trigger: crate::Trigger::AfterRun,
                    decision: crate::Decision::Compact,
                    measured: crate::Measured {
                        turns: 2,
                        tokens: 7,
                    },
                    policy: crate::Policy {
                        max_turns: Some(1),
                        ..crate::Policy::default()
                    },
                },
                state: JobState::Running,
                extension: (),
            },
        },
    ];

    let mut refine = case(
        "refine",
        &["source", "response"],
        r#"    node "source" "ConversationRefinementSource" {"instance_name": "Source"}
    node "response" "ConversationStaticPrompt" {"instance_name": "Response", "parameter_values": {"text": "On the mat, asleep."}}
    node "out" "ConversationOutput" {"instance_name": "Out"}
    node "parse" "ConversationRefinementParser" {"instance_name": "Parse"}
    node "apply" "ConversationRefinementOutput" {"instance_name": "Apply"}
    edge "e1" "source"."text" -> "out"."text" {"order": 0}
    edge "e2" "source"."turn" -> "parse"."turn" {"order": 0}
    edge "e3" "response"."text" -> "parse"."text" {"order": 1}
    edge "e4" "parse"."revision" -> "apply"."revision" {"order": 0}"#,
        "source",
        json!({"turn": "t2"}),
    );
    refine.before = vec![seed(
        vec![
            turn(Some("t1"), Role::User, "Where is the cat?"),
            turn(Some("t2"), Role::Assistant, "On the mat."),
        ],
        None,
    )];

    vec![
        prompt,
        transcript,
        memory,
        chat_stream,
        chat_tools,
        over_budget,
        cancelled,
        overflow,
        stage,
        compaction,
        memory_output,
        refine,
    ]
}

/// The suite: the kit, its manifest and its cases.
pub fn suite() -> Suite<Op> {
    Suite {
        kit: &KIT,
        manifests: vec![(crate::NAME, MANIFEST)],
        register_token_types: register_for_suite,
        comparable_events,
        cases: cases(),
    }
}

/// The reference host with the kit's handlers, as the suite's oracle.
pub fn reference_host(clock: Arc<ManualClock>) -> ReferenceHost<reference::State> {
    let mut builder = RegistryBuilder::default();
    register_handlers(&mut builder).expect("the kit's handlers");
    reference::host(builder.build(), clock)
}

/// The reference host as the suite's oracle.
pub struct ReferenceFixture;
impl Fixture for ReferenceFixture {
    type Host = ReferenceHost<reference::State>;
    fn host(&self, clock: Arc<ManualClock>) -> Self::Host {
        reference_host(clock)
    }
    fn state(
        &self,
        host: &Self::Host,
        scheduler: SchedulerState,
    ) -> ReferenceState<reference::State> {
        host.state(
            scheduler,
            reference::State::new(TRANSCRIPT, host.clock().clone()),
        )
    }
    fn events(&self, host: &Self::Host, run_id: &str) -> Vec<ExecutionEvent> {
        host.run_events(run_id)
    }
}

/// Plays every case on the reference host and on `fixture`'s host and reports the first
/// divergence.
pub async fn run<F: Fixture>(fixture: &F) -> Result<(), Failure>
where
    F::Host: Probe,
{
    battersea_kit::conformance::run(&suite(), &ReferenceFixture, fixture, &Access).await
}
