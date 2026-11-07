//! Copyright (c) Scott A Dixon
//!
//! `conversation.chat`: one model call. The handler builds the request, checks the budget,
//! reserves spend and registers the stream with the driver; the stream maps the engine's
//! events to deltas and ends with one completion event, which the handler applies: it
//! publishes the reply, the status and the tool rounds, settles the reservation and records
//! the call.
use super::{
    messages as messages_of, now, provider_failed, refuse, request, text_of, token, turns_of,
    unsigned,
};
use crate::{
    codes, CallOutcome, CallRecord, CallStatus, Port, SpendEstimate, ToolCall, ToolRound, Turn,
};
use async_trait::async_trait;
use battersea_flow::{FlowNode, FlowNodeDefinition};
use battersea_model::{
    engine::EngineChatParameters, EngineAdapterRequestError, EngineTextStream,
    EngineTextStreamEvent, EngineTokenUsage, ErrorKind, Message, Role, StopReason, ToolCode,
};
use battersea_runtime::{ExecutionError, NodeHandler, ProviderEvent, Token};
use futures_util::{stream, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use tokio_util::sync::CancellationToken;

pub struct ChatHandler;

/// What the stream reports when it ends: the driver hands it to the handler as the
/// provider's last event.
#[derive(Debug, Serialize, Deserialize)]
struct Completion {
    text: String,
    rounds: Vec<ToolRound>,
    usage: EngineTokenUsage,
    stop_reason: Option<String>,
    started_at: u64,
    failure: Option<EngineAdapterRequestError>,
}

/// The stream's accumulation.
struct Progress {
    stream: EngineTextStream,
    text: String,
    rounds: Vec<ToolRound>,
    open: Vec<ToolCall>,
    usage: EngineTokenUsage,
    stop_reason: Option<String>,
    started_at: u64,
    done: bool,
}
impl Progress {
    fn completion(&mut self, failure: Option<EngineAdapterRequestError>) -> ProviderEvent {
        self.done = true;
        let completion = Completion {
            text: std::mem::take(&mut self.text),
            rounds: std::mem::take(&mut self.rounds),
            usage: self.usage,
            stop_reason: self.stop_reason.take(),
            started_at: self.started_at,
            failure,
        };
        ProviderEvent::Data {
            value: serde_json::to_value(completion).expect("a completion serialises"),
        }
    }
}

/// `usage` added to `total`, field by field; a field neither reports stays unreported.
fn add(total: &mut EngineTokenUsage, usage: EngineTokenUsage) {
    fn sum(total: &mut Option<u64>, next: Option<u64>) {
        if total.is_some() || next.is_some() {
            *total = Some(total.unwrap_or(0) + next.unwrap_or(0));
        }
    }
    total.turn_index = usage.turn_index;
    sum(&mut total.cached_input_tokens, usage.cached_input_tokens);
    sum(
        &mut total.cache_write_input_tokens,
        usage.cache_write_input_tokens,
    );
    sum(
        &mut total.reasoning_output_tokens,
        usage.reasoning_output_tokens,
    );
    sum(&mut total.input_tokens, usage.input_tokens);
    sum(&mut total.output_tokens, usage.output_tokens);
    sum(&mut total.total_tokens, usage.total_tokens);
}

/// The stop reason as the model crate spells it.
fn spelled(reason: StopReason) -> String {
    match reason {
        StopReason::Other(other) => other,
        reason => serde_json::to_value(reason)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default(),
    }
}

fn raw(port: &str, text: String) -> ProviderEvent {
    ProviderEvent::Token {
        port: port.into(),
        value: Token {
            token_type: "chat.raw".into(),
            value: Value::String(text),
        },
    }
}

/// The engine's events as the driver's: each text and reasoning delta on its port, tool
/// events accumulated, and one completion event last.
fn provider_events(
    stream: EngineTextStream,
    started_at: u64,
) -> impl Stream<Item = ProviderEvent> + Send + 'static {
    let progress = Progress {
        stream,
        text: String::new(),
        rounds: vec![],
        open: vec![],
        usage: EngineTokenUsage::default(),
        stop_reason: None,
        started_at,
        done: false,
    };
    stream::unfold(progress, |progress| async move {
        let mut progress = progress;
        if progress.done {
            return None;
        }
        loop {
            let event = match progress.stream.next().await {
                None => progress.completion(None),
                Some(Err(error)) => progress.completion(Some(error)),
                Some(Ok(EngineTextStreamEvent::TextDelta { text })) => {
                    progress.text.push_str(&text);
                    raw("text", text)
                }
                Some(Ok(EngineTextStreamEvent::ReasoningDelta { text })) => raw("reasoning", text),
                Some(Ok(event)) => {
                    match event {
                        EngineTextStreamEvent::ToolCallStarted {
                            id,
                            name,
                            arguments,
                            ..
                        } => progress.open.push(ToolCall {
                            id,
                            name,
                            arguments,
                            ok: false,
                            result: Value::Null,
                            evidence: None,
                        }),
                        EngineTextStreamEvent::ToolCallCompleted {
                            id,
                            name,
                            ok,
                            result,
                            tool_call,
                            ..
                        } => match progress.open.iter_mut().find(|call| call.id == id) {
                            Some(call) => {
                                call.ok = ok;
                                call.result = result;
                                call.evidence = tool_call;
                            }
                            None => progress.open.push(ToolCall {
                                id,
                                name,
                                arguments: Value::Null,
                                ok,
                                result,
                                evidence: tool_call,
                            }),
                        },
                        EngineTextStreamEvent::MessageStop { reason } => {
                            if !progress.open.is_empty() {
                                let round = progress.rounds.len() as u32;
                                progress.rounds.push(ToolRound {
                                    round,
                                    calls: std::mem::take(&mut progress.open),
                                });
                            }
                            progress.stop_reason = Some(spelled(reason));
                        }
                        EngineTextStreamEvent::TokenUsage { usage } => {
                            add(&mut progress.usage, usage)
                        }
                        EngineTextStreamEvent::MessageStart { .. }
                        | EngineTextStreamEvent::ContentBlock { .. }
                        | EngineTextStreamEvent::StructuredOutput { .. }
                        | EngineTextStreamEvent::TextDelta { .. }
                        | EngineTextStreamEvent::ReasoningDelta { .. } => {}
                    }
                    continue;
                }
            };
            return Some((event, progress));
        }
    })
}

