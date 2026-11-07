//! Copyright (c) Scott A Dixon
//!
//! The kit's handlers, generic over the port they reach the application through, and what
//! they share: token and message conversions, parameter reads and formatter rendering.
use crate::{codes, Port, ToolPlan, Turn};
use battersea_flow::{
    ports::effective_parameter_value,
    prompt_markdown::{prompt_fragment_text, prompt_fragment_value, PromptFragmentEncoding},
    template_engine::render_port_formatter,
    FlowNode, FlowNodeDefinition,
};
use battersea_model::{
    engine::EngineChatParameters, EngineAdapter, EngineAdapterRequest, EngineAdapterRequestError,
    EngineOperation, EngineTextStreamRequest, Message,
};
use battersea_runtime::{error_codes, ExecutionError, RegistryBuilder, Token};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Map, Value};

pub mod chat;
pub mod inline;
pub mod sinks;
pub mod sources;
pub use chat::ChatHandler;
pub use inline::{
    BufferHandler, CompactionParserHandler, RefinementParserHandler, SignalTapHandler,
};
pub use sinks::{
    CompactionRequestHandler, MemoryOutputHandler, OutputHandler, RefinementOutputHandler,
    TranscriptOutputHandler,
};
pub use sources::{
    MemoryHandler, PromptHandler, RefinementSourceHandler, StaticPromptHandler, TranscriptHandler,
};

/// Installs the kit's handlers on a host that implements [`Port`].
pub fn register_handlers<H: Port>(builder: &mut RegistryBuilder<H>) -> Result<(), String> {
    builder.register(PromptHandler)?;
    builder.register(StaticPromptHandler)?;
    builder.register(TranscriptHandler)?;
    builder.register(MemoryHandler)?;
    builder.register(ChatHandler)?;
    builder.register(BufferHandler)?;
    builder.register(SignalTapHandler)?;
    builder.register(OutputHandler)?;
    builder.register(TranscriptOutputHandler)?;
    builder.register(CompactionRequestHandler)?;
    builder.register(CompactionParserHandler)?;
    builder.register(MemoryOutputHandler)?;
    builder.register(RefinementSourceHandler)?;
    builder.register(RefinementParserHandler)?;
    builder.register(RefinementOutputHandler)?;
    Ok(())
}

/// The node's unsigned parameter, the manifest's default applied.
pub(crate) fn unsigned(
    node: &FlowNode,
    definition: &FlowNodeDefinition,
    name: &str,
) -> Option<u64> {
    effective_parameter_value(node, definition, name).and_then(|value| value.as_u64())
}

/// `text` as a plain-text prompt fragment.
pub(crate) fn fragment(text: impl Into<String>) -> Token {
    Token {
        token_type: "prompt.fragment".into(),
        value: prompt_fragment_value(text, PromptFragmentEncoding::PlainText),
    }
}

/// The text of a `prompt.fragment` or `chat.raw` token.
pub(crate) fn text_of<E: ExecutionError>(token: &Token) -> Result<String, E> {
    prompt_fragment_text(&token.value).ok_or_else(|| {
        E::invalid_request(
            error_codes::INCOMPATIBLE_TOKEN_TYPE,
            format!("A {} token carries no text.", token.token_type),
        )
    })
}

/// `record` as a token of `token_type`.
pub(crate) fn token<E: ExecutionError>(
    token_type: &str,
    record: &impl Serialize,
) -> Result<Token, E> {
    Ok(Token {
        token_type: token_type.into(),
        value: serde_json::to_value(record).map_err(|error| E::internal(error.to_string()))?,
    })
}

/// The record a token of the kit's type carries.
pub(crate) fn record<T: DeserializeOwned, E: ExecutionError>(token: &Token) -> Result<T, E> {
    serde_json::from_value(token.value.clone()).map_err(|error| {
        E::invalid_request(
            error_codes::INCOMPATIBLE_TOKEN_TYPE,
            format!("A {} token is malformed: {error}", token.token_type),
        )
    })
}

/// The turns a `conversation.turn` or `conversation.turns` token carries.
pub(crate) fn turns_of<X: DeserializeOwned, E: ExecutionError>(
    token: &Token,
) -> Result<Vec<Turn<X>>, E> {
    match token.token_type.as_str() {
        "conversation.turn" => Ok(vec![record(token)?]),
        _ => record(token),
    }
}

/// The messages a chat request makes of `turns`.
pub(crate) fn messages<X>(turns: &[Turn<X>]) -> Vec<Message> {
    turns
        .iter()
        .map(|turn| Message {
            role: turn.role,
            content: turn.content.clone(),
        })
        .collect()
}

/// A request for `messages` with `chat` and the tools of `plan`, allowed `rounds` tool rounds.
pub(crate) fn request(
    messages: Vec<Message>,
    chat: EngineChatParameters,
    plan: &ToolPlan,
    rounds: usize,
) -> EngineTextStreamRequest {
    EngineTextStreamRequest {
        mock_response: None,
        shared: EngineAdapterRequest {
            operation: EngineOperation::FlowTextStream,
            messages,
            chat,
            debug_rules: None,
        },
        local_tools: plan.definitions.clone(),
        local_tool_executor: plan.executor.clone(),
        max_tool_rounds: rounds,
    }
}

/// The input tokens `engine` counts for `messages`, as it counts a chat request without tools.
pub(crate) async fn count(
    engine: &dyn EngineAdapter,
    messages: Vec<Message>,
) -> Result<u64, EngineAdapterRequestError> {
    let chat = EngineChatParameters::default_for_provider(crate::NAME);
    engine
        .count_text_stream_input_tokens(request(messages, chat, &ToolPlan::default(), 0))
        .await
}

/// A provider's refusal as the kit's `conversation.provider_failed`.
pub(crate) fn provider_failed<E: ExecutionError>(error: &EngineAdapterRequestError) -> E {
    E::invalid_request(codes::PROVIDER_FAILED, error.message.to_string())
}

/// What the formatter blocks bind for one turn.
pub(crate) fn turn_binding<X>(turn: &Turn<X>) -> Value {
    json!({"role": turn.role, "text": turn.text_content()})
}

/// Renders the node's output `port` with its formatter, `variant` or the default.
pub(crate) fn render<E: ExecutionError>(
    definition: &FlowNodeDefinition,
    port: &str,
    variant: Option<&str>,
    bindings: Map<String, Value>,
) -> Result<String, E> {
    let port = definition
        .output_ports
        .iter()
        .find(|candidate| candidate.name == port)
        .ok_or_else(|| {
            E::internal(format!(
                "{} declares no output port {port:?}.",
                definition.class_name
            ))
        })?;
    render_port_formatter(port, variant, &bindings).map_err(|error| {
        E::internal(format!(
            "Rendering {}.{} failed: {error}",
            definition.class_name, port.name
        ))
    })
}

/// The clock reading of `host`, in milliseconds since the Unix epoch.
pub(crate) fn now<H: Port>(host: &H) -> Result<u64, H::Error> {
    host.clock()
        .unix_ms()
        .ok_or_else(|| H::Error::internal("The clock reads before the Unix epoch."))
}

/// A refusal with one of the kit's codes.
pub(crate) fn refuse<E: ExecutionError>(code: &'static str, message: impl Into<String>) -> E {
    E::invalid_request(code, message)
}
