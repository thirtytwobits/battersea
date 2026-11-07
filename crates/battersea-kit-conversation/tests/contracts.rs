//! Distinguishing cases from contracts/kit-conversation.md: names, model-facing text, the
//! records, the port on the reference host and the compaction evaluator.
use battersea_flow::registry::Registry;
use battersea_kit::{conformance::EPOCH_MS, Clock, KitError, ManualClock, ReferenceHost};
use battersea_kit_conversation::{
    codes,
    compaction::{evaluate, Decision, Policy, Trigger},
    conformance::{Probe, ScriptedResponse},
    reference::{self, ScriptedEngine, State},
    CallOutcome, CallRecord, CallStatus, JobState, Memory, Options, Port, Revision, RunState,
    SpendEstimate, Summary, Turn, KIT, MANIFEST,
};
use battersea_model::{
    EngineAdapter, EngineAdapterRequest, EngineLocalToolCall, EngineLocalToolExecutor,
    EngineLocalToolResult, EngineOperation, EngineTextStreamEvent, EngineTextStreamRequest,
    EngineTokenUsage, Message, Role,
};
use battersea_runtime::{ActivationHost, ExecutionError, RegistryBuilder, SchedulerState};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};
use tokio_util::sync::CancellationToken;

fn registry() -> Registry {
    let mut registry = Registry::default();
    battersea_kit_conversation::conformance::register_for_suite(&mut registry).unwrap();
    registry
}

#[test]
fn the_kit_keeps_its_names_and_normalises_its_manifest() {
    KIT.check().unwrap();
    let definitions = KIT.definitions(&registry()).unwrap();
    assert_eq!(definitions.len(), 15);
    assert!(definitions
        .iter()
        .all(|definition| definition.class_name.starts_with("Conversation")));
    assert!(matches!(
        battersea_kit::Kit {
            manifest: Box::leak(
                MANIFEST
                    .replace("conversation.prompt", "chat.prompt")
                    .into_boxed_str()
            ),
            ..KIT
        }
        .check(),
        Err(KitError::HandlerId { .. })
    ));
}

#[test]
fn model_facing_text_has_the_three_blocks_and_the_references_to_them() {
    let text = KIT.model_facing_text().unwrap();
    let keys = text.keys().cloned().collect::<Vec<_>>();
    assert_eq!(
        keys,
        [
            "ConversationMemory.context.default",
            "ConversationRefinementSource.text.default",
            "ConversationTranscript.text.default",
            "formatter_blocks.conversation.memory",
            "formatter_blocks.conversation.transcript",
            "formatter_blocks.conversation.turn",
        ]
    );
    assert_eq!(
        text["ConversationTranscript.text.default"],
        text["formatter_blocks.conversation.transcript"]
    );
    let changed = battersea_kit::Kit {
        manifest: Box::leak(
            MANIFEST
                .replace("Summary of the conversation so far", "Earlier")
                .into_boxed_str(),
        ),
        ..KIT
    }
    .model_facing_text()
    .unwrap();
    assert_ne!(
        changed["formatter_blocks.conversation.memory"],
        text["formatter_blocks.conversation.memory"]
    );
    assert_ne!(
        changed["ConversationMemory.context.default"],
        text["ConversationMemory.context.default"]
    );
}

#[test]
fn token_types_embed_the_extension_schema() {
    let mut registry = Registry::default();
    battersea_kit_conversation::register_token_types(
        &mut registry,
        &Options {
            extension_schema: json!({"type": "object", "required": ["author"]}),
        },
    )
    .unwrap();
    let turn = |extension: Value| {
        serde_json::to_value(Turn {
            extension,
            ..Turn::text(Role::User, "hello", Value::Null)
        })
        .unwrap()
    };
    registry
        .validate_token("conversation.turn", &turn(json!({"author": "ada"})))
        .unwrap();
    assert!(registry
        .validate_token("conversation.turn", &turn(json!({})))
        .is_err());
    assert!(registry
        .validate_token("conversation.turns", &json!([turn(json!({}))]))
        .is_err());
    registry
        .validate_token(
            "conversation.summary",
            &json!({"text": "so far", "through": "turn-1", "extension": {"author": "ada"}}),
        )
        .unwrap();
    assert!(registry
        .validate_token(
            "conversation.summary",
            &json!({"text": "so far", "through": "turn-1", "extension": {}}),
        )
        .is_err());
}