/// Settles the node's open reservation, if any, and records the call.
async fn record_call<H: Port>(
    host: &H,
    runtime: &mut H::State,
    node: &str,
    status: CallStatus,
    started_at: u64,
) -> Result<(), H::Error> {
    let reservation = H::run_state(runtime)
        .reservations
        .iter()
        .find(|reservation| reservation.node == node)
        .cloned();
    if let Some(reservation) = &reservation {
        host.settle(runtime, reservation.clone(), status.usage)
            .await?;
    }
    let finished_at = now(host)?;
    host.record_call(
        runtime,
        CallRecord {
            node: node.into(),
            status,
            reservation,
            started_at,
            finished_at,
        },
    )
    .await
}

#[async_trait]
impl<H: Port> NodeHandler<H> for ChatHandler {
    fn handler_id(&self) -> &'static str {
        "conversation.chat"
    }
    async fn execute_node(
        &self,
        host: &H,
        runtime: &mut H::State,
        node: &FlowNode,
        definition: &FlowNodeDefinition,
        _values: Option<&HashMap<String, Value>>,
        cancel: &CancellationToken,
    ) -> Result<(), H::Error> {
        let connected: HashSet<String> = host
            .connected_input_ports(runtime, &node.id)
            .into_iter()
            .collect();
        let mut messages = vec![];
        if connected.contains("system") {
            let system =
                text_of::<H::Error>(&*host.take_flow_input_token(runtime, &node.id, "system")?)?;
            if !system.trim().is_empty() {
                messages.push(Message::text(Role::System, system));
            }
        }
        if connected.contains("history") {
            let history: Vec<Turn<H::Extension>> =
                turns_of(&*host.take_flow_input_token(runtime, &node.id, "history")?)?;
            messages.extend(messages_of(&history));
        }
        let prompt =
            text_of::<H::Error>(&*host.take_flow_input_token(runtime, &node.id, "prompt")?)?;
        messages.push(Message::text(Role::User, prompt));

        let budget = unsigned(node, definition, "budget").unwrap_or(8000);
        let max_output_tokens = unsigned(node, definition, "max_output_tokens").unwrap_or(1024);
        let mut rounds = unsigned(node, definition, "max_tool_rounds").unwrap_or(4) as usize;
        let plan = host.tool_plan(runtime, &node.id)?;
        if plan.max_rounds > 0 {
            rounds = rounds.min(plan.max_rounds);
        }
        let mut chat = EngineChatParameters::default_for_provider(crate::NAME);
        chat.max_output_tokens = Some(u32::try_from(max_output_tokens).unwrap_or(u32::MAX));
        chat.max_tool_rounds = u32::try_from(rounds).unwrap_or(u32::MAX);
        let request = request(messages, chat, &plan, rounds);
        let engine = host.engine(runtime, &node.id)?;
        let started_at = now(host)?;

        let input_tokens = engine
            .count_text_stream_input_tokens(request.clone())
            .await
            .map_err(|error| provider_failed::<H::Error>(&error))?;
        if input_tokens > budget {
            let status = CallStatus {
                outcome: CallOutcome::OverBudget,
                stop_reason: None,
                usage: EngineTokenUsage {
                    input_tokens: Some(input_tokens),
                    ..EngineTokenUsage::default()
                },
                rounds: 0,
            };
            record_call(host, runtime, &node.id, status.clone(), started_at).await?;
            host.emit_flow_token(
                runtime,
                &node.id,
                "status",
                token("conversation.call_status", &status)?,
                cancel,
            )
            .await?;
            host.emit_flow_signal(runtime, node, "over_budget").await?;
            return Err(refuse(
                codes::OVER_BUDGET,
                format!("{input_tokens} input tokens exceed the budget of {budget}."),
            ));
        }
        host.reserve(
            runtime,
            &node.id,
            SpendEstimate {
                input_tokens,
                max_output_tokens: Some(max_output_tokens),
            },
        )
        .await?;
        match engine.stream_text(request).await {
            Ok(stream) => {
                host.start_provider(runtime, &node.id, provider_events(stream, started_at))
                    .await
            }
            Err(error) => {
                let status = CallStatus {
                    outcome: CallOutcome::Failed,
                    stop_reason: None,
                    usage: EngineTokenUsage::default(),
                    rounds: 0,
                };
                record_call(host, runtime, &node.id, status.clone(), started_at).await?;
                host.emit_flow_token(
                    runtime,
                    &node.id,
                    "status",
                    token("conversation.call_status", &status)?,
                    cancel,
                )
                .await?;
                Err(provider_failed(&error))
            }
        }
    }
    async fn receive_provider_event(
        &self,
        host: &H,
        runtime: &mut H::State,
        node: &FlowNode,
        _definition: &FlowNodeDefinition,
        value: &Value,
        cancel: &CancellationToken,
    ) -> Result<(), H::Error> {
        let completion: Completion = serde_json::from_value(value.clone()).map_err(|error| {
            H::Error::internal(format!("The chat completion is malformed: {error}"))
        })?;
        let (outcome, failure) = match &completion.failure {
            None => (CallOutcome::Completed, None),
            Some(error) if error.tool_code == Some(ToolCode::ToolRoundsExceeded) => {
                (CallOutcome::Failed, Some(codes::TOOL_ROUNDS_EXHAUSTED))
            }
            Some(error) if error.classification == ErrorKind::Cancelled => {
                (CallOutcome::Cancelled, Some(codes::PROVIDER_FAILED))
            }
            Some(_) => (CallOutcome::Failed, Some(codes::PROVIDER_FAILED)),
        };
        let status = CallStatus {
            outcome,
            stop_reason: completion.stop_reason.clone(),
            usage: completion.usage,
            rounds: completion.rounds.len() as u32,
        };
        record_call(
            host,
            runtime,
            &node.id,
            status.clone(),
            completion.started_at,
        )
        .await?;
        if failure.is_none() {
            let reply = Turn::text(Role::Assistant, completion.text, H::Extension::default());
            host.emit_flow_token(
                runtime,
                &node.id,
                "reply",
                token("conversation.turn", &reply)?,
                cancel,
            )
            .await?;
        }
        host.emit_flow_token(
            runtime,
            &node.id,
            "status",
            token("conversation.call_status", &status)?,
            cancel,
        )
        .await?;
        host.emit_flow_token(
            runtime,
            &node.id,
            "tool_rounds",
            token("conversation.tool_rounds", &completion.rounds)?,
            cancel,
        )
        .await?;
        match (failure, completion.failure) {
            (Some(code), Some(error)) => Err(refuse(code, error.message.to_string())),
            _ => host.emit_flow_signal(runtime, node, "completed").await,
        }
    }
}
