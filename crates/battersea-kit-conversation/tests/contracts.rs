//! Distinguishing cases from contracts/kit-conversation.md: names, model-facing text, the
//! records, the port on the reference host and the compaction evaluator.
use battersea_flow::registry::Registry;
use battersea_kit::{
    conformance::{play, Outcome, Played, EPOCH_MS},
    Clock, KitError, ManualClock, ReferenceHost,
};
use battersea_kit_conversation::{
    codes,
    compaction::{evaluate, Decision, Policy, Runner, Trigger},
    conformance::{suite, Access, Probe, ReferenceFixture, ScriptedResponse},
    reference::{self, ScriptedEngine, State},
    CallOutcome, CallRecord, CallStatus, JobState, Memory, Options, Port, Revision, RunState,
    SpendEstimate, Summary, TranscriptId, Turn, KIT, MANIFEST,
};
use battersea_model::{
    EngineAdapter, EngineAdapterRequest, EngineLocalToolCall, EngineLocalToolExecutor,
    EngineLocalToolResult, EngineOperation, EngineTextStreamEvent, EngineTextStreamRequest,
    EngineTokenUsage, Message, Role,
};
use battersea_runtime::{ActivationHost, ExecutionError, RegistryBuilder, SchedulerState};
use futures_util::{FutureExt, StreamExt};
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
fn an_extension_schema_keeps_its_local_references_when_embedded() {
    let mut registry = Registry::default();
    battersea_kit_conversation::register_token_types(
        &mut registry,
        &Options {
            extension_schema: json!({
                "type": "object",
                "properties": {"author": {"$ref": "#/$defs/Author"}},
                "required": ["author"],
                "$defs": {"Author": {"type": "string", "minLength": 1}}
            }),
        },
    )
    .unwrap();
    let turn = |author: &str| {
        serde_json::to_value(Turn {
            extension: json!({"author": author}),
            ..Turn::text(Role::User, "hello", Value::Null)
        })
        .unwrap()
    };
    registry
        .validate_token("conversation.turn", &turn("ada"))
        .unwrap();
    assert!(registry
        .validate_token("conversation.turn", &turn(""))
        .is_err());
    registry
        .validate_token("conversation.turns", &json!([turn("ada")]))
        .unwrap();
    assert!(registry
        .validate_token("conversation.turns", &json!([turn("")]))
        .is_err());
}

#[test]
fn an_extension_schema_keeps_its_anchors_when_embedded() {
    let mut registry = Registry::default();
    battersea_kit_conversation::register_token_types(
        &mut registry,
        &Options {
            extension_schema: json!({
                "type": "object",
                "properties": {"author": {"$ref": "#Author"}},
                "required": ["author"],
                "$defs": {"Author": {"$anchor": "Author", "type": "string", "minLength": 1}}
            }),
        },
    )
    .unwrap();
    let turn = |author: &str| {
        serde_json::to_value(Turn {
            extension: json!({"author": author}),
            ..Turn::text(Role::User, "hello", Value::Null)
        })
        .unwrap()
    };
    registry
        .validate_token("conversation.turn", &turn("ada"))
        .unwrap();
    assert!(registry
        .validate_token("conversation.turn", &turn(""))
        .is_err());
}

#[test]
fn a_subschema_with_its_own_id_keeps_its_references_when_embedded() {
    let branch = json!({
        "$id": "https://example.test/author",
        "type": "object",
        "properties": {"author": {"$ref": "#/$defs/Author"}},
        "required": ["author"],
        "$defs": {"Author": {"type": "string", "minLength": 1}}
    });
    let schemas = battersea_kit_conversation::token_schemas(&json!({"anyOf": [branch]}));
    let turn = schemas
        .iter()
        .find(|(name, _)| *name == "conversation.turn")
        .map(|(_, schema)| schema)
        .unwrap();
    assert_eq!(
        turn["properties"]["extension"]["anyOf"][0]["properties"]["author"]["$ref"],
        "#/$defs/Author"
    );
    let mut registry = Registry::default();
    battersea_kit_conversation::register_token_types(
        &mut registry,
        &Options {
            extension_schema: json!({"anyOf": [branch]}),
        },
    )
    .unwrap();
    let turn = |author: &str| {
        serde_json::to_value(Turn {
            extension: json!({"author": author}),
            ..Turn::text(Role::User, "hello", Value::Null)
        })
        .unwrap()
    };
    registry
        .validate_token("conversation.turn", &turn("ada"))
        .unwrap();
    assert!(registry
        .validate_token("conversation.turn", &turn(""))
        .is_err());
}