#[test]
fn run_state_is_versioned_and_resume_refuses_another_version() {
    let state = RunState::<Value>::new("t".into());
    let versioned = state.to_versioned().unwrap();
    assert_eq!(versioned["version"], RunState::<Value>::STATE_VERSION);
    assert_eq!(
        RunState::<Value>::from_versioned(&versioned).unwrap(),
        state
    );
    let mut other = versioned.clone();
    other["version"] = json!(RunState::<Value>::STATE_VERSION + 1);
    assert!(RunState::<Value>::from_versioned(&other)
        .unwrap_err()
        .contains("version"));
}

fn host(clock: &Arc<ManualClock>) -> ReferenceHost<State> {
    reference::host(RegistryBuilder::default().build(), clock.clone())
}

fn flow(name: &str) -> String {
    let limits = serde_json::to_string(&battersea_flow::FlowExecutionLimits::default()).unwrap();
    format!(
        r#"flow 1 "{name}" {{
    version = 3;
    title = "{name}";
    execution = {{"source_order": ["prompt"], "limits": {limits}}};
    node "prompt" "ConversationStaticPrompt" {{"instance_name": "Prompt", "parameter_values": {{"text": "hello"}}}}
}}"#
    )
}

/// A run state around the kit manifest's catalogue, with no handlers on the host.
fn scheduler(name: &str) -> SchedulerState {
    let mut with_handlers = registry();
    for definition in KIT.definitions(&registry()).unwrap() {
        with_handlers
            .register_handler(&definition.handler_id)
            .unwrap();
    }
    let registry = with_handlers;
    let catalog = battersea_flow::catalog::Catalog::from_manifest(MANIFEST, registry).unwrap();
    let flow = battersea_flow::dsl::parse_flow_dsl(&flow(name)).unwrap();
    SchedulerState::new(name.into(), &flow, catalog.definitions().clone()).unwrap()
}

#[tokio::test]
async fn the_reference_port_stages_turns_and_commits_them_on_completion() {
    let clock = Arc::new(ManualClock::new(EPOCH_MS));
    let host = host(&clock);
    let mut state = host.state(
        scheduler("stage"),
        State::new("transcript", clock.clone() as Arc<dyn Clock>),
    );
    host.seed(&mut state, "transcript".into(), vec![], None)
        .await
        .unwrap();
    assert!(host
        .turns(&state, &"transcript".into())
        .await
        .unwrap()
        .is_empty());
    assert!(matches!(
        host.turns(&state, &"other".into())
            .await
            .unwrap_err()
            .reason_code(),
        Some(codes::UNKNOWN_TRANSCRIPT)
    ));
    let turn = Turn::text(Role::User, "hello", Value::Null);
    assert_eq!(turn.id, None);
    clock.advance(std::time::Duration::from_millis(5));
    let id = host.stage(&mut state, turn).await.unwrap();
    let staged = &<ReferenceHost<State> as Port>::run_state(&mut state).staged;
    assert_eq!(staged[0].id, Some(id.clone()));
    assert_eq!(staged[0].created_at, Some(EPOCH_MS + 5));
    assert!(host
        .turns(&state, &"transcript".into())
        .await
        .unwrap()
        .is_empty());
    reference::commit(&mut state).unwrap();
    let committed = host.turns(&state, &"transcript".into()).await.unwrap();
    assert_eq!(committed.len(), 1);
    assert_eq!(committed[0].id, Some(id.clone()));
    assert!(state.kit.run.staged.is_empty());
    host.revise(
        &mut state,
        Revision {
            turn: id.clone(),
            content: vec![battersea_model::ContentBlock::Text {
                text: "hello again".into(),
            }],
            extension: Value::Null,
        },
    )
    .await
    .unwrap();
    let revised = host.turns(&state, &"transcript".into()).await.unwrap();
    assert_eq!(revised[0].revision, 1);
    assert_eq!(revised[0].text_content(), "hello again");
    assert!(matches!(
        host.revise(
            &mut state,
            Revision {
                turn: "missing".into(),
                content: vec![],
                extension: Value::Null
            }
        )
        .await
        .unwrap_err()
        .reason_code(),
        Some(codes::UNKNOWN_TURN)
    ));
}

#[tokio::test]
async fn a_completed_activation_commits_and_a_failed_one_does_not() {
    let clock = Arc::new(ManualClock::new(EPOCH_MS));
    let host = host(&clock);
    let mut state = host.state(
        scheduler("commit"),
        State::new("transcript", clock.clone() as Arc<dyn Clock>),
    );
    host.stage(&mut state, Turn::text(Role::User, "hello", Value::Null))
        .await
        .unwrap();
    // The static prompt has no handler on this host, so the activation fails.
    assert!(host
        .run_activation(
            &mut state,
            "prompt",
            HashMap::new(),
            CancellationToken::new()
        )
        .await
        .is_err());
    assert_eq!(state.kit.run.staged.len(), 1);
    assert!(state.kit.store().transcripts.is_empty());
}

