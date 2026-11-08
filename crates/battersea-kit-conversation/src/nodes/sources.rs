//! Copyright (c) Scott A Dixon
//!
//! The sources: the prompt, the static prompt, the transcript, the memory and the turn to
//! refine. All but the static prompt read the port.
use super::{
    count, fragment, messages, provider_failed, refuse, render, token, turn_binding, unsigned,
};
use crate::{codes, Port, Turn};
use async_trait::async_trait;
use battersea_flow::{FlowNode, FlowNodeDefinition};
use battersea_kit::text::{activation_text, prepare_static_prompt};
use battersea_model::{EngineAdapter, EngineAdapterRequestError, Role};
use battersea_runtime::NodeHandler;
use serde_json::{json, Map, Value};
use std::{collections::HashMap, ops::Range};
use tokio_util::sync::CancellationToken;

/// `conversation.prompt`: the activation's `prompt` value as text and as a user turn.
pub struct PromptHandler;
/// `conversation.static_prompt`: the `text` parameter as a prompt fragment.
pub struct StaticPromptHandler;
/// `conversation.transcript.source`: the run's transcript, as turns and as text.
pub struct TranscriptHandler;
/// `conversation.memory`: the summary and the turns after it that fit `budget`.
pub struct MemoryHandler;
/// `conversation.refine.source`: the committed turn the activation's `turn` value names.
pub struct RefinementSourceHandler;

#[async_trait]
impl<H: Port> NodeHandler<H> for PromptHandler {
    fn handler_id(&self) -> &'static str {
        "conversation.prompt"
    }
    async fn execute_node(
        &self,
        host: &H,
        runtime: &mut H::State,
        node: &FlowNode,
        _definition: &FlowNodeDefinition,
        values: Option<&HashMap<String, Value>>,
        cancel: &CancellationToken,
    ) -> Result<(), H::Error> {
        let text =
            activation_text(values, "prompt").map_err(|error| error.into_host::<H::Error>())?;
        host.emit_flow_token(runtime, &node.id, "text", fragment(text.clone()), cancel)
            .await?;
        let turn = Turn::text(Role::User, text, H::Extension::default());
        host.emit_flow_token(
            runtime,
            &node.id,
            "turn",
            token("conversation.turn", &turn)?,
            cancel,
        )
        .await
    }
}

#[async_trait]
impl<H: Port> NodeHandler<H> for StaticPromptHandler {
    fn handler_id(&self) -> &'static str {
        "conversation.static_prompt"
    }
    async fn materialize_node(
        &self,
        host: &H,
        runtime: &mut H::State,
        node: &FlowNode,
        definition: &FlowNodeDefinition,
        cancel: &CancellationToken,
    ) -> Result<(), H::Error> {
        host.emit_flow_token(
            runtime,
            &node.id,
            "text",
            prepare_static_prompt(node, definition),
            cancel,
        )
        .await
    }
}

/// The formatter bindings of `turns`.
fn turns_binding<X>(turns: &[Turn<X>]) -> Value {
    Value::Array(turns.iter().map(turn_binding).collect())
}

#[async_trait]
impl<H: Port> NodeHandler<H> for TranscriptHandler {
    fn handler_id(&self) -> &'static str {
        "conversation.transcript.source"
    }
    async fn materialize_node(
        &self,
        host: &H,
        runtime: &mut H::State,
        node: &FlowNode,
        definition: &FlowNodeDefinition,
        cancel: &CancellationToken,
    ) -> Result<(), H::Error> {
        let transcript = H::run_state(runtime).transcript.clone();
        let turns = host.turns(runtime, &transcript).await?;
        host.emit_flow_token(
            runtime,
            &node.id,
            "turns",
            token("conversation.turns", &turns)?,
            cancel,
        )
        .await?;
        let mut bindings = Map::new();
        bindings.insert("turns".into(), turns_binding(&turns));
        let text = render::<H::Error>(definition, "text", None, bindings)?;
        host.emit_flow_token(runtime, &node.id, "text", fragment(text), cancel)
            .await
    }
}

/// The units of `turns`: an assistant turn that calls tools and the turns answering its calls,
/// in the tool or the user role, are one unit; every other turn is one.
fn units<X>(turns: &[Turn<X>]) -> Vec<Range<usize>> {
    let mut units = vec![];
    let mut start = 0;
    while start < turns.len() {
        let mut end = start + 1;
        if turns[start].role == Role::Assistant {
            let calls = turns[start].tool_call_ids();
            if !calls.is_empty() {
                while end < turns.len() && turns[end].answers(&calls) {
                    end += 1;
                }
            }
        }
        units.push(start..end);
        start = end;
    }
    units
}

