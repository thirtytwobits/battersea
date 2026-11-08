//! Copyright (c) Scott A Dixon
//!
//! The inline nodes beside the chat: the buffer, the signal tap and the two parsers.
use super::{fragment, record, refuse, text_of, token, unsigned};
use crate::{codes, JobId, Port, Revision, Summary, Turn};
use async_trait::async_trait;
use battersea_flow::{FlowNode, FlowNodeDefinition};
use battersea_model::ContentBlock;
use battersea_runtime::{error_codes, ExecutionError, NodeHandler, Token};
use serde_json::Value;
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;

/// `conversation.buffer`: collects a stream's text, at most `retain` bytes, into one fragment.
pub struct BufferHandler;
/// `conversation.tap`: passes its input through and fires `received`.
pub struct SignalTapHandler;
/// `conversation.compaction.parse`: the model's response as a summary of the given turns.
pub struct CompactionParserHandler;
/// `conversation.refine.parse`: the model's response as a revision of the given turn.
pub struct RefinementParserHandler;

#[async_trait]
impl<H: Port> NodeHandler<H> for BufferHandler {
    fn handler_id(&self) -> &'static str {
        "conversation.buffer"
    }
    async fn receive_input_token(
        &self,
        _host: &H,
        runtime: &mut H::State,
        node: &FlowNode,
        definition: &FlowNodeDefinition,
        _port: &str,
        value: &Token,
        _cancel: &CancellationToken,
    ) -> Result<(), H::Error> {
        let retain = unsigned(node, definition, "retain").unwrap_or(65_536);
        let delta = text_of::<H::Error>(value)?;
        let held = H::run_state(runtime)
            .buffers
            .entry(node.id.clone())
            .or_default();
        let total = held.len() as u64 + delta.len() as u64;
        if total > retain {
            return Err(H::Error::invalid_request(
                error_codes::CAPACITY_EXHAUSTED,
                format!("The stream exceeds the {retain} bytes the buffer retains."),
            ));
        }
        held.push_str(&delta);
        Ok(())
    }
    async fn execute_node(
        &self,
        host: &H,
        runtime: &mut H::State,
        node: &FlowNode,
        _definition: &FlowNodeDefinition,
        _values: Option<&HashMap<String, Value>>,
        cancel: &CancellationToken,
    ) -> Result<(), H::Error> {
        let text = H::run_state(runtime)
            .buffers
            .remove(&node.id)
            .unwrap_or_default();
        host.emit_flow_token(runtime, &node.id, "text", fragment(text), cancel)
            .await
    }
}

#[async_trait]
impl<H: Port> NodeHandler<H> for SignalTapHandler {
    fn handler_id(&self) -> &'static str {
        "conversation.tap"
    }
    async fn execute_node(
        &self,
        host: &H,
        runtime: &mut H::State,
        node: &FlowNode,
        _definition: &FlowNodeDefinition,
        _values: Option<&HashMap<String, Value>>,
        cancel: &CancellationToken,
    ) -> Result<(), H::Error> {
        let input = host.take_flow_input_token(runtime, &node.id, "input")?;
        host.emit_flow_token(runtime, &node.id, "output", (*input).clone(), cancel)
            .await?;
        host.emit_flow_signal(runtime, node, "received").await
    }
}

/// The response text, or the refusal for one the parsers cannot read.
fn response<E: ExecutionError>(text: &Token) -> Result<String, E> {
    let text = text_of::<E>(text)?;
    let text = text.trim();
    if text.is_empty() {
        return Err(refuse(
            codes::UNPARSABLE_RESPONSE,
            "The model's response holds no text.",
        ));
    }
    Ok(text.to_string())
}

#[async_trait]
impl<H: Port> NodeHandler<H> for CompactionParserHandler {
    fn handler_id(&self) -> &'static str {
        "conversation.compaction.parse"
    }
    async fn execute_node(
        &self,
        host: &H,
        runtime: &mut H::State,
        node: &FlowNode,
        _definition: &FlowNodeDefinition,
        _values: Option<&HashMap<String, Value>>,
        cancel: &CancellationToken,
    ) -> Result<(), H::Error> {
        let turns: Vec<Turn<H::Extension>> =
            record(&*host.take_flow_input_token(runtime, &node.id, "turns")?)?;
        let text = response::<H::Error>(&*host.take_flow_input_token(runtime, &node.id, "text")?)?;
        let through = turns
            .last()
            .and_then(|turn| turn.id.clone())
            .ok_or_else(|| {
                refuse::<H::Error>(
                    codes::UNKNOWN_TURN,
                    "A summary covers committed turns; the last turn given has no id.",
                )
            })?;
        let job = runtime
            .activation_values
            .get("job")
            .and_then(Value::as_str)
            .map(JobId::from);
        let summary = Summary {
            text,
            through,
            job,
            extension: H::Extension::default(),
        };
        host.emit_flow_token(
            runtime,
            &node.id,
            "summary",
            token("conversation.summary", &summary)?,
            cancel,
        )
        .await
    }
}

#[async_trait]
impl<H: Port> NodeHandler<H> for RefinementParserHandler {
    fn handler_id(&self) -> &'static str {
        "conversation.refine.parse"
    }
    async fn execute_node(
        &self,
        host: &H,
        runtime: &mut H::State,
        node: &FlowNode,
        _definition: &FlowNodeDefinition,
        _values: Option<&HashMap<String, Value>>,
        cancel: &CancellationToken,
    ) -> Result<(), H::Error> {
        let turn: Turn<H::Extension> =
            record(&*host.take_flow_input_token(runtime, &node.id, "turn")?)?;
        let text = response::<H::Error>(&*host.take_flow_input_token(runtime, &node.id, "text")?)?;
        let id = turn.id.ok_or_else(|| {
            refuse::<H::Error>(
                codes::UNKNOWN_TURN,
                "A revision applies to a committed turn; the turn given has no id.",
            )
        })?;
        let revision = Revision {
            turn: id,
            content: vec![ContentBlock::Text { text }],
            extension: H::Extension::default(),
        };
        host.emit_flow_token(
            runtime,
            &node.id,
            "revision",
            token("conversation.revision", &revision)?,
            cancel,
        )
        .await
    }
}
