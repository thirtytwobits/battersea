//! Model-declared content capabilities and native structured-output validation.
use crate::{ContentBlock, EngineAdapterRequestError, ErrorKind, Message, Role};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    Text,
    Image,
    Document,
    Audio,
    Video,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StructuredOutputMode {
    JsonObject,
    JsonSchema,
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContentCapabilities {
    pub input_modalities: Vec<Modality>,
    pub output_modalities: Vec<Modality>,
    pub structured_output: Vec<StructuredOutputMode>,
    pub native_continuation: bool,
    pub prompt_caching: bool,
}
impl ContentCapabilities {
    pub fn text() -> Self {
        Self {
            input_modalities: vec![Modality::Text],
            output_modalities: vec![Modality::Text],
            structured_output: vec![],
            native_continuation: false,
            prompt_caching: false,
        }
    }
    pub fn validate_subset(&self, transport: &Self) -> Result<(), String> {
        if self.input_modalities.is_empty()
            || self.output_modalities.is_empty()
            || self
                .input_modalities
                .iter()
                .any(|m| !transport.input_modalities.contains(m))
            || self
                .output_modalities
                .iter()
                .any(|m| !transport.output_modalities.contains(m))
            || self
                .structured_output
                .iter()
                .any(|m| !transport.structured_output.contains(m))
            || (self.native_continuation && !transport.native_continuation)
            || (self.prompt_caching && !transport.prompt_caching)
        {
            return Err(
                "Model content capabilities exceed the registered transport capabilities.".into(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResponseFormat {
    JsonObject,
    JsonSchema { name: String, schema: Value },
}
impl ResponseFormat {
    pub fn mode(&self) -> StructuredOutputMode {
        match self {
            Self::JsonObject => StructuredOutputMode::JsonObject,
            Self::JsonSchema { .. } => StructuredOutputMode::JsonSchema,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if let Self::JsonSchema { name, schema } = self {
            if name.is_empty()
                || name.len() > 64
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                return Err("Structured output schema name must contain 1-64 ASCII letters, digits, underscores or hyphens.".into());
            }
            validate_local_schema(schema)?;
            jsonschema::validator_for(schema).map_err(|e| format!("Invalid output schema: {e}"))?;
        }
        Ok(())
    }
    pub fn parse(&self, provider: &str, text: &str) -> Result<Value, EngineAdapterRequestError> {
        let invalid = |message| EngineAdapterRequestError::invalid_response(provider, message);
        let value: Value = serde_json::from_str(text)
            .map_err(|e| invalid(format!("Malformed structured output: {e}")))?;
        match self {
            Self::JsonObject if !value.is_object() => {
                return Err(invalid("Structured output is not a JSON object.".into()))
            }
            Self::JsonSchema { schema, .. } => {
                let validator =
                    jsonschema::validator_for(schema).map_err(|e| invalid(e.to_string()))?;
                validator
                    .validate(&value)
                    .map_err(|e| invalid(format!("Structured output violates its schema: {e}")))?;
            }
            _ => {}
        }
        Ok(value)
    }
}
fn validate_local_schema(value: &Value) -> Result<(), String> {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if matches!(key.as_str(), "$ref" | "$dynamicRef")
                    && value.as_str().is_some_and(|s| !s.starts_with('#'))
                {
                    return Err("Output schemas cannot fetch external references.".into());
                }
                validate_local_schema(value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_local_schema(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn validate_messages(
    provider: &str,
    capabilities: &ContentCapabilities,
    messages: &[Message],
) -> Result<(), EngineAdapterRequestError> {
    let invalid = |text| EngineAdapterRequestError::new(provider, text, ErrorKind::InvalidRequest);
    let mut conversation = false;
    let mut calls = std::collections::HashMap::new();
    let mut completed = std::collections::HashSet::new();
    for message in messages {
        if message.content.is_empty() {
            return Err(invalid("Messages must contain content."));
        }
        if message.role == Role::System && conversation {
            return Err(invalid("System messages must precede the conversation."));
        }
        conversation |= message.role != Role::System;
        for block in &message.content {
            if message.role == Role::System && !matches!(block, ContentBlock::Text { .. }) {
                return Err(invalid("System messages accept text only."));
            }
            let modality = match block {
                ContentBlock::Text { .. } => Some(Modality::Text),
                ContentBlock::Image { .. } => Some(Modality::Image),
                ContentBlock::Document { .. } => Some(Modality::Document),
                ContentBlock::Audio { .. } => Some(Modality::Audio),
                ContentBlock::Video { .. } => Some(Modality::Video),
                _ => None,
            };
            if modality.is_some_and(|m| !capabilities.input_modalities.contains(&m)) {
                return Err(invalid(
                    "Backend does not support the requested input modality.",
                ));
            }
            match block {
                ContentBlock::Image { url, mime_type }
                | ContentBlock::Audio { url, mime_type }
                | ContentBlock::Video { url, mime_type }
                | ContentBlock::Document { url, mime_type, .. } => {
                    let generated = message.role == Role::Assistant
                        && capabilities.native_continuation
                        && modality
                            .is_some_and(|mode| capabilities.output_modalities.contains(&mode));
                    if message.role != Role::User && !generated {
                        return Err(invalid(
                            "Media must be user input or supported assistant output.",
                        ));
                    }
                    if url.trim().is_empty()
                        || mime_type.trim().is_empty()
                        || !(url.starts_with("https://")
                            || url.starts_with("data:")
                            || url.starts_with("gs://"))
                    {
                        return Err(invalid("Media requires a data URL or a remote reference and an explicit MIME type."));
                    }
                    let prefix = match modality {
                        Some(Modality::Image) => "image/",
                        Some(Modality::Audio) => "audio/",
                        Some(Modality::Video) => "video/",
                        _ => "",
                    };
                    if !mime_type.starts_with(prefix) {
                        return Err(invalid("Media MIME type conflicts with its modality."));
                    }
                    if url.starts_with("data:")
                        && !url.starts_with(&format!("data:{mime_type};base64,"))
                    {
                        return Err(invalid(
                            "Inline media must be base64 with the declared MIME type.",
                        ));
                    }
                }
                ContentBlock::Native {
                    provider: owner,
                    payload,
                } => {
                    if owner != provider
                        || !capabilities.native_continuation
                        || message.role != Role::Assistant
                        || !payload.is_object()
                    {
                        return Err(invalid("Native continuation requires its owning provider and an assistant object block."));
                    }
                }
                ContentBlock::Reasoning {
                    provider: owner,
                    signature,
                    ..
                } => {
                    if owner != provider
                        || !capabilities.native_continuation
                        || message.role != Role::Assistant
                        || signature.as_deref().is_none_or(str::is_empty)
                    {
                        return Err(invalid(
                            "Reasoning continuation requires its owning provider and signature.",
                        ));
                    }
                }
                ContentBlock::ToolCall {
                    id,
                    name,
                    arguments,
                } => {
                    if message.role != Role::Assistant
                        || id.is_empty()
                        || name.is_empty()
                        || !arguments.is_object()
                        || calls.insert(id, name).is_some()
                    {
                        return Err(invalid("Invalid or duplicate conversation tool call."));
                    }
                }
                ContentBlock::ToolResult { id, .. }
                    if !matches!(message.role, Role::Tool | Role::User)
                        || !calls.contains_key(id)
                        || !completed.insert(id) =>
                {
                    return Err(invalid("Tool result must match one preceding tool call."));
                }
                _ => {}
            }
            if message.role == Role::Tool && !matches!(block, ContentBlock::ToolResult { .. }) {
                return Err(invalid("Tool messages contain only tool results."));
            }
        }
    }
    if !conversation {
        return Err(invalid("A conversation message is required."));
    }
    Ok(())
}
