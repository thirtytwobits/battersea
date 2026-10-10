//! Lossless translation of admitted conversations onto the three provider protocols.
#![cfg_attr(
    not(any(feature = "openai", feature = "anthropic", feature = "google")),
    allow(dead_code)
)]
use base64::Engine as _;
use battersea_model::{
    ContentBlock, ContentCapabilities, EngineAdapterRequestError, Message, Modality, Role,
    StructuredOutputMode,
};
use serde_json::{json, Value};

pub(crate) fn capabilities(provider: &str) -> ContentCapabilities {
    use Modality::*;
    use StructuredOutputMode::*;
    ContentCapabilities {
        input_modalities: match provider {
            "mock" => vec![Text],
            "openai" | "anthropic" => vec![Text, Image, Document],
            _ => vec![Text, Image, Document, Audio, Video],
        },
        output_modalities: if provider == "google" {
            vec![Text, Image, Audio]
        } else {
            vec![Text]
        },
        structured_output: if provider == "anthropic" {
            vec![JsonSchema]
        } else {
            vec![JsonObject, JsonSchema]
        },
        native_continuation: provider != "mock",
        prompt_caching: provider != "mock",
    }
}
fn invalid(provider: &str, message: &str) -> EngineAdapterRequestError {
    EngineAdapterRequestError::new(provider, message, "invalid_request")
}
pub(crate) fn media_source<'a>(
    provider: &str,
    url: &'a str,
    mime: &str,
) -> Result<Option<&'a str>, EngineAdapterRequestError> {
    if url.starts_with("data:") {
        let data = url
            .strip_prefix(&format!("data:{mime};base64,"))
            .ok_or_else(|| invalid(provider, "Inline media MIME type mismatch."))?;
        if data.is_empty()
            || base64::engine::general_purpose::STANDARD
                .decode(data)
                .is_err()
        {
            return Err(invalid(
                provider,
                "Inline media must contain valid non-empty base64.",
            ));
        }
        Ok(Some(data))
    } else if url.starts_with("https://") || (provider == "google" && url.starts_with("gs://")) {
        let parsed = reqwest::Url::parse(url)
            .map_err(|_| invalid(provider, "Invalid remote media reference."))?;
        if parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(invalid(provider, "Invalid remote media reference."));
        }
        Ok(None)
    } else {
        Err(invalid(provider, "Unsupported media reference."))
    }
}

