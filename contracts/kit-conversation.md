# Conversation kit

Status: proposed; designed in [#69](https://github.com/thirtytwobits/battersea/issues/69). Built
to the [kit contract](kits.md).

The conversation kit is the node set of a chat loop: it takes a prompt, calls a model with the
conversation so far, records the reply and keeps the conversation within a budget. Its handlers
reach the application's transcript, model, tools and spend through `Port`. The application
keeps its sessions, its storage and its own vocabulary; the kit never names them.

## Parts

| Part | Form |
|---|---|
| Crate | `battersea-kit-conversation`, kit name `conversation` |
| Descriptor | `pub const KIT: battersea_kit::Kit` |
| Definitions | `MANIFEST` at `src/nodes.yaml`; `register_token_types(registry, &Options { extension_schema })` |
| Handlers | `register_handlers<H: Port>` |
| Port | `pub trait Port: ExecutionHost` |
| Reference host | `impl Port for battersea_kit::ReferenceHost<reference::State>` |
| Compaction | `compaction::Policy`, `compaction::evaluate` and `compaction::Runner<H: Port>` |
| Conformance | `conformance::run`, behind the `conformance` feature |
| Example host | `examples/kit-conversation`, a transcript in files |

The kit has no editor package. Every parameter uses a built-in editor.

## Names

Handler ids are `conversation.*`, token types `conversation.*` and class names `Conversation*`.
The kit references `prompt.fragment` and `chat.raw` from the shared vocabulary.

| Class | Handler id | Kind | Behaviour |
|---|---|---|---|
| `ConversationPrompt` | `conversation.prompt` | source | Emits the activation's `prompt` value as `prompt.fragment` and as a user `conversation.turn` |
| `ConversationStaticPrompt` | `conversation.static_prompt` | source | Emits its `text` parameter as `prompt.fragment` |
| `ConversationTranscript` | `conversation.transcript.source` | source | Emits the transcript's committed turns as `conversation.turns` and renders them as `prompt.fragment` |
| `ConversationMemory` | `conversation.memory` | source | Emits the latest summary and the turns after it that fit `budget` tokens, as `conversation.turns` and rendered `prompt.fragment` |
| `ConversationChat` | `conversation.chat` | inline | Calls the model with `system`, `history` and `prompt`; streams `text` and `reasoning`; publishes `reply`, `status` and `tool_rounds` |
| `ConversationBuffer` | `conversation.buffer` | inline | Collects a `chat.raw` stream into one `prompt.fragment` within `retain` bytes |
| `ConversationSignalTap` | `conversation.tap` | inline | Passes its input through and fires `received` |
| `ConversationOutput` | `conversation.output` | sink | Delivers text to the application through the port |
| `ConversationTranscriptOutput` | `conversation.transcript.output` | sink | Stages each turn it receives for the application to commit |
| `ConversationCompactionRequest` | `conversation.compaction.request` | sink | Evaluates the policy and requests a compaction job when it decides to compact |
| `ConversationCompactionParser` | `conversation.compaction.parse` | inline | Parses a compaction response into a `conversation.summary` |
| `ConversationMemoryOutput` | `conversation.memory.output` | sink | Writes a summary as the transcript's memory and completes its job |
| `ConversationRefinementSource` | `conversation.refine.source` | source | Emits the turn named by the activation's `turn` value for refinement |
| `ConversationRefinementParser` | `conversation.refine.parse` | inline | Parses a refinement response into a `conversation.revision` |
| `ConversationRefinementOutput` | `conversation.refine.output` | sink | Revises a committed turn through the port |

`ConversationChat` fires `completed` when it publishes a reply and `over_budget` when the
budget check refuses dispatch. `ConversationCompactionRequest` fires `requested` or `held`.

## Types

Each record carries the application's extension at `extension`. The reference host's extension
schema is `true`.

| Token type | Record |
|---|---|
| `conversation.turn` | `Turn<X>`: `id`, `role` (`system`, `user`, `assistant`, `tool`), `content` as `battersea_model::ContentBlock` values, `created_at`, `revision`, `extension` |
| `conversation.turns` | `Vec<Turn<X>>` in transcript order |
| `conversation.summary` | `Summary<X>`: `text`, `through` (the id of the last turn it covers), `job`, `extension` |
| `conversation.revision` | `Revision<X>`: `turn`, `content`, `extension` |
| `conversation.call_status` | `CallStatus`: `outcome` (`completed`, `failed`, `cancelled`, `over_budget`), `stop_reason`, `usage` as `EngineTokenUsage`, `rounds` |
| `conversation.tool_rounds` | `Vec<ToolRound>`: each round's calls with `id`, `name`, `arguments`, `ok`, `result` and `ToolCallEvidence` |

The port assigns `id` and `created_at` when it stages a turn; a turn the kit emits before
staging has neither. `revision` counts the revisions applied to a committed turn, from zero.

Port records that are not tokens: `CompactionJob<X>` (`id`, `transcript`, `evaluation`,
`state`, `extension`) with states `requested`, `running`, `completed` and `failed`;
`PolicyEvaluation` (`transcript`, `trigger` as `after_run` or `idle`, `decision` as `compact`
or `hold`, `measured` turns and tokens after the summary, the `policy` applied); `CallRecord` (`node`, `status`, the
reservation it settled, `started_at`, `finished_at`); and `SpendEstimate`
(`input_tokens`, `max_output_tokens`).

## Port

```rust
pub trait Port: ExecutionHost {
    type Extension: Serialize + DeserializeOwned + Clone + Default + Send + Sync;
    fn run_state(state: &mut Self::State) -> &mut RunState<Self::Extension>;
    fn clock(&self) -> Arc<dyn Clock>;

    // Transcript
    async fn turns(&self, state: &Self::State, transcript: &TranscriptId) -> Result<Vec<Turn<Self::Extension>>, Self::Error>;
    async fn stage(&self, state: &mut Self::State, turn: Turn<Self::Extension>) -> Result<TurnId, Self::Error>;
    async fn revise(&self, state: &mut Self::State, revision: Revision<Self::Extension>) -> Result<(), Self::Error>;

    // Memory
    async fn memory(&self, state: &Self::State, transcript: &TranscriptId) -> Result<Memory<Self::Extension>, Self::Error>;
    async fn write_summary(&self, state: &mut Self::State, summary: Summary<Self::Extension>) -> Result<(), Self::Error>;

    // Calls
    fn engine(&self, state: &Self::State, node: &str) -> Result<Arc<dyn EngineAdapter>, Self::Error>;
    fn tool_plan(&self, state: &Self::State, node: &str) -> Result<ToolPlan, Self::Error>;
    async fn reserve(&self, state: &mut Self::State, node: &str, estimate: SpendEstimate) -> Result<Reservation, Self::Error>;
    async fn settle(&self, state: &mut Self::State, reservation: Reservation, usage: EngineTokenUsage) -> Result<(), Self::Error>;
    async fn record_call(&self, state: &mut Self::State, record: CallRecord) -> Result<(), Self::Error>;

    // Compaction
    fn policy(&self, state: &Self::State, transcript: &TranscriptId) -> Result<Policy, Self::Error>;
    async fn request_compaction(&self, state: &mut Self::State, evaluation: PolicyEvaluation) -> Result<JobId, Self::Error>;
    async fn transition(&self, state: &mut Self::State, job: &JobId, to: JobState) -> Result<(), Self::Error>;

    // Output
    async fn deliver(&self, state: &mut Self::State, text: String) -> Result<(), Self::Error>;
}
```

`Memory<X>` is the latest `Summary<X>`, if any, and the turns after it. `ToolPlan` is the
run's `EngineLocalToolDefinition`s, its `EngineLocalToolExecutor` and a round limit that caps a
chat node's `max_tool_rounds`; a plan without definitions runs the call without tools. `Policy`
is the compaction policy in force for the transcript. A record the kit creates, such as the
prompt's turn or a parsed summary, carries `Extension::default()`; the port replaces it when it
stages, writes or applies the record. The kit's timestamps read the port's clock.

`RunState<X>` declares `STATE_VERSION`. It holds the run's `transcript`, the turns the run has
staged, its call records, its reservations, its deliveries and the text its buffers hold. The
application includes it in its versioned host state, and resume refuses another version.

The port waits for no job and lists no memory artifacts: the application runs the compaction
flow when it chooses and reads its own records. The kit never commits a turn. Staged turns
become committed when the application's durable commit of the activation succeeds; a failed or
cancelled activation leaves them uncommitted.

## Nodes

`ConversationChat` builds one `EngineTextStreamRequest` from `system`, the `history` turns and
`prompt`, with the run's tool plan and the node's `parameters` for `EngineChatParameters`. The
round limit is `max_tool_rounds` capped by the plan and clamped to the range the adapters
accept, as `EngineChatParameters::max_tool_rounds` clamps it. It counts the request's input
tokens with `count_text_stream_input_tokens`; an empty request measures zero without a count.
A count above `budget`
refuses dispatch with the recoverable failure `conversation.over_budget`, fires `over_budget`,
records the call and publishes a `status` with that outcome. It then reserves spend; a refused
reservation is `conversation.spend_refused`. It streams through `start_provider`: each
`TextDelta` is a `chat.raw` delta on `text`, each `ReasoningDelta` one on `reasoning`, tool
events accumulate `tool_rounds`, and the stream's final text is the assistant `reply`. Token
usage settles the reservation and the call is recorded with its status before the node's
outputs close. The activation token cancels the stream; cancellation retains its control
outcome, records no call and leaves the reservation open for the application to settle. A
provider error ends the call with the failed status published and the reservation settled with
the usage reported: `conversation.tool_rounds_exhausted` where the model crate names exhausted
tool rounds, `conversation.provider_failed` otherwise. The error's tool-call evidence, where it
carries any, is attached to the call it names in `tool_rounds`.

`ConversationMemory` fits turns newest first until adding one would exceed `budget`, measured by
the port's engine as it measures the chat request. The summary is not counted. An assistant turn
that calls tools and the turns answering its calls, in the tool or the user role, are one unit,
kept or dropped together, so the history is a request `validate_messages` accepts.

`ConversationBuffer` holds at most `retain` bytes of collected text and refuses a stream that
exceeds it with the runtime's `capacity_exhausted`. `ConversationTranscriptOutput` stages turns in
delivery order; a `conversation.turns` value stages each turn in turn.
`ConversationCompactionRequest` measures the turns it receives with the transcript's memory and
evaluates with the `after_run` trigger. `ConversationCompactionParser` reads the response as the
summary's text, covering the turns it was given through the last, and names the job in the
activation's `job` value, if any. `ConversationMemoryOutput` writes the summary and transitions
the summary's job to `completed`; a job the port no longer knows is `conversation.unknown_job`.
`ConversationCompactionParser` and `ConversationRefinementParser` refuse a response without text
as `conversation.unparsable_response` and a turn without an id as `conversation.unknown_turn`.
`ConversationRefinementSource` emits the committed turn the activation's `turn` value names.
`ConversationRefinementOutput` refuses a revision of a turn the transcript does not hold as
`conversation.unknown_turn`.

The codes above are recoverable node failures: classified, with external effects settled, so
`emit` and `skip` apply to them. Cancellation, capacity and contract violations retain their
control outcomes.

## Compaction

`compaction::Policy` is `max_turns`, `max_tokens` and `idle`, each optional;
`ConversationCompactionRequest`'s parameters override the first two.
`compaction::evaluate(policy, trigger, memory, measure)` is pure. With the `after_run` trigger
it decides `compact` when the turns after the summary exceed `max_turns` or their measured
tokens exceed `max_tokens`. With the `idle` trigger it decides `compact` when any turn follows
the summary. Otherwise it decides `hold`. The evaluation records the trigger and what it
measured.

`compaction::Runner<H: Port>` holds the port and its clock. `after_run(state, transcript)`
evaluates with `after_run` once a run completes and requests a job on `compact`. Where the
policy sets `idle`, it then arms a timer for `idle` on the clock; a run on the transcript
re-arms it, and a run under a policy without `idle` disarms it. `idle_elapsed()` completes when a timer elapses, naming the transcript, and
`idle(state, transcript)` then evaluates once with `idle`, holding under a policy without
`idle`. The runner requests at most one job
per transcript until the application reports the job ended with `job_ended`, which it does when
its port transitions the job to `completed` or `failed`. A transcript is held while its
evaluation runs and released when the evaluation holds, fails or is dropped before it ends. The runner never activates a flow: the
application runs the compaction flow for a requested job, and that flow's
`ConversationMemoryOutput` completes it.

## Reference host

`reference::State` holds transcripts and their turns, staged turns by run, summaries, jobs, a
spend ledger, call records, deliveries and a scripted `EngineAdapter`. The script is a list of
responses, each a text or a tool-call batch, consumed in order; token counts are the request's
text length in words. The reference host commits a run's staged turns when its activation
completes, without durability, and refuses an unknown transcript, job or turn.

## Model-facing text

The manifest holds three formatter blocks: `conversation.turn` renders one turn as its role and
text, `conversation.transcript` renders turns in order, and `conversation.memory` renders the
summary followed by the turns after it. `ConversationTranscript.text` and
`ConversationMemory.context` reference them. The instructions a compaction or refinement flow
gives the model are the application's prompts, not the kit's.

## Conformance

Cases seed the transcript and script the engine through `conformance::Probe`, a trait behind
the `conformance` feature that a host implements for the suite beside `Port`:

```rust
pub trait Probe: Port {
    /// The extension the suite's seeded records carry.
    fn extension(&self) -> Self::Extension;
    /// Creates `transcript` holding `turns` committed and `summary` as its memory.
    async fn seed(&self, state: &mut Self::State, transcript: TranscriptId, turns: Vec<Turn<Self::Extension>>, summary: Option<Summary<Self::Extension>>) -> Result<(), Self::Error>;
    /// Creates `job` as the port holds it, under its id.
    async fn seed_job(&self, state: &mut Self::State, job: CompactionJob<Self::Extension>) -> Result<(), Self::Error>;
    /// Installs the responses the engine returns, in order.
    async fn script(&self, state: &mut Self::State, responses: Vec<ScriptedResponse>) -> Result<(), Self::Error>;
    /// Offers `plan` to every chat node of the run.
    async fn offer_tools(&self, state: &mut Self::State, plan: ToolPlan) -> Result<(), Self::Error>;
    /// The observable state: the transcripts' turns and summaries, the run's staged turns,
    /// jobs, call records, open reservations and deliveries.
    async fn snapshot(&self, state: &Self::State) -> Result<Snapshot<Self::Extension>, Self::Error>;
}
```

The kit implements `PortAccess<H: Probe>` once over these operations: a case's `Seed`, `Job`,
`Script` and `Tools` operations run before the activation, and `snapshot` is compared after
it, each turn, job and reservation id replaced by its order of appearance and the extensions
removed. The reference host and the example host implement `Probe`; an application implements
it on its test host. A `ScriptedResponse` is a text, a reasoning delta, a tool-call batch or a
wait on the clock; the suite's `Tools` operation answers every call with `Echo`, which returns
the call's arguments. A case may cancel the activation at the wait after its last clock step.

| Case | Proves |
|---|---|
| `prompt` | The prompt source emits the activation value and a user turn; the static prompt emits its parameter |
| `transcript` | The transcript source emits seeded turns in order and renders them |
| `memory-budget` | Memory emits the summary and the newest turns that fit the budget; a budget that falls between a tool call and its result drops both |
| `chat-stream` | Text and reasoning deltas, the reply, the status and a settled reservation |
| `chat-tools` | One tool round through the plan's executor; `tool_rounds` carries its evidence |
| `chat-over-budget` | No dispatch, `over_budget` fires, the status says so, nothing is reserved |
| `chat-cancelled` | Cancelling the activation while the engine waits on the clock stops the stream with the cancelled outcome; no reply or status is published and the reservation stays open |
| `buffer-overflow` | A stream beyond `retain` fails the buffer with the runtime's capacity code; `chat-stream` collects one within it |
| `stage` | Turns are staged in delivery order and committed on completion |
| `compaction-request` | A transcript over the node's `max_turns` requests one job; under it, `held` fires |
| `memory-output` | The summary is written, naming the activation's `job`, and that job completes |
| `refine` | The named turn is emitted, the revision parsed and applied, and `revision` increments |

The example host keeps transcripts in files under a directory the test owns, one file per
transcript, and runs the suite against them.

## Boundary

Battersea owns the conversation nodes and compaction over the port. Applications keep
sessions, storage and durable commits, with the story memory and media the
[execution contract](execution.md) places under Application.

## Verification

| Clause | Distinguishing proof |
|---|---|
| Names | `KIT.check` passes; a handler id, token type or class name outside `conversation` is refused |
| Turn identity | A turn emitted by the prompt source has no id; the staged turn has the id the port assigned |
| Budget | A request one token over `budget` is not dispatched; one at the budget is |
| Spend | Usage from the stream settles the reservation; a refused reservation dispatches nothing |
| Cancellation | Cancelling mid-stream ends the activation cancelled with no call recorded, the reservation open and no reply staged |
| Tool rounds | A scripted tool call runs through the plan's executor; the limit yields `conversation.tool_rounds_exhausted` |
| Staging | A failed activation commits nothing on the reference host; a completed one commits in order |
| Memory | The summary is not counted against `budget`; turns are fitted newest first; a tool call and its results leave together |
| Compaction | `evaluate` decides from the policy and trigger alone; the runner requests one job per transcript until `job_ended` |
| Idle | Advancing the clock past `idle` compacts a transcript under both size thresholds; a run before that re-arms the timer; an idle-only policy compacts on no other trigger |
| Refinement | A revision increments `revision` and leaves the turn's id; an unknown turn is refused |
| Model-facing text | `model_facing_text` has the three blocks and the two references; changing a block changes it |
| Conformance | The suite reaches a host through `Port` and `Probe` only; it passes on the reference host and the file-backed example; a port that drops a staged turn fails with `stage` named |
