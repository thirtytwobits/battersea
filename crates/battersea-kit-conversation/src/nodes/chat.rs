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
    EngineTextStreamEvent, EngineTokenUsage, ErrorKind, Message, Role, StopReason,
    ToolCallEvidence, ToolCode,
};
use battersea_runtime::{ExecutionError, NodeHandler, ProviderEvent, Token};
use futures_util::{stream, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
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
    /// The tool calls of the round in progress.
    open: Vec<ToolCall>,
    /// The latest usage each provider turn reported.
    usage: BTreeMap<u32, EngineTokenUsage>,
    stop_reason: Option<String>,
    started_at: u64,
    done: bool,
}
impl Progress {
    /// Closes the round in progress, if it holds a call.
    fn flush_round(&mut self) {
        if !self.open.is_empty() {
            let round = self.rounds.len() as u32;
            self.rounds.push(ToolRound {
                round,
                calls: std::mem::take(&mut self.open),
            });
        }
    }
    /// Keeps the latest fields a turn reports; a provider's snapshots within a turn are
    /// cumulative, not additive.
    fn observe(&mut self, usage: EngineTokenUsage) {
        let turn = self.usage.entry(usage.turn_index).or_default();
        turn.turn_index = usage.turn_index;
        for (held, next) in [
            (&mut turn.cached_input_tokens, usage.cached_input_tokens),
            (
                &mut turn.cache_write_input_tokens,
                usage.cache_write_input_tokens,
            ),
            (
                &mut turn.reasoning_output_tokens,
                usage.reasoning_output_tokens,
            ),
            (&mut turn.input_tokens, usage.input_tokens),
            (&mut turn.output_tokens, usage.output_tokens),
            (&mut turn.total_tokens, usage.total_tokens),
        ] {
            *held = next.or(*held);
        }
    }
    /// Attaches `evidence` to the call it names, in the round in progress or a closed one;
    /// a call the stream never started is added to the round in progress.
    fn attest(&mut self, evidence: &ToolCallEvidence) {
        let call = self
            .open
            .iter_mut()
            .chain(
                self.rounds
                    .iter_mut()
                    .flat_map(|round| round.calls.iter_mut()),
            )
            .find(|call| call.id == evidence.call_id);
        match call {
            Some(call) => call.evidence = Some(evidence.clone()),
            None => self.open.push(ToolCall {
                id: evidence.call_id.clone(),
                name: evidence.tool_name.clone(),
                arguments: Value::Null,
                ok: false,
                result: Value::Null,
                evidence: Some(evidence.clone()),
            }),
        }
    }
    fn completion(&mut self, failure: Option<EngineAdapterRequestError>) -> ProviderEvent {
        self.done = true;
        if let Some(evidence) = failure
            .as_ref()
            .and_then(|error| error.tool_call.as_deref())
        {
            self.attest(evidence);
        }
        self.flush_round();
        let completion = Completion {
            text: std::mem::take(&mut self.text),
            rounds: std::mem::take(&mut self.rounds),
            usage: total(self.usage.values()),
            stop_reason: self.stop_reason.take(),
            started_at: self.started_at,
            failure,
        };
        ProviderEvent::Data {
            value: serde_json::to_value(completion).expect("a completion serialises"),
        }
    }
}

/// The usage of every turn summed, field by field; a field no turn reports stays unreported,
/// and `turn_index` is the last turn's.
fn total<'a>(turns: impl Iterator<Item = &'a EngineTokenUsage>) -> EngineTokenUsage {
    fn sum(total: &mut Option<u64>, next: Option<u64>) {
        if total.is_some() || next.is_some() {
            *total = Some(total.unwrap_or(0) + next.unwrap_or(0));
        }
    }
    let mut all = EngineTokenUsage::default();
    for usage in turns {
        all.turn_index = usage.turn_index;
        sum(&mut all.cached_input_tokens, usage.cached_input_tokens);
        sum(
            &mut all.cache_write_input_tokens,
            usage.cache_write_input_tokens,
        );
        sum(
            &mut all.reasoning_output_tokens,
            usage.reasoning_output_tokens,
        );
        sum(&mut all.input_tokens, usage.input_tokens);
        sum(&mut all.output_tokens, usage.output_tokens);
        sum(&mut all.total_tokens, usage.total_tokens);
    }
    all
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
        usage: BTreeMap::new(),
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
                            progress.flush_round();
                            progress.stop_reason = Some(spelled(reason));
                        }
                        EngineTextStreamEvent::MessageStart { .. } => progress.flush_round(),
                        EngineTextStreamEvent::TokenUsage { usage } => progress.observe(usage),
                        EngineTextStreamEvent::ContentBlock { .. }
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

