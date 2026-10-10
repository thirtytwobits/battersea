//! Provider-neutral input. Hosts assemble their own messages and prompt text.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContentBlock {
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
    Video {
        url: String,
        mime_type: String,
    },
    Document {
        url: String,
        mime_type: String,
        filename: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
    ToolResult {
        id: String,
        content: Value,
    },
    Reasoning {
        provider: String,
        text: String,
        signature: Option<String>,
    },
    Native {
        provider: String,
        payload: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}
impl Message {
    pub fn text(role: Role, text: String) -> Self {
        Self {
            role,
            content: vec![ContentBlock::Text { text }],
        }
    }
    pub fn text_content(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_content_round_trip_preserves_signature_and_unknown_fields() {
        let message = Message {
            role: Role::Assistant,
            content: vec![ContentBlock::Native {
                provider: "fixture".into(),
                payload: serde_json::json!({"signature": "signed-by-provider", "future": [true, {"opaque": "bytes"}]}),
            }],
        };
        let encoded = serde_json::to_vec(&message).unwrap();
        assert_eq!(
            serde_json::from_slice::<Message>(&encoded).unwrap(),
            message
        );
    }
    #[test]
    fn text_admission_rejects_unsupported_content_and_late_system_messages() {
        let text = Message::text(Role::User, "input".into());
        assert!(crate::validate_messages(
            "fixture",
            &crate::ContentCapabilities::text(),
            std::slice::from_ref(&text)
        )
        .is_ok());
        for block in [
            ContentBlock::Image {
                url: "fixture".into(),
                mime_type: "image/png".into(),
            },
            ContentBlock::Native {
                provider: "fixture".into(),
                payload: Value::Null,
            },
        ] {
            assert!(crate::validate_messages(
                "fixture",
                &crate::ContentCapabilities::text(),
                &[Message {
                    role: Role::User,
                    content: vec![block]
                }]
            )
            .is_err());
        }
        assert!(crate::validate_messages(
            "fixture",
            &crate::ContentCapabilities::text(),
            &[text, Message::text(Role::System, "instructions".into())]
        )
        .is_err());
    }
}