fn document_mime(mime: &str) -> bool {
    (mime.starts_with("text/")
        && mime.len() > 5
        && mime
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/-+._".contains(&b)))
        || [
            "application/pdf",
            "application/json",
            "application/javascript",
            "application/typescript",
            "application/csv",
            "application/rtf",
            "application/msword",
            "application/vnd.ms-excel",
            "application/vnd.ms-powerpoint",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "application/vnd.oasis.opendocument.text",
        ]
        .contains(&mime)
}

pub(crate) fn messages(
    provider: &str,
    messages: &[Message],
) -> Result<Vec<Value>, EngineAdapterRequestError> {
    let mut result = Vec::new();
    let mut tool_names = std::collections::HashMap::new();
    for message in messages.iter().filter(|m| m.role != Role::System) {
        let mut blocks = Vec::new();
        let role = match message.role {
            Role::Assistant => {
                if provider == "google" {
                    "model"
                } else {
                    "assistant"
                }
            }
            _ => "user",
        };
        for block in &message.content {
            let native = match block {
                ContentBlock::Text { text } => match provider {
                    "openai" => json!({"type": "input_text", "text": text}),
                    "anthropic" => json!({"type": "text", "text": text}),
                    _ => json!({"text": text}),
                },
                ContentBlock::Image { url, mime_type }
                | ContentBlock::Audio { url, mime_type }
                | ContentBlock::Video { url, mime_type }
                | ContentBlock::Document { url, mime_type, .. } => {
                    let data = media_source(provider, url, mime_type)?;
                    match provider {
                        "openai" => match block {
                            ContentBlock::Image { .. }
                                if ["image/png", "image/jpeg", "image/webp", "image/gif"]
                                    .contains(&mime_type.as_str()) =>
                            {
                                json!({"type": "input_image", "image_url": url, "detail": "auto"})
                            }
                            ContentBlock::Document { filename, .. }
                                if !filename.trim().is_empty() && document_mime(mime_type) =>
                            {
                                if data.is_some() {
                                    json!({"type": "input_file", "filename": filename, "file_data": url})
                                } else {
                                    json!({"type": "input_file", "file_url": url})
                                }
                            }
                            _ => {
                                return Err(invalid(
                                    provider,
                                    "Responses does not support the requested media type.",
                                ))
                            }
                        },
                        "anthropic" => {
                            let kind = match block {
                                ContentBlock::Image { .. }
                                    if ["image/png", "image/jpeg", "image/webp", "image/gif"]
                                        .contains(&mime_type.as_str()) =>
                                {
                                    "image"
                                }
                                ContentBlock::Document { .. } if mime_type == "application/pdf" => {
                                    "document"
                                }
                                _ => {
                                    return Err(invalid(
                                        provider,
                                        "Messages does not support the requested media type.",
                                    ))
                                }
                            };
                            let source = if let Some(data) = data {
                                json!({"type": "base64", "media_type": mime_type, "data": data})
                            } else {
                                json!({"type": "url", "url": url})
                            };
                            json!({"type": kind, "source": source})
                        }
                        "google" => {
                            let supported = match block {
                                ContentBlock::Image { .. } => [
                                    "image/png",
                                    "image/jpeg",
                                    "image/webp",
                                    "image/heic",
                                    "image/heif",
                                ]
                                .contains(&mime_type.as_str()),
                                ContentBlock::Audio { .. } => [
                                    "audio/wav",
                                    "audio/mp3",
                                    "audio/aiff",
                                    "audio/aac",
                                    "audio/ogg",
                                    "audio/flac",
                                    "audio/mpeg",
                                    "audio/m4a",
                                    "audio/l16",
                                    "audio/opus",
                                    "audio/alaw",
                                    "audio/mulaw",
                                    "audio/webm",
                                ]
                                .contains(&mime_type.as_str()),
                                ContentBlock::Video { .. } => [
                                    "video/mp4",
                                    "video/mpeg",
                                    "video/quicktime",
                                    "video/avi",
                                    "video/x-flv",
                                    "video/mpg",
                                    "video/webm",
                                    "video/wmv",
                                    "video/3gpp",
                                ]
                                .contains(&mime_type.as_str()),
                                ContentBlock::Document { .. } => {
                                    ["application/pdf", "text/plain"].contains(&mime_type.as_str())
                                }
                                _ => false,
                            };
                            if !supported {
                                return Err(invalid(provider, "GenerateContent does not support the requested media MIME type."));
                            }
                            if let Some(data) = data {
                                json!({"inlineData": {"mimeType": mime_type, "data": data}})
                            } else {
                                json!({"fileData": {"mimeType": mime_type, "fileUri": url}})
                            }
                        }
                        _ => return Err(invalid(provider, "Unknown content encoder.")),
                    }
                }
                ContentBlock::Native {
                    provider: owner,
                    payload,
                } => {
                    if owner != provider {
                        return Err(invalid(provider, "Cross-provider native continuation."));
                    }
                    payload.clone()
                }
                ContentBlock::Reasoning {
                    provider: owner,
                    text,
                    signature,
                } => {
                    if owner != provider {
                        return Err(invalid(provider, "Cross-provider reasoning continuation."));
                    }
                    match provider {
                        "anthropic" => {
                            json!({"type": "thinking", "thinking": text, "signature": signature})
                        }
                        "google" => {
                            json!({"text": text, "thought": true, "thoughtSignature": signature})
                        }
                        _ => {
                            return Err(invalid(
                                provider,
                                "Responses reasoning must retain its native item envelope.",
                            ))
                        }
                    }
                }
                ContentBlock::ToolCall {
                    id,
                    name,
                    arguments,
                } => {
                    tool_names.insert(id.as_str(), name.as_str());
                    match provider {
                        "openai" => {
                            json!({"type": "function_call", "call_id": id, "name": name, "arguments": arguments.to_string()})
                        }
                        "anthropic" => {
                            json!({"type": "tool_use", "id": id, "name": name, "input": arguments})
                        }
                        _ => json!({"functionCall": {"id": id, "name": name, "args": arguments}}),
                    }
                }
                ContentBlock::ToolResult { id, content } => match provider {
                    "openai" => {
                        json!({"type": "function_call_output", "call_id": id, "output": content.to_string()})
                    }
                    "anthropic" => {
                        json!({"type": "tool_result", "tool_use_id": id, "content": content.to_string()})
                    }
                    _ => {
                        json!({"functionResponse": {"id": id, "name": tool_names.get(id.as_str()).ok_or_else(|| invalid(provider, "Tool result has no named call."))?, "response": if content.is_object() { content.clone() } else { json!({"result": content}) }}})
                    }
                },
            };
            if provider == "openai"
                && matches!(
                    block,
                    ContentBlock::Native { .. }
                        | ContentBlock::ToolCall { .. }
                        | ContentBlock::ToolResult { .. }
                )
            {
                if !blocks.is_empty() {
                    result.push(json!({"role": role, "content": std::mem::take(&mut blocks)}));
                }
                result.push(native);
            } else {
                blocks.push(native);
            }
        }
        if !blocks.is_empty() {
            result.push(if provider == "google" {
                json!({"role": role, "parts": blocks})
            } else {
                json!({"role": role, "content": blocks})
            });
        }
    }
    Ok(result)
}

pub(crate) fn output_format(
    provider: &str,
    body: &mut Value,
    chat: &battersea_model::engine::EngineChatParameters,
) {
    if let Some(format) = &chat.response_format {
        use battersea_model::ResponseFormat::*;
        match provider {
            "openai" => {
                body["text"]["format"] = match format {
                    JsonObject => json!({"type": "json_object"}),
                    JsonSchema { name, schema } => {
                        json!({"type": "json_schema", "name": name, "schema": schema, "strict": true})
                    }
                }
            }
            "anthropic" => {
                if let JsonSchema { schema, .. } = format {
                    body["output_config"]["format"] =
                        json!({"type": "json_schema", "schema": schema});
                }
            }
            "google" => {
                body["generationConfig"]["responseMimeType"] = json!("application/json");
                if let JsonSchema { schema, .. } = format {
                    body["generationConfig"]["responseJsonSchema"] = schema.clone();
                }
            }
            _ => {}
        }
    }
    if provider == "google" {
        if let Some(modalities) = &chat.output_modalities {
            body["generationConfig"]["responseModalities"] = json!(modalities
                .iter()
                .map(|m| match m {
                    Modality::Text => "TEXT",
                    Modality::Image => "IMAGE",
                    Modality::Audio => "AUDIO",
                    _ => "UNSUPPORTED",
                })
                .collect::<Vec<_>>());
        }
    }
}