/// Sets `chat`'s round limit to `parameter`, capped by `cap` where the plan sets one, and
/// returns the limit the adapters accept: `parameter` normalised to their range of 1 to 32.
fn tool_rounds(chat: &mut EngineChatParameters, parameter: u64, cap: usize) -> usize {
    let mut rounds = u32::try_from(parameter).unwrap_or(u32::MAX);
    if cap > 0 {
        rounds = rounds.min(u32::try_from(cap).unwrap_or(u32::MAX));
    }
    chat.max_tool_rounds = rounds;
    let rounds = chat.max_tool_rounds();
    chat.max_tool_rounds = rounds as u32;
    rounds
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
        let plan = host.tool_plan(runtime, &node.id)?;
        let mut chat = EngineChatParameters::default_for_provider(crate::NAME);
        chat.max_output_tokens = Some(u32::try_from(max_output_tokens).unwrap_or(u32::MAX));
        let rounds = tool_rounds(
            &mut chat,
            unsigned(node, definition, "max_tool_rounds").unwrap_or(4),
            plan.max_rounds,
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use battersea_model::{EngineAdapterRequestError, ToolCallOutcome};
    use serde_json::json;

    /// The completion the stream of `events` ends with.
    async fn completion_of(
        events: Vec<Result<EngineTextStreamEvent, EngineAdapterRequestError>>,
    ) -> (Vec<ProviderEvent>, Completion) {
        let mut events = provider_events(Box::pin(stream::iter(events)), 7)
            .collect::<Vec<_>>()
            .await;
        let last = events.pop().expect("a completion event");
        let ProviderEvent::Data { value } = last else {
            panic!("the stream ends with its completion, not {last:?}");
        };
        (events, serde_json::from_value(value).expect("a completion"))
    }

    fn usage(turn_index: u32, input: Option<u64>, output: Option<u64>) -> EngineTextStreamEvent {
        EngineTextStreamEvent::TokenUsage {
            usage: EngineTokenUsage {
                turn_index,
                input_tokens: input,
                output_tokens: output,
                ..EngineTokenUsage::default()
            },
        }
    }

    fn started(id: &str) -> EngineTextStreamEvent {
        EngineTextStreamEvent::ToolCallStarted {
            id: id.into(),
            name: "echo".into(),
            arguments: json!({}),
            started_at: String::new(),
        }
    }

    #[test]
    fn the_round_limit_is_capped_by_the_plan_and_normalised_to_the_adapters_range() {
        let mut chat = EngineChatParameters::default_for_provider(crate::NAME);
        assert_eq!(tool_rounds(&mut chat, 4, 0), 4);
        assert_eq!(tool_rounds(&mut chat, 4, 2), 2);
        assert_eq!(tool_rounds(&mut chat, 0, 0), 1);
        assert_eq!(tool_rounds(&mut chat, 100, 0), 32);
        assert_eq!(chat.max_tool_rounds, 32);
    }

    #[tokio::test]
    async fn usage_keeps_a_turns_latest_snapshot_and_sums_across_turns() {
        let (deltas, completion) = completion_of(vec![
            Ok(EngineTextStreamEvent::MessageStart { turn_index: 0 }),
            Ok(usage(0, Some(10), Some(1))),
            Ok(EngineTextStreamEvent::TextDelta { text: "Hel".into() }),
            Ok(EngineTextStreamEvent::TextDelta { text: "lo".into() }),
            Ok(usage(0, None, Some(5))),
            Ok(EngineTextStreamEvent::MessageStart { turn_index: 1 }),
            Ok(usage(1, Some(4), Some(2))),
            Ok(EngineTextStreamEvent::MessageStop {
                reason: StopReason::Complete,
            }),
        ])
        .await;
        assert_eq!(deltas.len(), 2);
        assert_eq!(completion.text, "Hello");
        assert_eq!(completion.usage.input_tokens, Some(14));
        assert_eq!(completion.usage.output_tokens, Some(7));
        assert_eq!(completion.usage.total_tokens, None);
        assert_eq!(completion.usage.turn_index, 1);
        assert_eq!(completion.stop_reason.as_deref(), Some("complete"));
        assert!(completion.failure.is_none());
    }

    #[tokio::test]
    async fn tool_rounds_close_at_the_next_message_and_at_a_failed_end() {
        let mut error = EngineAdapterRequestError::new("test", "rounds", ErrorKind::Request)
            .with_tool_code(ToolCode::ToolRoundsExceeded);
        error.tool_call = Some(Box::new(ToolCallEvidence {
            call_id: "call-2".into(),
            tool_name: "echo".into(),
            outcome: ToolCallOutcome::Uncertain,
            request_id: None,
        }));
        let (_, completion) = completion_of(vec![
            Ok(EngineTextStreamEvent::MessageStart { turn_index: 0 }),
            Ok(started("call-1")),
            Ok(EngineTextStreamEvent::ToolCallCompleted {
                id: "call-1".into(),
                name: "echo".into(),
                ok: true,
                result: json!(1),
                error_message: None,
                error_classification: None,
                tool_result: None,
                tool_call: None,
                finished_at: String::new(),
            }),
            Ok(EngineTextStreamEvent::MessageStart { turn_index: 1 }),
            Ok(started("call-2")),
            Err(error),
        ])
        .await;
        assert_eq!(completion.rounds.len(), 2);
        assert_eq!(completion.rounds[0].calls[0].id, "call-1");
        assert!(completion.rounds[0].calls[0].ok);
        assert_eq!(completion.rounds[1].round, 1);
        assert_eq!(completion.rounds[1].calls[0].id, "call-2");
        assert_eq!(
            completion.rounds[1].calls[0]
                .evidence
                .as_ref()
                .map(|evidence| evidence.outcome),
            Some(ToolCallOutcome::Uncertain)
        );
        assert_eq!(
            completion.failure.and_then(|error| error.tool_code),
            Some(ToolCode::ToolRoundsExceeded)
        );
    }
}
