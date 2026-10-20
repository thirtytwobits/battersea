//! Provider-neutral orchestration for engine-local tools.

use super::{
    emit_error, emit_stream_event, error::DispatchState, EngineAdapterDebugContext,
    EngineAdapterLogger, EngineAdapterRequestError, EngineLocalToolCall, EngineLocalToolExecutor,
    EngineLocalToolResult, EngineTextStreamEvent, EngineTextStreamRequest,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::{collections::HashSet, sync::Arc};
use tokio::sync::mpsc;

pub type ToolEventSender = mpsc::Sender<Result<EngineTextStreamEvent, EngineAdapterRequestError>>;

pub struct ExecutedToolCall {
    pub call: EngineLocalToolCall,
    pub result: EngineLocalToolResult,
    /// Provider representation, excluding host-only metadata.
    pub provider_content: Value,
}

/// The adapter owns the provider transcript and transport. An empty call list
/// completes the run. A turn must finish decoding successfully before any of
/// its calls can execute; partial arguments never cross this boundary.
// async-trait adds must_use to boxed futures, which already carry it.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait ToolConversation: Send {
    async fn next_turn(
        &mut self,
        request: &EngineTextStreamRequest,
        results: Vec<ExecutedToolCall>,
        events: &ToolEventSender,
    ) -> Result<Vec<EngineLocalToolCall>, EngineAdapterRequestError>;
}

pub fn decode_tool_call(
    provider: &str,
    id: Option<&Value>,
    name: Option<&Value>,
    arguments: Option<Value>,
) -> Result<EngineLocalToolCall, EngineAdapterRequestError> {
    let invalid = || {
        EngineAdapterRequestError::invalid_response(provider, "Malformed provider tool call.")
            .with_tool_code(crate::ToolCode::MalformedToolCall)
    };
    let id = id
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(invalid)?;
    let name = name
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(invalid)?;
    let arguments = arguments.filter(Value::is_object).ok_or_else(invalid)?;
    Ok(EngineLocalToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments,
    })
}

pub async fn send_event(
    events: &ToolEventSender,
    event: EngineTextStreamEvent,
) -> Result<(), EngineAdapterRequestError> {
    super::payload::check_payload(&event, "tools")?;
    events.send(Ok(event)).await.map_err(|_| {
        EngineAdapterRequestError::new("engine", "Tool stream consumer disconnected.", "cancelled")
    })
}

pub async fn run_tool_loop(
    mut conversation: impl ToolConversation,
    request: EngineTextStreamRequest,
    executor: Arc<dyn EngineLocalToolExecutor>,
    logger: Option<Arc<dyn EngineAdapterLogger>>,
    context: EngineAdapterDebugContext,
    events: ToolEventSender,
) {
    let mut active_tool = None;
    // Dropping the consumer cancels both provider waits and local execution,
    // including periods during which neither emits progress events.
    tokio::select! {
        biased;
        _ = events.closed() => {},
        result = async {
            let run = drive_tool_loop(&mut conversation, &request, executor.as_ref(), &logger, &context, &events, &mut active_tool);
            match context.timeout_ms {
                Some(ms) => tokio::time::timeout(std::time::Duration::from_millis(ms), run).await
                    .map_err(|_| {
                        let mut error = EngineAdapterRequestError::new(&context.provider, "Request deadline exceeded during tool orchestration.", super::error::ErrorKind::Timeout);
                        error.tool_call = active_tool.clone().map(Box::new);
                        error
                    })?,
                None => run.await,
            }
        } => {
            if let Err(mut error) = result {
                if error.tool_call.is_none() {
                    error.tool_call = active_tool.clone().map(Box::new);
                }
                emit_error(logger.as_ref(), &context, request.shared.operation, &error).await;
                let _ = events.send(Err(error)).await;
            }
        }
    }
}

/// A local tool fails after the provider has answered the turn that called it, so its failure
/// cannot report the provider request as not sent.
fn after_provider_turn(error: EngineAdapterRequestError) -> EngineAdapterRequestError {
    if error.dispatch == DispatchState::NotSent {
        error.with_dispatch(DispatchState::Unknown)
    } else {
        error
    }
}

async fn forward_turn_event(
    events: &ToolEventSender,
    event: Result<EngineTextStreamEvent, EngineAdapterRequestError>,
    turn_index: u32,
) -> Result<(), EngineAdapterRequestError> {
    let mut event = event?;
    if matches!(event, EngineTextStreamEvent::MessageStop { .. }) {
        return Ok(());
    }
    if let EngineTextStreamEvent::TokenUsage { usage } = &mut event {
        usage.turn_index = turn_index;
    }
    send_event(events, event).await
}

