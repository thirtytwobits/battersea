//! Completion validation shared by every registered provider.
use super::{
    error::EngineAdapterRequestError, payload::PROVIDER_PAYLOAD_BYTES, EngineTextStream,
    EngineTextStreamEvent,
};
use crate::ResponseFormat;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Complete,
    ToolUse,
    Length,
    Refusal,
    Other(String),
}

pub fn validate_output(
    stream: EngineTextStream,
    provider: String,
    format: Option<ResponseFormat>,
    modalities: Vec<crate::Modality>,
) -> EngineTextStream {
    struct State {
        stream: EngineTextStream,
        provider: String,
        format: Option<ResponseFormat>,
        modalities: Vec<crate::Modality>,
        text: String,
        stop: Option<StopReason>,
        done: bool,
        pending: std::collections::VecDeque<EngineTextStreamEvent>,
    }
    Box::pin(futures_util::stream::unfold(
        State {
            stream,
            provider,
            format,
            modalities,
            text: String::new(),
            stop: None,
            done: false,
            pending: Default::default(),
        },
        |mut state| async move {
            if let Some(event) = state.pending.pop_front() {
                return Some((Ok(event), state));
            }
            if state.done {
                return None;
            }
            loop {
                let event = match state.stream.next().await {
                    Some(Ok(event)) => {
                        match &event {
                            EngineTextStreamEvent::ContentBlock { block } => {
                                let modality = match block {
                                    crate::ContentBlock::Image { .. } => {
                                        Some(crate::Modality::Image)
                                    }
                                    crate::ContentBlock::Audio { .. } => {
                                        Some(crate::Modality::Audio)
                                    }
                                    crate::ContentBlock::Video { .. } => {
                                        Some(crate::Modality::Video)
                                    }
                                    crate::ContentBlock::Document { .. } => {
                                        Some(crate::Modality::Document)
                                    }
                                    _ => None,
                                };
                                if modality.is_some_and(|mode| !state.modalities.contains(&mode)) {
                                    state.done = true;
                                    return Some((
                                        Err(EngineAdapterRequestError::invalid_response(
                                            &state.provider,
                                            "Provider returned an unrequested output modality.",
                                        )),
                                        state,
                                    ));
                                }
                            }
                            EngineTextStreamEvent::MessageStart { .. }
                            | EngineTextStreamEvent::ToolCallStarted { .. } => {
                                state.text.clear();
                                state.stop = None;
                            }
                            EngineTextStreamEvent::TextDelta { .. }
                                if !state.modalities.contains(&crate::Modality::Text) =>
                            {
                                state.done = true;
                                return Some((
                                    Err(EngineAdapterRequestError::invalid_response(
                                        &state.provider,
                                        "Provider returned unrequested text output.",
                                    )),
                                    state,
                                ));
                            }
                            EngineTextStreamEvent::TextDelta { text } if state.format.is_some() => {
                                if text.len()
                                    > PROVIDER_PAYLOAD_BYTES.saturating_sub(state.text.len())
                                {
                                    state.done = true;
                                    return Some((
                                        Err(EngineAdapterRequestError::invalid_response(
                                            &state.provider,
                                            "Structured output exceeds its byte limit.",
                                        )),
                                        state,
                                    ));
                                }
                                state.text.push_str(text);
                                continue;
                            }
                            EngineTextStreamEvent::MessageStop {
                                reason: StopReason::ToolUse,
                            } => {
                                state.text.clear();
                                state.stop = Some(StopReason::ToolUse);
                            }
                            EngineTextStreamEvent::MessageStop { reason } => {
                                state.done = true;
                                if *reason != StopReason::Complete {
                                    return Some((Err(EngineAdapterRequestError::invalid_response(&state.provider, format!("Provider stopped without a complete result: {reason:?}."))), state));
                                }
                                if let Some(format) = &state.format {
                                    match format.parse(&state.provider, &state.text) {
                                        Ok(value) => {
                                            state.pending.push_back(
                                                EngineTextStreamEvent::StructuredOutput { value },
                                            );
                                            state.pending.push_back(event);
                                            let text = std::mem::take(&mut state.text);
                                            return Some((
                                                Ok(EngineTextStreamEvent::TextDelta { text }),
                                                state,
                                            ));
                                        }
                                        Err(error) => return Some((Err(error), state)),
                                    }
                                }
                            }
                            _ => {}
                        }
                        Ok(event)
                    }
                    Some(Err(error)) => {
                        state.done = true;
                        Err(error)
                    }
                    None => {
                        state.done = true;
                        Err(EngineAdapterRequestError::invalid_response(
                            &state.provider,
                            "Provider stream ended without completing its final message.",
                        ))
                    }
                };
                return Some((event, state));
            }
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn format() -> ResponseFormat {
        ResponseFormat::JsonSchema {
            name: "result".into(),
            schema: json!({"type":"object", "properties":{"ok":{"type":"boolean"}}, "required":["ok"], "additionalProperties":false}),
        }
    }
    #[tokio::test]
    async fn only_the_completed_final_turn_can_publish_a_validated_value() {
        let expected = json!({"ok":true});
        let input = vec![
            EngineTextStreamEvent::MessageStart { turn_index: 0 },
            EngineTextStreamEvent::TextDelta {
                text: "intermediate tool reasoning".into(),
            },
            EngineTextStreamEvent::MessageStop {
                reason: StopReason::ToolUse,
            },
            EngineTextStreamEvent::MessageStart { turn_index: 1 },
            EngineTextStreamEvent::TextDelta {
                text: expected.to_string(),
            },
            EngineTextStreamEvent::MessageStop {
                reason: StopReason::Complete,
            },
        ];
        let stream = Box::pin(futures_util::stream::iter(input.into_iter().map(Ok)));
        let output = validate_output(
            stream,
            "fixture".into(),
            Some(format()),
            vec![crate::Modality::Text],
        )
        .collect::<Vec<_>>()
        .await;
        assert!(
            matches!(output.iter().rev().nth(1),Some(Ok(EngineTextStreamEvent::StructuredOutput {value})) if value == &expected)
        );
        assert_eq!(
            output
                .iter()
                .filter(|event| matches!(event, Ok(EngineTextStreamEvent::StructuredOutput { .. })))
                .count(),
            1
        );
    }
    #[tokio::test]
    async fn refusal_truncation_malformed_json_and_schema_mismatch_never_publish_a_value() {
        for (text, reason) in [
            ("{\"ok\":true}", None),
            ("{\"ok\":true}", Some(StopReason::Refusal)),
            ("{\"ok\":true}", Some(StopReason::Length)),
            ("{", Some(StopReason::Complete)),
            ("{\"ok\":3}", Some(StopReason::Complete)),
        ] {
            let mut input = vec![Ok(EngineTextStreamEvent::TextDelta { text: text.into() })];
            if let Some(reason) = reason {
                input.push(Ok(EngineTextStreamEvent::MessageStop { reason }));
            }
            let output = validate_output(
                Box::pin(futures_util::stream::iter(input)),
                "fixture".into(),
                Some(format()),
                vec![crate::Modality::Text],
            )
            .collect::<Vec<_>>()
            .await;
            assert!(output.last().unwrap().is_err());
            assert!(!output
                .iter()
                .any(|event| matches!(event, Ok(EngineTextStreamEvent::StructuredOutput { .. }))));
        }
    }
    #[test]
    fn schema_admission_rejects_remote_refs_and_invalid_schemas_without_io() {
        for schema in [
            json!({"$ref":"https://example.test/schema"}),
            json!({"type":"invalid"}),
            json!({"properties":{"child":{"$ref":"file:///tmp/schema"}}}),
        ] {
            assert!(ResponseFormat::JsonSchema {
                name: "test".into(),
                schema
            }
            .validate()
            .is_err());
        }
        assert!(format().validate().is_ok());
    }
}