#[tokio::test]
async fn memory_is_the_summary_and_the_turns_after_it() {
    let clock = Arc::new(ManualClock::new(EPOCH_MS));
    let host = host(&clock);
    let mut state = host.state(
        scheduler("memory"),
        State::new("transcript", clock.clone() as Arc<dyn Clock>),
    );
    let turns = ["one", "two", "three"]
        .map(|text| Turn::text(Role::User, text, Value::Null))
        .to_vec();
    host.seed(&mut state, "transcript".into(), turns, None)
        .await
        .unwrap();
    let all = host.memory(&state, &"transcript".into()).await.unwrap();
    assert_eq!(all.summary, None);
    assert_eq!(all.turns.len(), 3);
    let through = all.turns[1].id.clone().unwrap();
    host.write_summary(
        &mut state,
        Summary {
            text: "one and two".into(),
            through,
            job: None,
            extension: Value::Null,
        },
    )
    .await
    .unwrap();
    let after = host.memory(&state, &"transcript".into()).await.unwrap();
    assert_eq!(
        after.summary.as_ref().map(|s| s.text.as_str()),
        Some("one and two")
    );
    assert_eq!(after.turns.len(), 1);
    assert_eq!(after.turns[0].text_content(), "three");
}

#[tokio::test]
async fn spend_is_reserved_settled_and_refused_beyond_the_budget() {
    let clock = Arc::new(ManualClock::new(EPOCH_MS));
    let host = host(&clock);
    let mut state = host.state(
        scheduler("spend"),
        State::new("transcript", clock.clone() as Arc<dyn Clock>),
    );
    state.kit.store().budget = Some(100);
    let estimate = SpendEstimate {
        input_tokens: 60,
        max_output_tokens: Some(10),
    };
    let reservation = host.reserve(&mut state, "chat", estimate).await.unwrap();
    assert_eq!(
        state.kit.run.reservations,
        std::slice::from_ref(&reservation)
    );
    let refused = host
        .reserve(&mut state, "chat", estimate)
        .await
        .unwrap_err();
    assert_eq!(refused.reason_code(), Some(codes::SPEND_REFUSED));
    let usage = EngineTokenUsage {
        input_tokens: Some(60),
        output_tokens: Some(4),
        ..EngineTokenUsage::default()
    };
    host.settle(&mut state, reservation.clone(), usage)
        .await
        .unwrap();
    assert!(state.kit.run.reservations.is_empty());
    assert_eq!(state.kit.store().ledger, [(reservation.clone(), usage)]);
    assert!(host
        .settle(&mut state, reservation.clone(), usage)
        .await
        .is_err());
    host.record_call(
        &mut state,
        CallRecord {
            node: "chat".into(),
            status: CallStatus {
                outcome: CallOutcome::Completed,
                stop_reason: Some("complete".into()),
                usage,
                rounds: 0,
            },
            reservation: Some(reservation),
            started_at: EPOCH_MS,
            finished_at: EPOCH_MS,
        },
    )
    .await
    .unwrap();
    assert_eq!(state.kit.run.calls.len(), 1);
    host.deliver(&mut state, "done".into()).await.unwrap();
    assert_eq!(state.kit.run.deliveries, ["done"]);
}

#[tokio::test]
async fn compaction_jobs_are_requested_and_transitioned() {
    let clock = Arc::new(ManualClock::new(EPOCH_MS));
    let host = host(&clock);
    let mut state = host.state(
        scheduler("jobs"),
        State::new("transcript", clock.clone() as Arc<dyn Clock>),
    );
    assert!(host.policy(&state, &"transcript".into()).is_err());
    host.seed(&mut state, "transcript".into(), vec![], None)
        .await
        .unwrap();
    state.kit.store().policy = Policy {
        max_turns: Some(2),
        ..Policy::default()
    };
    let policy = host.policy(&state, &"transcript".into()).unwrap();
    let memory = host.memory(&state, &"transcript".into()).await.unwrap();
    let evaluation = evaluate(&policy, Trigger::Idle, &memory, 0);
    assert_eq!(evaluation.decision, Decision::Hold);
    let job = host
        .request_compaction(&mut state, evaluation)
        .await
        .unwrap();
    assert_eq!(state.kit.store().jobs[&job].state, JobState::Requested);
    host.transition(&mut state, &job, JobState::Completed)
        .await
        .unwrap();
    assert_eq!(state.kit.store().jobs[&job].state, JobState::Completed);
    assert_eq!(
        host.transition(&mut state, &"missing".into(), JobState::Failed)
            .await
            .unwrap_err()
            .reason_code(),
        Some(codes::UNKNOWN_JOB)
    );
}