async fn drive_tool_loop(
    conversation: &mut impl ToolConversation,
    request: &EngineTextStreamRequest,
    executor: &dyn EngineLocalToolExecutor,
    logger: &Option<Arc<dyn EngineAdapterLogger>>,
    context: &EngineAdapterDebugContext,
    events: &ToolEventSender,
    active_tool: &mut Option<crate::ToolCallEvidence>,
) -> Result<(), EngineAdapterRequestError> {
    let operation = request.shared.operation;
    emit_stream_event(
        logger.as_ref(),
        context,
        operation,
        "local_tool_list",
        json!({
            "tools": request.local_tools.iter().map(|tool| &tool.name).collect::<Vec<_>>()
        }),
    )
    .await;
    let mut results = Vec::new();
    let mut retained_tool_bytes = 0usize;
    for turn in 0..request.max_tool_rounds {
        let turn = u32::try_from(turn).map_err(|_| {
            EngineAdapterRequestError::invalid_response(
                &context.provider,
                "Tool turn index overflow.",
            )
        })?;
        send_event(
            events,
            EngineTextStreamEvent::MessageStart { turn_index: turn },
        )
        .await?;
        send_event(
            events,
            EngineTextStreamEvent::TokenUsage {
                usage: super::EngineTokenUsage {
                    turn_index: turn,
                    ..Default::default()
                },
            },
        )
        .await?;
        let (turn_tx, mut turn_rx) = mpsc::channel(1);
        let provider_turn = conversation.next_turn(request, results, &turn_tx);
        tokio::pin!(provider_turn);
        let calls = loop {
            tokio::select! {
                result = &mut provider_turn => {
                    turn_rx.close();
                    while let Some(event) = turn_rx.recv().await { forward_turn_event(events,event,turn).await?; }
                    break result?;
                }
                Some(event) = turn_rx.recv() => { forward_turn_event(events,event,turn).await?; }
            }
        };
        // The provider turn has consumed the preceding results successfully.
        *active_tool = None;
        super::payload::check_payload(&calls, &context.provider).map_err(|error| {
            let mut error = error.with_tool_code(crate::ToolCode::ToolTranscriptTooLarge);
            if let Some(call) = calls.first() {
                error = error.with_tool_call(crate::ToolCallEvidence {
                    call_id: call.id.clone(),
                    tool_name: call.name.clone(),
                    outcome: crate::ToolCallOutcome::NotDispatched,
                    request_id: None,
                });
            }
            error
        })?;
        if calls.is_empty() {
            send_event(
                events,
                EngineTextStreamEvent::MessageStop {
                    reason: super::StopReason::Complete,
                },
            )
            .await?;
            return Ok(());
        }
        if request.shared.chat.tool_choice.mode == crate::engine::EngineChatToolChoiceMode::None {
            return Err(EngineAdapterRequestError::invalid_response(
                &context.provider,
                "Provider requested tools when tool use was disabled.",
            )
            .with_tool_code(crate::ToolCode::ToolUseDisabled)
            .with_tool_call(crate::ToolCallEvidence {
                call_id: calls[0].id.clone(),
                tool_name: calls[0].name.clone(),
                outcome: crate::ToolCallOutcome::NotDispatched,
                request_id: None,
            }));
        }
        // Validate the complete batch before permitting any effects.
        let mut ids = HashSet::new();
        for call in &calls {
            let validation = (|| {
                if call.id.is_empty() || !ids.insert(&call.id) {
                    return Err(EngineAdapterRequestError::invalid_response(
                        &context.provider,
                        "Provider returned missing or duplicate tool call identifiers.",
                    )
                    .with_tool_code(crate::ToolCode::InvalidToolCallId));
                }
                if !request
                    .local_tools
                    .iter()
                    .any(|tool| super::local_tool_name(&tool.name) == call.name)
                {
                    return Err(EngineAdapterRequestError::invalid_response(
                        &context.provider,
                        format!("Provider requested an unoffered tool: {}.", call.name),
                    )
                    .with_tool_code(crate::ToolCode::UnofferedTool));
                }
                if request.shared.chat.tool_choice.mode
                    == crate::engine::EngineChatToolChoiceMode::Tool
                    && request
                        .shared
                        .chat
                        .tool_choice
                        .tool_name
                        .as_deref()
                        .map(super::local_tool_name)
                        .as_deref()
                        != Some(call.name.as_str())
                {
                    return Err(EngineAdapterRequestError::invalid_response(
                        &context.provider,
                        "Provider requested a tool outside the named tool choice.",
                    )
                    .with_tool_code(crate::ToolCode::ToolOutsideChoice));
                }
                if !call.arguments.is_object() {
                    return Err(EngineAdapterRequestError::invalid_response(
                        &context.provider,
                        format!("Tool arguments must be an object: {}.", call.name),
                    )
                    .with_tool_code(crate::ToolCode::InvalidToolArguments));
                }
                let definition = request
                    .local_tools
                    .iter()
                    .find(|tool| super::local_tool_name(&tool.name) == call.name)
                    .expect("offered tool");
                let validator =
                    jsonschema::validator_for(&definition.input_schema).map_err(|error| {
                        EngineAdapterRequestError::new(
                            &context.provider,
                            error.to_string(),
                            super::error::ErrorKind::InvalidRequest,
                        )
                        .with_tool_code(crate::ToolCode::InvalidToolSchema)
                    })?;
                validator.validate(&call.arguments).map_err(|error| {
                    EngineAdapterRequestError::invalid_response(
                        &context.provider,
                        error.to_string(),
                    )
                    .with_tool_code(crate::ToolCode::InvalidToolArguments)
                })?;
                Ok(())
            })();
            validation.map_err(|error: EngineAdapterRequestError| {
                after_provider_turn(error.with_tool_call(crate::ToolCallEvidence {
                    call_id: call.id.clone(),
                    tool_name: call.name.clone(),
                    outcome: crate::ToolCallOutcome::NotDispatched,
                    request_id: None,
                }))
            })?;
        }
        results = Vec::with_capacity(calls.len());
        // Tools may share application state. Execute in provider order.
        for call in calls {
            emit_stream_event(
                logger.as_ref(),
                context,
                operation,
                "local_tool_call_started",
                serde_json::to_value(&call).expect("tool calls serialize"),
            )
            .await;
            send_event(
                events,
                EngineTextStreamEvent::ToolCallStarted {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                    started_at: chrono::Utc::now().to_rfc3339(),
                },
            )
            .await?;
            let mut evidence = crate::ToolCallEvidence {
                call_id: call.id.clone(),
                tool_name: call.name.clone(),
                outcome: crate::ToolCallOutcome::Uncertain,
                request_id: None,
            };
            *active_tool = Some(evidence.clone());
            let result = executor.call(call.clone()).await;
            if result.is_ok() {
                evidence.outcome = crate::ToolCallOutcome::Completed;
            } else if let Err(error) = &result {
                if let Some(retained) = &error.tool_call {
                    evidence = retained.as_ref().clone();
                }
            }
            *active_tool = Some(evidence.clone());
            let mut host_result = None;
            let projected = result
                .and_then(|result| {
                    // Charge host metadata as well as content before cloning or publishing it.
                    retained_tool_bytes = retained_tool_bytes.saturating_add(
                        super::payload::check_payload(&(&call, &result), &context.provider)
                            .map_err(|error| {
                                error.with_tool_code(crate::ToolCode::ToolTranscriptTooLarge)
                            })?,
                    );
                    if retained_tool_bytes > super::payload::PROVIDER_PAYLOAD_BYTES {
                        return Err(EngineAdapterRequestError::invalid_response(
                            &context.provider,
                            "Tool transcript exceeds its encoded-byte limit.",
                        )
                        .with_tool_code(crate::ToolCode::ToolTranscriptTooLarge));
                    }
                    host_result = Some(Box::new(result.clone()));
                    let content = result.provider_content()?;
                    // Both representations remain live until the next provider turn consumes them.
                    retained_tool_bytes = retained_tool_bytes.saturating_add(
                        super::payload::check_payload(&content, &context.provider).map_err(
                            |error| error.with_tool_code(crate::ToolCode::ToolTranscriptTooLarge),
                        )?,
                    );
                    if retained_tool_bytes > super::payload::PROVIDER_PAYLOAD_BYTES {
                        return Err(EngineAdapterRequestError::invalid_response(
                            &context.provider,
                            "Tool transcript exceeds its encoded-byte limit.",
                        )
                        .with_tool_code(crate::ToolCode::ToolTranscriptTooLarge));
                    }
                    Ok((result, content))
                })
                .map_err(|mut error| {
                    if error.tool_call.is_none() {
                        error.tool_call = Some(Box::new(evidence.clone()));
                    }
                    error
                });
            let (content, message, classification, ok) = match &projected {
                Ok((result, content)) => (content.clone(), None, None, !result.is_error()),
                Err(error) => (
                    Value::Null,
                    Some(error.message.to_string()),
                    Some(error.classification.to_string()),
                    false,
                ),
            };
            emit_stream_event(logger.as_ref(), context, operation,
                if projected.is_ok() { "local_tool_result_returned" } else { "local_tool_error" },
                if projected.is_ok() { json!({"id": call.id, "name": call.name, "result": content}) }
                else { json!({"id": call.id, "name": call.name, "message": message, "classification": classification}) }
            ).await;
            send_event(
                events,
                EngineTextStreamEvent::ToolCallCompleted {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    ok,
                    result: content,
                    error_message: message,
                    error_classification: classification,
                    tool_result: host_result,
                    tool_call: Some(evidence.clone()),
                    finished_at: chrono::Utc::now().to_rfc3339(),
                },
            )
            .await
            .map_err(|error| after_provider_turn(error.with_tool_call(evidence)))?;
            let (result, provider_content) = projected.map_err(after_provider_turn)?;
            results.push(ExecutedToolCall {
                call,
                result,
                provider_content,
            });
        }
    }
    emit_stream_event(
        logger.as_ref(),
        context,
        operation,
        "max_tool_rounds_exceeded",
        json!({"max_tool_rounds": request.max_tool_rounds}),
    )
    .await;
    Err(EngineAdapterRequestError::new(
        &context.provider,
        "Maximum local tool rounds exceeded.",
        "request",
    )
    .with_tool_code(crate::ToolCode::ToolRoundsExceeded))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{EngineAdapterRequest, EngineLocalToolDefinition, EngineOperation};
    use crate::engine::EngineChatParameters;
    use std::sync::Mutex;

    struct Script {
        turns: Vec<Vec<EngineLocalToolCall>>,
        received: Arc<Mutex<Vec<ExecutedToolCall>>>,
    }
    #[async_trait]
    impl ToolConversation for Script {
        async fn next_turn(
            &mut self,
            _: &EngineTextStreamRequest,
            results: Vec<ExecutedToolCall>,
            _: &ToolEventSender,
        ) -> Result<Vec<EngineLocalToolCall>, EngineAdapterRequestError> {
            self.received.lock().unwrap().extend(results);
            Ok(self.turns.remove(0))
        }
    }
    #[derive(Default)]
    struct Executor {
        calls: Mutex<Vec<EngineLocalToolCall>>,
        fail: bool,
    }
    #[async_trait]
    impl EngineLocalToolExecutor for Executor {
        async fn call(
            &self,
            call: EngineLocalToolCall,
        ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
            self.calls.lock().unwrap().push(call.clone());
            if self.fail {
                return Err(EngineAdapterRequestError::new(
                    "engine",
                    "Rejected by domain validation",
                    "invalid_request",
                ));
            }
            Ok(EngineLocalToolResult::Json {
                content: call.arguments,
            })
        }
    }
    fn context() -> EngineAdapterDebugContext {
        EngineAdapterDebugContext {
            provider: "test".into(),
            backend: "test".into(),
            model: "test".into(),
            sdk: "test".into(),
            base_url: None,
            timeout_ms: None,
            max_retries: None,
        }
    }
    fn request() -> EngineTextStreamRequest {
        EngineTextStreamRequest {
            mock_response: None,
            shared: EngineAdapterRequest {
                operation: EngineOperation::FlowTextStream,
                messages: vec![
                    crate::Message::text(crate::Role::System, String::new()),
                    crate::Message::text(crate::Role::User, String::new()),
                ],
                chat: EngineChatParameters::default_for_provider("test"),
                debug_rules: None,
            },
            local_tools: vec![EngineLocalToolDefinition {
                name: "lookup".into(),
                description: String::new(),
                input_schema: json!({"type":"object"}),
            }],
            local_tool_executor: None,
            max_tool_rounds: 3,
        }
    }
    fn call(id: &str) -> EngineLocalToolCall {
        EngineLocalToolCall {
            id: id.into(),
            name: "lookup".into(),
            arguments: json!({"key": id}),
        }
    }
    async fn run(
        script: Script,
        request: EngineTextStreamRequest,
        executor: Arc<dyn EngineLocalToolExecutor>,
    ) -> Vec<Result<EngineTextStreamEvent, EngineAdapterRequestError>> {
        let (tx, mut rx) = mpsc::channel(64);
        run_tool_loop(script, request, executor, None, context(), tx).await;
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            // These tests specify tool effects and lifecycle ordering. Usage
            // scopes have independent accounting coverage below.
            if !matches!(
                event,
                Ok(EngineTextStreamEvent::TokenUsage { .. }
                    | EngineTextStreamEvent::MessageStart { .. }
                    | EngineTextStreamEvent::MessageStop { .. })
            ) {
                events.push(event);
            }
        }
        events
    }
    #[tokio::test]
    async fn shares_ordered_execution_results_and_lifecycle_across_turns() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let executor = Arc::new(Executor::default());
        let calls = vec![call("b"), call("a"), call("c")];
        let script = Script {
            turns: vec![calls[..2].to_vec(), calls[2..].to_vec(), vec![]],
            received: received.clone(),
        };
        let events = run(script, request(), executor.clone()).await;
        assert_eq!(*executor.calls.lock().unwrap(), calls);
        let results = received.lock().unwrap();
        assert_eq!(results.len(), calls.len());
        for (index, call) in calls.iter().enumerate() {
            assert_eq!(results[index].call, *call);
            assert_eq!(
                results[index].result,
                EngineLocalToolResult::json(call.arguments.clone())
            );
            assert!(
                matches!(&events[index * 2], Ok(EngineTextStreamEvent::ToolCallStarted { id, .. }) if id == &call.id)
            );
            assert!(
                matches!(&events[index * 2 + 1], Ok(EngineTextStreamEvent::ToolCallCompleted { id, ok: true, result, .. }) if id == &call.id && result == &call.arguments)
            );
        }
        assert_eq!(events.len(), calls.len() * 2);
    }
    #[tokio::test]
    async fn validates_whole_batch_before_any_effect() {
        for (invalid, code) in [
            (
                EngineLocalToolCall {
                    name: "unoffered".into(),
                    ..call("bad")
                },
                crate::ToolCode::UnofferedTool,
            ),
            (
                EngineLocalToolCall {
                    arguments: json!("broken"),
                    ..call("bad")
                },
                crate::ToolCode::InvalidToolArguments,
            ),
            (call("first"), crate::ToolCode::InvalidToolCallId),
        ] {
            let executor = Arc::new(Executor::default());
            let script = Script {
                turns: vec![vec![call("first"), invalid]],
                received: Default::default(),
            };
            let events = run(script, request(), executor.clone()).await;
            assert!(executor.calls.lock().unwrap().is_empty());
            assert!(
                matches!(&events[..], [Err(error)] if error.classification.as_str() == "invalid_response" && error.tool_code == Some(code)),
                "{code}: {events:?}"
            );
        }
    }
    #[tokio::test]
    async fn enforces_disabled_and_named_tool_policy_before_execution() {
        for (mode, code) in [
            (
                crate::engine::EngineChatToolChoiceMode::None,
                crate::ToolCode::ToolUseDisabled,
            ),
            (
                crate::engine::EngineChatToolChoiceMode::Tool,
                crate::ToolCode::ToolOutsideChoice,
            ),
        ] {
            let mut request = request();
            request.shared.chat.tool_choice.mode = mode;
            request.shared.chat.tool_choice.tool_name = Some("other_tool".into());
            let executor = Arc::new(Executor::default());
            let script = Script {
                turns: vec![vec![call("first")]],
                received: Default::default(),
            };
            let events = run(script, request, executor.clone()).await;
            assert!(executor.calls.lock().unwrap().is_empty());
            assert!(
                matches!(&events[..], [Err(error)] if error.classification.as_str() == "invalid_response" && error.tool_code == Some(code)),
                "{code}: {events:?}"
            );
        }
    }

    #[tokio::test]
    async fn executor_failure_finishes_call_then_stops_without_continuation() {
        let executor = Arc::new(Executor {
            fail: true,
            ..Default::default()
        });
        let received = Arc::new(Mutex::new(Vec::new()));
        let script = Script {
            turns: vec![vec![call("first"), call("second")]],
            received: received.clone(),
        };
        let events = run(script, request(), executor.clone()).await;
        assert_eq!(executor.calls.lock().unwrap().len(), 1);
        assert!(received.lock().unwrap().is_empty());
        assert!(
            matches!(&events[..], [Ok(EngineTextStreamEvent::ToolCallStarted { .. }),
            Ok(EngineTextStreamEvent::ToolCallCompleted { ok: false, error_classification: Some(kind), .. }), Err(error)]
            if kind == error.classification.as_str())
        );
        // The provider answered the turn before the executor ran.
        let Some(Err(error)) = events.last() else {
            unreachable!("the run ends in the executor's failure")
        };
        assert_ne!(error.dispatch, crate::DispatchState::NotSent);
        assert_eq!(
            error.tool_call.as_ref().unwrap().outcome,
            crate::ToolCallOutcome::Uncertain
        );
    }

    struct Answer(Mutex<Option<Result<EngineLocalToolResult, EngineAdapterRequestError>>>);
    #[async_trait]
    impl EngineLocalToolExecutor for Answer {
        async fn call(
            &self,
            _: EngineLocalToolCall,
        ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
            self.0.lock().unwrap().take().expect("executed once")
        }
    }

    fn content_result(is_error: bool) -> EngineLocalToolResult {
        EngineLocalToolResult::Content(crate::ToolContentResult {
            content_blocks: vec![crate::ToolContentBlock::Text {
                text: "tool answer".into(),
            }],
            structured_content: Some(json!({"answer": 42})),
            is_error,
            meta: serde_json::Map::from_iter([("host".into(), json!("private-host-value"))]),
        })
    }

    #[derive(Default)]
    struct Logs(Mutex<Vec<Value>>);
    #[async_trait]
    impl EngineAdapterLogger for Logs {
        async fn on_stream_event(&self, entry: super::super::EngineAdapterStreamLog) {
            self.0.lock().unwrap().push(entry.detail);
        }
    }

    #[tokio::test]
    async fn tool_reported_error_continues_with_public_content_and_retains_private_host_result() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let result = content_result(true);
        let script = Script {
            turns: vec![vec![call("first")], vec![]],
            received: received.clone(),
        };
        let logs = Arc::new(Logs::default());
        let (tx, mut rx) = mpsc::channel(64);
        run_tool_loop(
            script,
            request(),
            Arc::new(Answer(Mutex::new(Some(Ok(result.clone()))))),
            Some(logs.clone()),
            context(),
            tx,
        )
        .await;
        let mut completed = false;
        while let Some(event) = rx.recv().await {
            if let EngineTextStreamEvent::ToolCallCompleted {
                ok,
                tool_result,
                tool_call,
                error_message,
                ..
            } = event.unwrap()
            {
                assert!(!ok);
                assert!(error_message.is_none());
                assert_eq!(tool_result.as_deref(), Some(&result));
                assert_eq!(
                    tool_call.unwrap().outcome,
                    crate::ToolCallOutcome::Completed
                );
                completed = true;
            }
        }
        assert!(completed);
        let received = received.lock().unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].result, result);
        assert_eq!(received[0].provider_content["isError"], true);
        assert!(!received[0]
            .provider_content
            .to_string()
            .contains("private-host-value"));
        assert!(!serde_json::to_string(&*logs.0.lock().unwrap())
            .unwrap()
            .contains("private-host-value"));
    }

    #[tokio::test]
    async fn provider_dispatch_classification_preserves_tool_refusal_and_remote_request_identity() {
        let evidence = crate::ToolCallEvidence {
            call_id: "first".into(),
            tool_name: "lookup".into(),
            outcome: crate::ToolCallOutcome::NotDispatched,
            request_id: Some("remote-request".into()),
        };
        let error =
            EngineAdapterRequestError::new("remote", "refused before sending", "invalid_request")
                .with_tool_call(evidence.clone());
        let events = run(
            Script {
                turns: vec![vec![call("first")]],
                received: Default::default(),
            },
            request(),
            Arc::new(Answer(Mutex::new(Some(Err(error))))),
        )
        .await;
        let error = events.last().unwrap().as_ref().unwrap_err();
        assert_eq!(error.dispatch, crate::DispatchState::Unknown);
        assert_eq!(error.tool_call.as_deref(), Some(&evidence));
        let wire = serde_json::to_value(error).unwrap();
        let restored: EngineAdapterRequestError = serde_json::from_value(wire).unwrap();
        assert_eq!(restored.tool_call.as_deref(), Some(&evidence));
    }

    #[tokio::test]
    async fn completed_evidence_survives_provider_delivery_failures_and_deadlines() {
        enum Failure {
            Returned,
            Streamed,
            Deadline,
        }
        struct Delivery {
            calls_returned: bool,
            failure: Failure,
        }
        #[async_trait]
        impl ToolConversation for Delivery {
            async fn next_turn(
                &mut self,
                _: &EngineTextStreamRequest,
                results: Vec<ExecutedToolCall>,
                events: &ToolEventSender,
            ) -> Result<Vec<EngineLocalToolCall>, EngineAdapterRequestError> {
                if !self.calls_returned {
                    self.calls_returned = true;
                    return Ok(vec![call("first"), call("second")]);
                }
                assert_eq!(results.len(), 2);
                assert_eq!(results[1].call.id, "second");
                let error = EngineAdapterRequestError::transport("test", "delivery failed")
                    .with_request_id(Some("provider-request".into()));
                match self.failure {
                    Failure::Returned => Err(error),
                    Failure::Streamed => {
                        events.send(Err(error)).await.unwrap();
                        std::future::pending().await
                    }
                    Failure::Deadline => std::future::pending().await,
                }
            }
        }
        for failure in [Failure::Returned, Failure::Streamed, Failure::Deadline] {
            let deadline = matches!(failure, Failure::Deadline);
            let executor = Arc::new(Executor::default());
            let mut context = context();
            context.timeout_ms = Some(200);
            let (tx, mut rx) = mpsc::channel(64);
            run_tool_loop(
                Delivery {
                    calls_returned: false,
                    failure,
                },
                request(),
                executor.clone(),
                None,
                context,
                tx,
            )
            .await;
            let mut completed = Vec::new();
            let mut failure = None;
            while let Some(event) = rx.recv().await {
                match event {
                    Ok(EngineTextStreamEvent::ToolCallCompleted { id, ok: true, .. }) => {
                        completed.push(id);
                    }
                    Err(error) => failure = Some(error),
                    _ => {}
                }
            }
            assert_eq!(completed, ["first", "second"]);
            assert_eq!(executor.calls.lock().unwrap().len(), 2);
            let error = failure.expect("delivery failed");
            assert_eq!(error.dispatch, DispatchState::Unknown);
            assert_eq!(
                error.classification,
                if deadline {
                    crate::ErrorKind::Timeout
                } else {
                    crate::ErrorKind::Transport
                }
            );
            if !deadline {
                assert_eq!(error.request_id.as_deref(), Some("provider-request"));
            }
            let evidence = error.tool_call.expect("retained completion");
            assert_eq!(evidence.call_id, "second");
            assert_eq!(evidence.outcome, crate::ToolCallOutcome::Completed);
        }
    }

    #[tokio::test]
    async fn oversized_call_batch_retains_refusal_evidence_without_execution() {
        let executor = Arc::new(Executor::default());
        let mut oversized = call("oversized");
        oversized.arguments = json!({
            "key": "x".repeat(super::super::payload::PROVIDER_PAYLOAD_BYTES)
        });
        let events = run(
            Script {
                turns: vec![vec![oversized]],
                received: Default::default(),
            },
            request(),
            executor.clone(),
        )
        .await;
        assert!(executor.calls.lock().unwrap().is_empty());
        assert_eq!(events.len(), 1);
        let error = events[0].as_ref().unwrap_err();
        assert_eq!(
            error.tool_code,
            Some(crate::ToolCode::ToolTranscriptTooLarge)
        );
        let evidence = error.tool_call.as_ref().expect("retained refusal");
        assert_eq!(evidence.call_id, "oversized");
        assert_eq!(evidence.outcome, crate::ToolCallOutcome::NotDispatched);
    }

    #[tokio::test]
    async fn unsupported_content_retains_completion_without_a_provider_continuation() {
        let mut result = content_result(false);
        let EngineLocalToolResult::Content(ref mut content) = result else {
            unreachable!()
        };
        content.content_blocks = vec![crate::ToolContentBlock::Image {
            url: "data:image/png;base64,AA==".into(),
            mime_type: "image/png".into(),
        }];
        let received = Arc::new(Mutex::new(Vec::new()));
        let events = run(
            Script {
                turns: vec![vec![call("first")]],
                received: received.clone(),
            },
            request(),
            Arc::new(Answer(Mutex::new(Some(Ok(result.clone()))))),
        )
        .await;
        assert!(received.lock().unwrap().is_empty());
        assert!(
            matches!(&events[1], Ok(EngineTextStreamEvent::ToolCallCompleted {
            ok: false, tool_result: Some(retained), ..
        }) if retained.as_ref() == &result)
        );
        let error = events.last().unwrap().as_ref().unwrap_err();
        assert_eq!(
            error.tool_code,
            Some(crate::ToolCode::UnsupportedToolContent)
        );
        assert_eq!(
            error.tool_call.as_ref().unwrap().outcome,
            crate::ToolCallOutcome::Completed
        );
    }

    #[tokio::test]
    async fn oversized_host_metadata_is_charged_and_preserves_completion_evidence() {
        let mut result = content_result(false);
        let EngineLocalToolResult::Content(ref mut content) = result else {
            unreachable!()
        };
        content.meta.insert(
            "large".into(),
            Value::String("x".repeat(super::super::payload::PROVIDER_PAYLOAD_BYTES)),
        );
        let events = run(
            Script {
                turns: vec![vec![call("first")]],
                received: Default::default(),
            },
            request(),
            Arc::new(Answer(Mutex::new(Some(Ok(result))))),
        )
        .await;
        let error = events.last().unwrap().as_ref().unwrap_err();
        assert_eq!(
            error.tool_code,
            Some(crate::ToolCode::ToolTranscriptTooLarge)
        );
        assert_eq!(
            error.tool_call.as_ref().unwrap().outcome,
            crate::ToolCallOutcome::Completed
        );
        assert!(matches!(
            &events[1],
            Ok(EngineTextStreamEvent::ToolCallCompleted {
                tool_result: None,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn retained_host_and_provider_representations_share_the_transcript_limit() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let result = EngineLocalToolResult::json(Value::String(
            "x".repeat(super::super::payload::PROVIDER_PAYLOAD_BYTES / 2),
        ));
        let events = run(
            Script {
                turns: vec![vec![call("first")]],
                received: received.clone(),
            },
            request(),
            Arc::new(Answer(Mutex::new(Some(Ok(result))))),
        )
        .await;
        assert!(received.lock().unwrap().is_empty());
        let error = events.last().unwrap().as_ref().unwrap_err();
        assert_eq!(
            error.tool_code,
            Some(crate::ToolCode::ToolTranscriptTooLarge)
        );
        assert_eq!(
            error.tool_call.as_ref().unwrap().outcome,
            crate::ToolCallOutcome::Completed
        );
    }

    #[tokio::test]
    async fn limits_provider_rounds_without_issuing_an_extra_request() {
        let script = Script {
            turns: vec![vec![call("first")]],
            received: Default::default(),
        };
        let mut request = request();
        request.max_tool_rounds = 1;
        let events = run(script, request, Arc::new(Executor::default())).await;
        assert!(events
            .last()
            .unwrap()
            .as_ref()
            .unwrap_err()
            .message
            .contains("Maximum local tool rounds"));
    }
    struct PendingTurn {
        started: Arc<tokio::sync::Notify>,
    }
    #[async_trait]
    impl ToolConversation for PendingTurn {
        async fn next_turn(
            &mut self,
            _: &EngineTextStreamRequest,
            _: Vec<ExecutedToolCall>,
            _: &ToolEventSender,
        ) -> Result<Vec<EngineLocalToolCall>, EngineAdapterRequestError> {
            self.started.notify_one();
            std::future::pending().await
        }
    }
    #[tokio::test]
    async fn consumer_disconnect_cancels_a_silent_provider_turn() {
        let started = Arc::new(tokio::sync::Notify::new());
        let (tx, rx) = mpsc::channel(64);
        let task = tokio::spawn(run_tool_loop(
            PendingTurn {
                started: started.clone(),
            },
            request(),
            Arc::new(Executor::default()),
            None,
            context(),
            tx,
        ));
        started.notified().await;
        drop(rx);
        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("cancelled promptly")
            .unwrap();
    }
    struct PendingExecutor {
        started: Arc<tokio::sync::Notify>,
    }
    #[async_trait]
    impl EngineLocalToolExecutor for PendingExecutor {
        async fn call(
            &self,
            _: EngineLocalToolCall,
        ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
            self.started.notify_one();
            std::future::pending().await
        }
    }
    #[tokio::test]
    async fn consumer_disconnect_cancels_pending_local_execution() {
        let started = Arc::new(tokio::sync::Notify::new());
        let script = Script {
            turns: vec![vec![call("first")]],
            received: Default::default(),
        };
        let (tx, rx) = mpsc::channel(64);
        let task = tokio::spawn(run_tool_loop(
            script,
            request(),
            Arc::new(PendingExecutor {
                started: started.clone(),
            }),
            None,
            context(),
            tx,
        ));
        started.notified().await;
        drop(rx);
        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("cancelled promptly")
            .unwrap();
    }
    #[tokio::test]
    async fn invalid_arguments_in_a_later_call_prevent_all_batch_effects() {
        let executor = Arc::new(Executor::default());
        let mut request = request();
        request.local_tools[0].input_schema =
            json!({"type":"object", "required":["key"], "properties":{"key":{"type":"string"}}});
        let mut invalid = call("second");
        invalid.arguments = json!({"key": 42});
        let script = Script {
            turns: vec![vec![call("first"), invalid]],
            received: Default::default(),
        };
        let events = run(script, request, executor.clone()).await;
        assert!(executor.calls.lock().unwrap().is_empty());
        assert!(events.iter().any(|event| matches!(event, Err(error) if error.classification == super::super::error::ErrorKind::InvalidResponse)));
        let error = events.last().unwrap().as_ref().unwrap_err();
        assert_eq!(error.dispatch, crate::DispatchState::Unknown);
        let evidence = error.tool_call.as_ref().unwrap();
        assert_eq!(evidence.call_id, "second");
        assert_eq!(evidence.outcome, crate::ToolCallOutcome::NotDispatched);
    }
    #[tokio::test]
    async fn configured_deadline_cancels_a_tool_that_never_returns() {
        struct Waiting;
        #[async_trait]
        impl EngineLocalToolExecutor for Waiting {
            async fn call(
                &self,
                _: EngineLocalToolCall,
            ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
                std::future::pending().await
            }
        }
        let script = Script {
            turns: vec![vec![call("first")]],
            received: Default::default(),
        };
        let mut context = context();
        context.timeout_ms = Some(20);
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(run_tool_loop(
            script,
            request(),
            Arc::new(Waiting),
            None,
            context,
            tx,
        ));
        let mut events = Vec::new();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while let Some(event) = rx.recv().await {
                events.push(event);
            }
        })
        .await
        .expect("deadline must terminate the run");
        task.await.unwrap();
        assert!(events.iter().any(|event| matches!(event, Err(error) if error.classification == super::super::error::ErrorKind::Timeout)));
        let evidence = events
            .iter()
            .find_map(|event| event.as_ref().err())
            .unwrap()
            .tool_call
            .as_ref()
            .unwrap();
        assert_eq!(evidence.call_id, "first");
        assert_eq!(evidence.outcome, crate::ToolCallOutcome::Uncertain);
        assert!(!events.iter().any(|event| matches!(
            event,
            Ok(EngineTextStreamEvent::ToolCallCompleted { ok: true, .. })
        )));
    }
}
