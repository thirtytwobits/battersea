//! Provider-neutral orchestration for engine-local tools.

use super::{
    emit_error, emit_stream_event, EngineAdapterDebugContext, EngineAdapterLogger,
    EngineAdapterRequestError, EngineLocalToolCall, EngineLocalToolExecutor, EngineLocalToolResult,
    EngineTextStreamEvent, EngineTextStreamRequest,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::{collections::HashSet, sync::Arc};
use tokio::sync::mpsc;

pub type ToolEventSender = mpsc::Sender<Result<EngineTextStreamEvent, EngineAdapterRequestError>>;

pub struct ExecutedToolCall {
    pub call: EngineLocalToolCall,
    pub result: EngineLocalToolResult,
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
    let invalid =
        || EngineAdapterRequestError::invalid_response(provider, "Malformed provider tool call.");
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
    // Dropping the consumer cancels both provider waits and local execution,
    // including periods during which neither emits progress events.
    tokio::select! {
        biased;
        _ = events.closed() => {},
        result = async {
            let run = drive_tool_loop(&mut conversation, &request, executor.as_ref(), &logger, &context, &events);
            match context.timeout_ms {
                Some(ms) => tokio::time::timeout(std::time::Duration::from_millis(ms), run).await
                    .map_err(|_| EngineAdapterRequestError::new(&context.provider, "Request deadline exceeded during tool orchestration.", super::error::ErrorKind::Timeout))?,
                None => run.await,
            }
        } => {
            if let Err(error) = result {
                emit_error(logger.as_ref(), &context, request.shared.operation, &error).await;
                let _ = events.send(Err(error)).await;
            }
        }
    }
}

async fn forward_turn_event(
    events: &ToolEventSender,
    event: Result<EngineTextStreamEvent, EngineAdapterRequestError>,
    turn_index: u32,
) -> Result<(), EngineAdapterRequestError> {
    let mut event = event?;
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
        super::payload::check_payload(&calls, &context.provider)?;
        if calls.is_empty() {
            return Ok(());
        }
        if request.shared.chat.tool_choice.mode == crate::engine::EngineChatToolChoiceMode::None {
            return Err(EngineAdapterRequestError::invalid_response(
                &context.provider,
                "Provider requested tools when tool use was disabled.",
            ));
        }
        // Validate the complete batch before permitting any effects.
        let mut ids = HashSet::new();
        for call in &calls {
            if call.id.is_empty() || !ids.insert(&call.id) {
                return Err(EngineAdapterRequestError::invalid_response(
                    &context.provider,
                    "Provider returned missing or duplicate tool call identifiers.",
                ));
            }
            if !request
                .local_tools
                .iter()
                .any(|tool| super::local_tool_name(&tool.name) == call.name)
            {
                return Err(EngineAdapterRequestError::invalid_response(
                    &context.provider,
                    format!("Provider requested an unoffered tool: {}.", call.name),
                ));
            }
            if request.shared.chat.tool_choice.mode == crate::engine::EngineChatToolChoiceMode::Tool
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
                ));
            }
            if !call.arguments.is_object() {
                return Err(EngineAdapterRequestError::invalid_response(
                    &context.provider,
                    format!("Tool arguments must be an object: {}.", call.name),
                ));
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
                })?;
            validator.validate(&call.arguments).map_err(|error| {
                EngineAdapterRequestError::invalid_response(&context.provider, error.to_string())
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
            let result = executor.call(call.clone()).await;
            if let Ok(result) = &result {
                retained_tool_bytes = retained_tool_bytes.saturating_add(
                    super::payload::check_payload(&(&call, &result.content), &context.provider)?,
                );
                if retained_tool_bytes > super::payload::PROVIDER_PAYLOAD_BYTES {
                    return Err(EngineAdapterRequestError::invalid_response(
                        &context.provider,
                        "Tool transcript exceeds its encoded-byte limit.",
                    ));
                }
            }
            let (content, message, classification) = match &result {
                Ok(result) => (result.content.clone(), None, None),
                Err(error) => (
                    Value::Null,
                    Some(error.message.clone()),
                    Some(error.classification.to_string()),
                ),
            };
            emit_stream_event(logger.as_ref(), context, operation,
                if result.is_ok() { "local_tool_result_returned" } else { "local_tool_error" },
                if result.is_ok() { json!({"id": call.id, "name": call.name, "result": content}) }
                else { json!({"id": call.id, "name": call.name, "message": message, "classification": classification}) }
            ).await;
            send_event(
                events,
                EngineTextStreamEvent::ToolCallCompleted {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    ok: result.is_ok(),
                    result: content,
                    error_message: message,
                    error_classification: classification,
                    finished_at: chrono::Utc::now().to_rfc3339(),
                },
            )
            .await?;
            results.push(ExecutedToolCall {
                call,
                result: result?,
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
    ))
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
            Ok(EngineLocalToolResult {
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
        executor: Arc<Executor>,
    ) -> Vec<Result<EngineTextStreamEvent, EngineAdapterRequestError>> {
        let (tx, mut rx) = mpsc::channel(64);
        run_tool_loop(script, request, executor, None, context(), tx).await;
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            // These tests specify tool effects and lifecycle ordering. Usage
            // scopes have independent accounting coverage below.
            if !matches!(event, Ok(EngineTextStreamEvent::TokenUsage { .. })) {
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
            assert_eq!(results[index].result.content, call.arguments);
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
        for invalid in [
            EngineLocalToolCall {
                name: "unoffered".into(),
                ..call("bad")
            },
            EngineLocalToolCall {
                arguments: json!("broken"),
                ..call("bad")
            },
            call("first"),
        ] {
            let executor = Arc::new(Executor::default());
            let script = Script {
                turns: vec![vec![call("first"), invalid]],
                received: Default::default(),
            };
            let events = run(script, request(), executor.clone()).await;
            assert!(executor.calls.lock().unwrap().is_empty());
            assert!(
                matches!(&events[..], [Err(error)] if error.classification.as_str() == "invalid_response")
            );
        }
    }
    #[tokio::test]
    async fn enforces_disabled_and_named_tool_policy_before_execution() {
        for mode in [
            crate::engine::EngineChatToolChoiceMode::None,
            crate::engine::EngineChatToolChoiceMode::Tool,
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
                matches!(&events[..], [Err(error)] if error.classification.as_str() == "invalid_response")
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
        assert!(!events.iter().any(|event| matches!(
            event,
            Ok(EngineTextStreamEvent::ToolCallCompleted { ok: true, .. })
        )));
    }
}