/// The newest units of `turns` that `engine` measures within `budget`, in order, and their
/// measure.
pub async fn fit<X: Clone>(
    engine: &dyn EngineAdapter,
    turns: &[Turn<X>],
    budget: u64,
) -> Result<(Vec<Turn<X>>, u64), EngineAdapterRequestError> {
    let mut kept = turns.len();
    let mut tokens = 0;
    for unit in units(turns).into_iter().rev() {
        let measured = count(engine, messages(&turns[unit.start..])).await?;
        if measured > budget {
            break;
        }
        kept = unit.start;
        tokens = measured;
    }
    Ok((turns[kept..].to_vec(), tokens))
}

#[async_trait]
impl<H: Port> NodeHandler<H> for MemoryHandler {
    fn handler_id(&self) -> &'static str {
        "conversation.memory"
    }
    async fn materialize_node(
        &self,
        host: &H,
        runtime: &mut H::State,
        node: &FlowNode,
        definition: &FlowNodeDefinition,
        cancel: &CancellationToken,
    ) -> Result<(), H::Error> {
        let budget = unsigned(node, definition, "budget").unwrap_or(4000);
        let transcript = H::run_state(runtime).transcript.clone();
        let memory = host.memory(runtime, &transcript).await?;
        let engine = host.engine(runtime, &node.id)?;
        let (turns, _) = fit(engine.as_ref(), &memory.turns, budget)
            .await
            .map_err(|error| provider_failed::<H::Error>(&error))?;
        host.emit_flow_token(
            runtime,
            &node.id,
            "turns",
            token("conversation.turns", &turns)?,
            cancel,
        )
        .await?;
        let mut bindings = Map::new();
        bindings.insert(
            "summary".into(),
            memory
                .summary
                .as_ref()
                .map_or(Value::Null, |summary| json!(summary.text)),
        );
        bindings.insert("turns".into(), turns_binding(&turns));
        let context = render::<H::Error>(definition, "context", None, bindings)?;
        host.emit_flow_token(runtime, &node.id, "context", fragment(context), cancel)
            .await
    }
}

#[async_trait]
impl<H: Port> NodeHandler<H> for RefinementSourceHandler {
    fn handler_id(&self) -> &'static str {
        "conversation.refine.source"
    }
    async fn execute_node(
        &self,
        host: &H,
        runtime: &mut H::State,
        node: &FlowNode,
        definition: &FlowNodeDefinition,
        values: Option<&HashMap<String, Value>>,
        cancel: &CancellationToken,
    ) -> Result<(), H::Error> {
        let id = activation_text(values, "turn").map_err(|error| error.into_host::<H::Error>())?;
        let transcript = H::run_state(runtime).transcript.clone();
        let turn = host
            .turns(runtime, &transcript)
            .await?
            .into_iter()
            .find(|turn| turn.id.as_ref().is_some_and(|held| held.as_str() == id))
            .ok_or_else(|| {
                refuse::<H::Error>(
                    codes::UNKNOWN_TURN,
                    format!("Transcript {transcript} does not hold turn {id:?}."),
                )
            })?;
        host.emit_flow_token(
            runtime,
            &node.id,
            "turn",
            token("conversation.turn", &turn)?,
            cancel,
        )
        .await?;
        let bindings = match turn_binding(&turn) {
            Value::Object(bindings) => bindings,
            _ => Map::new(),
        };
        let text = render::<H::Error>(definition, "text", None, bindings)?;
        host.emit_flow_token(runtime, &node.id, "text", fragment(text), cancel)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use battersea_model::ContentBlock;
    use serde_json::json;

    fn turn(role: Role, content: Vec<ContentBlock>) -> Turn<()> {
        Turn {
            id: None,
            role,
            content,
            created_at: None,
            revision: 0,
            extension: (),
        }
    }

    #[test]
    fn a_tool_result_in_the_user_role_stays_with_its_call() {
        let turns = vec![
            turn(Role::User, vec![ContentBlock::Text { text: "go".into() }]),
            turn(
                Role::Assistant,
                vec![ContentBlock::ToolCall {
                    id: "c1".into(),
                    name: "echo".into(),
                    arguments: json!({}),
                }],
            ),
            turn(
                Role::User,
                vec![ContentBlock::ToolResult {
                    id: "c1".into(),
                    content: json!("done"),
                }],
            ),
            turn(
                Role::Tool,
                vec![ContentBlock::ToolResult {
                    id: "other".into(),
                    content: json!("stray"),
                }],
            ),
            turn(
                Role::User,
                vec![ContentBlock::Text {
                    text: "next".into(),
                }],
            ),
        ];
        assert_eq!(units(&turns), vec![0..1, 1..3, 3..4, 4..5]);
    }
}
