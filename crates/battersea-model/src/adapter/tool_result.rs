//! Copyright (c) Scott A Dixon
//! Tool output admitted independently from its provider representation.
use crate::{EngineAdapterRequestError, ErrorKind, ToolCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// Ordered content returned by a tool. References are retained without fetching them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolContentBlock {
    Text {
        text: String,
    },
    Image {
        url: String,
        mime_type: String,
    },
    Audio {
        url: String,
        mime_type: String,
    },
    ResourceLink {
        uri: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },
    Resource {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        contents: ToolResourceContents,
    },
}

/// The inline payload of a resource returned by a tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "encoding", content = "data", rename_all = "snake_case")]
pub enum ToolResourceContents {
    Text(String),
    Base64(String),
}

/// Content and structured output returned by a tool.
///
/// `meta` belongs to the host. Provider projections and debug formatting exclude its values.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolContentResult {
    pub content_blocks: Vec<ToolContentBlock>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_structured_content"
    )]
    pub structured_content: Option<Value>,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default, rename = "_meta", skip_serializing_if = "Map::is_empty")]
    pub meta: Map<String, Value>,
}

fn present_structured_content<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

impl std::fmt::Debug for ToolContentResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContentResult")
            .field("content_blocks", &self.content_blocks)
            .field("structured_content", &self.structured_content)
            .field("is_error", &self.is_error)
            .field("metadata_entries", &self.meta.len())
            .finish()
    }
}

/// A tool result, retaining the ordinary JSON representation or explicit content semantics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged, deny_unknown_fields)]
pub enum EngineLocalToolResult {
    Json { content: Value },
    Content(ToolContentResult),
}

impl EngineLocalToolResult {
    /// An ordinary application JSON result.
    pub fn json(content: Value) -> Self {
        Self::Json { content }
    }

    /// Whether the tool completed with an execution error.
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Content(result) if result.is_error)
    }

    /// The value checked against a tool's output schema.
    pub fn structured_output(&self) -> Option<&Value> {
        match self {
            Self::Json { content } => Some(content),
            Self::Content(result) => result.structured_content.as_ref(),
        }
    }

    /// The JSON representation supported by text tool conversations.
    ///
    /// Text, resource references and embedded text retain their order. Binary content requires
    /// a provider-specific content translation and is refused before the next provider request.
    pub fn provider_content(&self) -> Result<Value, EngineAdapterRequestError> {
        match self {
            Self::Json { content } => Ok(content.clone()),
            Self::Content(result) => {
                if result.content_blocks.iter().any(|block| {
                    matches!(
                        block,
                        ToolContentBlock::Image { .. }
                            | ToolContentBlock::Audio { .. }
                            | ToolContentBlock::Resource {
                                contents: ToolResourceContents::Base64(_),
                                ..
                            }
                    )
                }) {
                    return Err(EngineAdapterRequestError::new(
                        "tools",
                        "The tool returned content unsupported by the text tool conversation.",
                        ErrorKind::InvalidResponse,
                    )
                    .with_tool_code(ToolCode::UnsupportedToolContent));
                }
                let mut content = json!({
                    "content": result.content_blocks,
                    "isError": result.is_error,
                });
                if let Some(structured) = &result.structured_content {
                    content["structuredContent"] = structured.clone();
                }
                Ok(content)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_json_round_trips_without_changing_its_representation() {
        // An application's own "isError" field is ordinary data.
        let wire = json!({"content": {"isError": true, "answer": 42}});
        let result: EngineLocalToolResult = serde_json::from_value(wire.clone()).unwrap();
        assert!(!result.is_error());
        assert_eq!(result.provider_content().unwrap(), wire["content"]);
        assert_eq!(serde_json::to_value(result).unwrap(), wire);
    }

    #[test]
    fn structured_null_is_distinct_from_absent_output() {
        for structured_content in [None, Some(Value::Null)] {
            let result = EngineLocalToolResult::Content(ToolContentResult {
                content_blocks: vec![],
                structured_content,
                is_error: false,
                meta: Map::new(),
            });
            let wire = serde_json::to_value(&result).unwrap();
            let restored: EngineLocalToolResult = serde_json::from_value(wire).unwrap();
            assert_eq!(restored, result);
        }
    }

    #[test]
    fn mixed_result_representations_are_rejected() {
        assert!(serde_json::from_value::<EngineLocalToolResult>(json!({
            "content": {},
            "content_blocks": [],
            "_meta": {"private": "host-secret"}
        }))
        .is_err());
    }

    #[test]
    fn provider_projection_preserves_order_structure_and_error_without_host_metadata() {
        let result = EngineLocalToolResult::Content(ToolContentResult {
            content_blocks: vec![
                ToolContentBlock::Text {
                    text: "first".into(),
                },
                ToolContentBlock::ResourceLink {
                    uri: "file:///host-owned".into(),
                    name: "reference".into(),
                    mime_type: None,
                },
                ToolContentBlock::Text {
                    text: "last".into(),
                },
            ],
            structured_content: Some(json!({"reason": "missing"})),
            is_error: true,
            meta: Map::from_iter([("private".into(), json!("host-secret"))]),
        });
        assert!(result.is_error());
        let projected = result.provider_content().unwrap();
        assert_eq!(projected["content"][0]["text"], "first");
        assert_eq!(projected["content"][1]["uri"], "file:///host-owned");
        assert_eq!(projected["content"][2]["text"], "last");
        assert_eq!(projected["structuredContent"], json!({"reason": "missing"}));
        assert_eq!(projected["isError"], true);
        assert!(!projected.to_string().contains("host-secret"));
        assert!(!format!("{result:?}").contains("host-secret"));
        let retained = serde_json::to_value(&result).unwrap();
        assert_eq!(retained["_meta"]["private"], "host-secret");
        assert_eq!(
            serde_json::from_value::<EngineLocalToolResult>(retained).unwrap(),
            result
        );
    }

    #[test]
    fn binary_content_requires_a_provider_translation() {
        for block in [
            ToolContentBlock::Image {
                url: "data:image/png;base64,AA==".into(),
                mime_type: "image/png".into(),
            },
            ToolContentBlock::Audio {
                url: "data:audio/wav;base64,AA==".into(),
                mime_type: "audio/wav".into(),
            },
            ToolContentBlock::Resource {
                uri: "file:///binary".into(),
                mime_type: None,
                contents: ToolResourceContents::Base64("AA==".into()),
            },
        ] {
            let result = EngineLocalToolResult::Content(ToolContentResult {
                content_blocks: vec![block],
                structured_content: None,
                is_error: false,
                meta: Map::new(),
            });
            assert_eq!(
                result.provider_content().unwrap_err().tool_code,
                Some(ToolCode::UnsupportedToolContent)
            );
        }
    }
}