#[test]
fn the_evaluator_decides_from_the_policy_and_the_trigger() {
    let memory = |count: usize| Memory {
        transcript: "t".into(),
        summary: None,
        turns: (0..count)
            .map(|_| Turn::text(Role::User, "x", ()))
            .collect(),
    };
    let policy = Policy {
        max_turns: Some(2),
        max_tokens: Some(50),
        idle: None,
    };
    assert_eq!(
        evaluate(&policy, Trigger::AfterRun, &memory(2), 50).decision,
        Decision::Hold
    );
    assert_eq!(
        evaluate(&policy, Trigger::AfterRun, &memory(3), 0).decision,
        Decision::Compact
    );
    assert_eq!(
        evaluate(&policy, Trigger::AfterRun, &memory(1), 51).decision,
        Decision::Compact
    );
    let idle_only = Policy {
        idle: Some(std::time::Duration::from_secs(60)),
        ..Policy::default()
    };
    assert_eq!(
        evaluate(&idle_only, Trigger::AfterRun, &memory(100), 1_000_000).decision,
        Decision::Hold
    );
    assert_eq!(
        evaluate(&idle_only, Trigger::Idle, &memory(1), 0).decision,
        Decision::Compact
    );
    assert_eq!(
        evaluate(&idle_only, Trigger::Idle, &memory(0), 0).decision,
        Decision::Hold
    );
    let evaluation = evaluate(&policy, Trigger::AfterRun, &memory(3), 7);
    assert_eq!(
        (evaluation.measured.turns, evaluation.measured.tokens),
        (3, 7)
    );
    assert_eq!(evaluation.trigger, Trigger::AfterRun);
    assert_eq!(
        Policy::default()
            .with_overrides(&Policy {
                max_turns: Some(9),
                ..Policy::default()
            })
            .max_turns,
        Some(9)
    );
}

struct Echo;
#[async_trait::async_trait]
impl EngineLocalToolExecutor for Echo {
    async fn call(
        &self,
        call: EngineLocalToolCall,
    ) -> Result<EngineLocalToolResult, battersea_model::EngineAdapterRequestError> {
        Ok(EngineLocalToolResult::json(call.arguments))
    }
}

fn request(
    text: &str,
    executor: Option<Arc<dyn EngineLocalToolExecutor>>,
) -> EngineTextStreamRequest {
    EngineTextStreamRequest {
        mock_response: None,
        shared: EngineAdapterRequest {
            operation: EngineOperation::FlowTextStream,
            messages: vec![Message::text(Role::User, text.into())],
            chat: battersea_model::engine::EngineChatParameters::default_for_provider("reference"),
            debug_rules: None,
        },
        local_tools: vec![],
        local_tool_executor: executor,
        max_tool_rounds: 1,
    }
}

#[tokio::test]
async fn the_scripted_engine_counts_words_and_streams_its_script() {
    let clock = Arc::new(ManualClock::new(EPOCH_MS));
    let engine = ScriptedEngine::new(clock.clone() as Arc<dyn Clock>);
    assert_eq!(
        engine
            .count_text_stream_input_tokens(request("one two three", None))
            .await
            .unwrap(),
        3
    );
    assert!(engine.stream_text(request("hi", None)).await.is_err());
    engine.extend([
        ScriptedResponse::ToolCalls(vec![EngineLocalToolCall {
            id: "call-1".into(),
            name: "echo".into(),
            arguments: json!({"n": 1}),
        }]),
        ScriptedResponse::Text("four five".into()),
    ]);
    let events = engine
        .stream_text(request("one two three", Some(Arc::new(Echo))))
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(matches!(
        &events[2],
        EngineTextStreamEvent::ToolCallCompleted { ok: true, result, .. } if *result == json!({"n": 1})
    ));
    assert!(matches!(
        events.last(),
        Some(EngineTextStreamEvent::TokenUsage { usage }) if usage.input_tokens == Some(3) && usage.output_tokens == Some(2)
    ));
    assert_eq!(engine.remaining(), 0);
    engine.extend([
        ScriptedResponse::ToolCalls(vec![]),
        ScriptedResponse::ToolCalls(vec![]),
    ]);
    assert!(engine
        .stream_text(request("hi", Some(Arc::new(Echo))))
        .await
        .is_err());
}

#[tokio::test]
async fn the_empty_suite_passes_on_the_reference_host() {
    battersea_kit_conversation::conformance::run(
        &battersea_kit_conversation::conformance::ReferenceFixture,
    )
    .await
    .unwrap();
}