#[test]
fn a_comparable_snapshot_renames_ids_and_keeps_content() {
    use battersea_kit_conversation::conformance::{Snapshot, TranscriptSnapshot};
    let snapshot = |id: &str, extension: Value| {
        let mut turn = Turn::text(Role::Assistant, "turn-1", extension);
        turn.id = Some(id.into());
        turn.content.push(battersea_model::ContentBlock::ToolCall {
            id: "call".into(),
            name: "echo".into(),
            arguments: json!({"extension": "kept"}),
        });
        Snapshot {
            transcripts: [(
                "t".into(),
                TranscriptSnapshot {
                    turns: vec![turn],
                    summary: Some(Summary {
                        text: "so far".into(),
                        through: id.into(),
                        job: None,
                        extension: Value::Null,
                    }),
                },
            )]
            .into_iter()
            .collect(),
            staged: vec![],
            jobs: vec![],
            calls: vec![],
            reservations: vec![],
            deliveries: vec![],
        }
        .comparable()
    };
    assert_eq!(snapshot("turn-1", json!(1)), snapshot("abc", json!(2)));
    let value = snapshot("turn-1", Value::Null);
    assert_eq!(value["transcripts"]["t"]["turns"][0]["id"], "turn#0");
    assert_eq!(value["transcripts"]["t"]["summary"]["through"], "turn#0");
    assert_eq!(
        value["transcripts"]["t"]["turns"][0]["content"][0]["text"],
        "turn-1"
    );
    assert_eq!(
        value["transcripts"]["t"]["turns"][0]["content"][1]["arguments"]["extension"],
        "kept"
    );
    assert!(value["transcripts"]["t"]["turns"][0]
        .get("extension")
        .is_none());
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
    host.seed(
        &mut state,
        "seeded".into(),
        vec![Turn {
            id: Some("turn-7".into()),
            ..Turn::text(Role::User, "seeded", Value::Null)
        }],
        None,
    )
    .await
    .unwrap();
    let turn = Turn::text(Role::User, "hello", Value::Null);
    assert_eq!(turn.id, None);
    clock.advance(std::time::Duration::from_millis(5));
    let id = host.stage(&mut state, turn).await.unwrap();
    assert_ne!(id, "turn-7".into());
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
    host.seed(
        &mut state,
        "other".into(),
        vec![Turn::text(Role::User, "elsewhere", Value::Null)],
        None,
    )
    .await
    .unwrap();
    let elsewhere = host.turns(&state, &"other".into()).await.unwrap()[0]
        .id
        .clone()
        .unwrap();
    for turn in ["missing".into(), elsewhere] {
        assert!(matches!(
            host.revise(
                &mut state,
                Revision {
                    turn,
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

/// An executor that fails every call, with the evidence it is given.
struct Failing(Option<battersea_model::ToolCallEvidence>);
#[async_trait::async_trait]
impl EngineLocalToolExecutor for Failing {
    async fn call(
        &self,
        _call: EngineLocalToolCall,
    ) -> Result<EngineLocalToolResult, battersea_model::EngineAdapterRequestError> {
        let error = battersea_model::EngineAdapterRequestError::new(
            "test",
            "the tool timed out",
            battersea_model::ErrorKind::Timeout,
        );
        Err(match &self.0 {
            Some(evidence) => error.with_tool_call(evidence.clone()),
            None => error,
        })
    }
}

#[tokio::test]
async fn a_failed_scripted_tool_call_keeps_the_executors_evidence_or_is_uncertain() {
    use battersea_model::{ToolCallEvidence, ToolCallOutcome};
    let clock = Arc::new(ManualClock::new(EPOCH_MS));
    let engine = ScriptedEngine::new(clock.clone() as Arc<dyn Clock>);
    let call = EngineLocalToolCall {
        id: "call-1".into(),
        name: "slow".into(),
        arguments: json!({}),
    };
    let supplied = ToolCallEvidence {
        call_id: "call-1".into(),
        tool_name: "slow".into(),
        outcome: ToolCallOutcome::NotDispatched,
        request_id: Some("req-9".into()),
    };
    let completed = |executor: Arc<dyn EngineLocalToolExecutor>| {
        let engine = &engine;
        let call = call.clone();
        async move {
            engine.extend([
                ScriptedResponse::ToolCalls(vec![call]),
                ScriptedResponse::Text("after".into()),
            ]);
            engine
                .stream_text(request("hi", Some(executor)))
                .await
                .unwrap()
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .into_iter()
                .find_map(|event| match event {
                    EngineTextStreamEvent::ToolCallCompleted { ok, tool_call, .. } => {
                        Some((ok, tool_call))
                    }
                    _ => None,
                })
                .unwrap()
        }
    };
    let (ok, evidence) = completed(Arc::new(Failing(Some(supplied.clone())))).await;
    assert!(!ok);
    assert_eq!(evidence, Some(supplied));
    let (ok, evidence) = completed(Arc::new(Failing(None))).await;
    assert!(!ok);
    assert_eq!(
        evidence.map(|evidence| evidence.outcome),
        Some(ToolCallOutcome::Uncertain)
    );
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
        ScriptedResponse::ToolCalls(vec![EngineLocalToolCall {
            id: "call-2".into(),
            name: "echo".into(),
            arguments: json!({"n": 2}),
        }]),
        ScriptedResponse::ToolCalls(vec![]),
    ]);
    let events = engine
        .stream_text(request("hi", Some(Arc::new(Echo))))
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(matches!(
        events.get(2),
        Some(Ok(EngineTextStreamEvent::ToolCallCompleted { id, ok: true, .. })) if id == "call-2"
    ));
    assert!(matches!(events.last(), Some(Err(_))));
    assert_eq!(events.len(), 6);
}

#[tokio::test]
async fn a_scripted_wait_sleeps_on_the_clock() {
    let clock = Arc::new(ManualClock::new(EPOCH_MS));
    let engine = ScriptedEngine::new(clock.clone() as Arc<dyn Clock>);
    engine.extend([
        ScriptedResponse::Wait(std::time::Duration::from_secs(5)),
        ScriptedResponse::Reasoning("hm".into()),
        ScriptedResponse::Text("later".into()),
    ]);
    let mut stream = engine.stream_text(request("hi", None)).await.unwrap();
    assert!(stream.next().now_or_never().is_none());
    assert_eq!(
        clock.pending_timers(),
        vec![std::time::Duration::from_secs(5)]
    );
    clock.advance(std::time::Duration::from_secs(5));
    assert!(matches!(
        stream.next().await,
        Some(Ok(EngineTextStreamEvent::MessageStart { turn_index: 0 }))
    ));
    assert!(matches!(
        stream.next().await,
        Some(Ok(EngineTextStreamEvent::ReasoningDelta { text })) if text == "hm"
    ));
    assert!(matches!(
        stream.next().await,
        Some(Ok(EngineTextStreamEvent::TextDelta { text })) if text == "later"
    ));
}

#[tokio::test]
async fn the_suite_passes_on_the_reference_host() {
    battersea_kit_conversation::conformance::run(&ReferenceFixture)
        .await
        .unwrap();
}

/// Plays the named case on the reference host.
async fn played(name: &str) -> Played {
    let suite = suite();
    let case = suite
        .cases
        .iter()
        .find(|case| case.name == name)
        .unwrap_or_else(|| panic!("case {name}"));
    play(&suite, &ReferenceFixture, case, &Access)
        .await
        .unwrap()
}

/// The values the run emitted on `node`.`port`, in order.
fn emitted(played: &Played, node: &str, port: &str) -> Vec<Value> {
    played
        .events
        .iter()
        .filter(|event| {
            event["kind"] == "token_emit"
                && event["node_id"] == node
                && event["detail"]["sourcePort"] == port
        })
        .map(|event| event["detail"]["value"].clone())
        .collect()
}

/// The signals `node` emitted, in order.
fn signals(played: &Played, node: &str) -> Vec<String> {
    played
        .events
        .iter()
        .filter(|event| event["kind"] == "signal_emit" && event["node_id"] == node)
        .filter_map(|event| event["detail"]["signalPort"].as_str().map(str::to_owned))
        .filter(|signal| signal != "post_activate")
        .collect()
}

fn failed_with(outcome: &Outcome, expected_code: &str, expected_reason: &str) -> bool {
    matches!(outcome, Outcome::Failed { code, reason, .. } if code == expected_code && reason.as_deref() == Some(expected_reason))
}

#[tokio::test]
async fn the_prompt_transcript_and_memory_cases_emit_what_their_rows_say() {
    let prompt = played("prompt").await;
    assert_eq!(prompt.outcome, Outcome::Completed);
    assert_eq!(
        prompt.state["deliveries"],
        json!(["Be brief.", "Hello there"])
    );
    let committed = &prompt.state["transcripts"]["conformance"]["turns"];
    assert_eq!(committed.as_array().unwrap().len(), 1);
    assert_eq!(committed[0]["content"][0]["text"], "Hello there");
    assert_eq!(committed[0]["created_at"], EPOCH_MS);
    assert_eq!(emitted(&prompt, "prompt", "turn")[0].get("id"), None);
    assert_eq!(signals(&prompt, "tap"), ["received"]);

    let transcript = played("transcript").await;
    assert_eq!(transcript.outcome, Outcome::Completed);
    assert_eq!(
        transcript.state["deliveries"],
        json!(["user: Where is the cat?\n\nassistant: On the mat."])
    );

    let memory = played("memory-budget").await;
    assert_eq!(memory.outcome, Outcome::Completed);
    assert_eq!(
        memory.state["deliveries"],
        json!(["Summary of the conversation so far:\nThe cat was found.\n\nuser: short reply"])
    );
    let fitted = &emitted(&memory, "memory", "turns")[0];
    assert_eq!(fitted.as_array().unwrap().len(), 1);
    assert_eq!(fitted[0]["id"], "t5");
}

#[tokio::test]
async fn the_chat_cases_stream_reserve_and_record_as_their_rows_say() {
    let stream = played("chat-stream").await;
    assert_eq!(stream.outcome, Outcome::Completed);
    assert_eq!(stream.state["deliveries"], json!(["In the garden."]));
    assert_eq!(
        emitted(&stream, "chat", "reasoning"),
        [json!("Thinking about dogs.")]
    );
    let call = &stream.state["calls"][0];
    assert_eq!(call["status"]["outcome"], "completed");
    assert_eq!(call["status"]["stop_reason"], "complete");
    assert_eq!(call["status"]["usage"]["input_tokens"], 13);
    assert_eq!(call["reservation"]["estimate"]["max_output_tokens"], 50);
    assert_eq!(stream.state["reservations"], json!([]));
    let committed = &stream.state["transcripts"]["conformance"]["turns"];
    assert_eq!(committed[2]["role"], "assistant");
    assert_eq!(committed[2]["content"][0]["text"], "In the garden.");
    assert_eq!(signals(&stream, "chat"), ["completed"]);

    let tools = played("chat-tools").await;
    assert_eq!(tools.outcome, Outcome::Completed);
    let rounds = &emitted(&tools, "chat", "tool_rounds")[0];
    assert_eq!(rounds[0]["calls"][0]["result"], json!({"text": "kept"}));
    assert_eq!(rounds[0]["calls"][0]["evidence"]["outcome"], "completed");
    assert_eq!(tools.state["calls"][0]["status"]["rounds"], 1);
    assert_eq!(tools.state["deliveries"], json!(["Echoed."]));

    let over = played("chat-over-budget").await;
    assert!(failed_with(
        &over.outcome,
        "invalid_request",
        codes::OVER_BUDGET
    ));
    assert_eq!(over.state["calls"][0]["status"]["outcome"], "over_budget");
    assert_eq!(over.state["calls"][0]["reservation"], Value::Null);
    assert_eq!(over.state["reservations"], json!([]));
    assert_eq!(signals(&over, "chat"), ["over_budget"]);
    assert_eq!(emitted(&over, "chat", "status").len(), 1);
    assert!(emitted(&over, "chat", "reply").is_empty());

    let cancelled = played("chat-cancelled").await;
    assert!(failed_with(
        &cancelled.outcome,
        "cancelled",
        battersea_runtime::error_codes::ACTIVATION_CANCELLED
    ));
    assert_eq!(cancelled.state["calls"], json!([]));
    assert_eq!(cancelled.state["reservations"].as_array().unwrap().len(), 1);
    assert_eq!(
        cancelled.state["transcripts"]["conformance"]["turns"],
        json!([])
    );
    assert!(emitted(&cancelled, "chat", "status").is_empty());

    let overflow = played("buffer-overflow").await;
    assert!(failed_with(
        &overflow.outcome,
        "invalid_request",
        battersea_runtime::error_codes::CAPACITY_EXHAUSTED
    ));
}

#[tokio::test]
async fn the_staging_compaction_and_refinement_cases_change_the_port_as_their_rows_say() {
    let stage = played("stage").await;
    assert_eq!(stage.outcome, Outcome::Completed);
    let texts = stage.state["transcripts"]["conformance"]["turns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|turn| turn["content"][0]["text"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        texts,
        [
            "Where is the cat?",
            "On the mat.",
            "And again",
            "Where is the cat?",
            "On the mat."
        ]
    );

    let compaction = played("compaction-request").await;
    assert_eq!(compaction.outcome, Outcome::Completed);
    assert_eq!(signals(&compaction, "over"), ["requested"]);
    assert_eq!(signals(&compaction, "under"), ["held"]);
    let jobs = compaction.state["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["state"], "requested");
    assert_eq!(jobs[0]["evaluation"]["measured"]["turns"], 3);
    assert_eq!(jobs[0]["evaluation"]["policy"]["max_turns"], 2);

    let output = played("memory-output").await;
    assert_eq!(output.outcome, Outcome::Completed);
    let summary = &output.state["transcripts"]["conformance"]["summary"];
    assert_eq!(summary["text"], "The cat is on the mat.");
    assert_eq!(summary["through"], "turn#1");
    assert_eq!(summary["job"], "job#0");
    assert_eq!(output.state["jobs"][0]["state"], "completed");

    let refine = played("refine").await;
    assert_eq!(refine.outcome, Outcome::Completed);
    assert_eq!(
        refine.state["deliveries"],
        json!(["assistant: On the mat."])
    );
    let revised = &refine.state["transcripts"]["conformance"]["turns"][1];
    assert_eq!(revised["content"][0]["text"], "On the mat, asleep.");
    assert_eq!(revised["revision"], 1);
    assert_eq!(revised["id"], "turn#1");
}

#[tokio::test]
async fn the_runner_requests_one_job_per_transcript_and_evaluates_on_idle() {
    let clock = Arc::new(ManualClock::new(EPOCH_MS));
    let host = Arc::new(host(&clock));
    let runner = Runner::new(host.clone());
    let transcript = TranscriptId::from("transcript");
    let mut state = host.state(
        scheduler("runner"),
        State::new("transcript", clock.clone() as Arc<dyn Clock>),
    );
    host.seed(
        &mut state,
        transcript.clone(),
        vec![
            Turn::text(Role::User, "one", Value::Null),
            Turn::text(Role::Assistant, "two", Value::Null),
        ],
        None,
    )
    .await
    .unwrap();
    state.kit.store().policy = Policy {
        max_turns: Some(1),
        max_tokens: None,
        idle: Some(std::time::Duration::from_secs(60)),
    };
    assert!(runner.idle_elapsed().now_or_never().is_none());
    let first = runner.after_run(&mut state, &transcript).await.unwrap();
    assert!(first.is_some());
    assert_eq!(runner.requested(&transcript), first);
    assert_eq!(
        state.kit.store().jobs[first.as_ref().unwrap()]
            .evaluation
            .trigger,
        Trigger::AfterRun
    );
    assert_eq!(
        runner.after_run(&mut state, &transcript).await.unwrap(),
        None
    );
    runner.job_ended(first.as_ref().unwrap());
    let second = runner.after_run(&mut state, &transcript).await.unwrap();
    assert!(second.is_some() && second != first);
    runner.job_ended(second.as_ref().unwrap());

    // The idle timer is armed by the run and re-armed by the next.
    state.kit.store().policy = Policy {
        max_turns: None,
        max_tokens: None,
        idle: Some(std::time::Duration::from_secs(60)),
    };
    assert_eq!(
        runner.after_run(&mut state, &transcript).await.unwrap(),
        None
    );
    let mut elapsed = Box::pin(runner.idle_elapsed());
    assert!(elapsed.as_mut().now_or_never().is_none());
    assert_eq!(
        clock.pending_timers(),
        vec![std::time::Duration::from_secs(60)]
    );
    clock.advance(std::time::Duration::from_secs(30));
    assert_eq!(
        runner.after_run(&mut state, &transcript).await.unwrap(),
        None
    );
    assert!(elapsed.as_mut().now_or_never().is_none());
    clock.advance(std::time::Duration::from_secs(30));
    assert!(elapsed.as_mut().now_or_never().is_none());
    clock.advance(std::time::Duration::from_secs(30));
    assert_eq!(elapsed.await, transcript);
    let idle = runner.idle(&mut state, &transcript).await.unwrap().unwrap();
    assert_eq!(
        state.kit.store().jobs[&idle].evaluation.trigger,
        Trigger::Idle
    );
    assert_eq!(runner.idle(&mut state, &transcript).await.unwrap(), None);
    runner.job_ended(&idle);

    // A policy that drops idle disarms the timer, and idle holds under it.
    assert_eq!(
        runner.after_run(&mut state, &transcript).await.unwrap(),
        None
    );
    let mut elapsed = Box::pin(runner.idle_elapsed());
    assert!(elapsed.as_mut().now_or_never().is_none());
    state.kit.store().policy = Policy::default();
    assert_eq!(
        runner.after_run(&mut state, &transcript).await.unwrap(),
        None
    );
    clock.advance(std::time::Duration::from_secs(90));
    assert!(elapsed.as_mut().now_or_never().is_none());
    assert_eq!(runner.idle(&mut state, &transcript).await.unwrap(), None);
    drop(elapsed);
    state.kit.store().policy = Policy {
        max_turns: None,
        max_tokens: None,
        idle: Some(std::time::Duration::from_secs(60)),
    };

    // An idle-only policy compacts on no other trigger, and never an empty memory.
    let through = state.kit.store().transcripts[&transcript].turns[1]
        .id
        .clone()
        .unwrap();
    host.write_summary(
        &mut state,
        Summary {
            text: "so far".into(),
            through,
            job: Some(idle),
            extension: Value::Null,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        runner.after_run(&mut state, &transcript).await.unwrap(),
        None
    );
    assert_eq!(runner.idle(&mut state, &transcript).await.unwrap(), None);
}
