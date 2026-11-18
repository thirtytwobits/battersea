//! Completion validation shared by every registered provider.
use super::{
    error::Failure, payload::PROVIDER_PAYLOAD_BYTES, EngineTextStream, EngineTextStreamEvent,
};
use crate::ResponseFormat;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How a turn ended, as far as control flow is concerned. The provider's own terminal payload
/// travels beside the class.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopClass {
    Complete,
    ToolUse,
    Length,
    Refusal,
    /// A native reason Battersea does not recognise.
    Other,
}
impl StopClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::ToolUse => "tool_use",
            Self::Length => "length",
            Self::Refusal => "refusal",
            Self::Other => "other",
        }
    }
}

/// The failure of a final turn that ended in any class but `complete`. `native` is the
/// provider's terminal payload and `partial` the text Battersea held undelivered.
pub fn terminal_failure(
    provider: &str,
    class: StopClass,
    native: Value,
    partial: String,
) -> Failure {
    let (kind, message) = match class {
        StopClass::Refusal => (crate::ErrorKind::Refusal, "Provider refused the request."),
        StopClass::Complete | StopClass::ToolUse | StopClass::Length | StopClass::Other => (
            crate::ErrorKind::Incomplete,
            "Provider stopped before completing its result.",
        ),
    };
    let mut failure = Failure::new(provider, message, kind);
    if !native.is_null() {
        failure = failure.with_native(crate::Reporter::Provider, native);
    }
    if !partial.is_empty() {
        failure = failure.with_partial(crate::PartialOutput {
            text: partial,
            blocks: vec![],
        });
    }
    failure
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
        stop: Option<StopClass>,
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
                                    let failure = held(
                                        Failure::invalid_response(
                                            &state.provider,
                                            "Provider returned an unrequested output modality.",
                                        ),
                                        &mut state.text,
                                    );
                                    return Some((Err(failure), state));
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
                                    Err(Failure::invalid_response(
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
                                    let failure = held(
                                        Failure::invalid_response(
                                            &state.provider,
                                            "Structured output exceeds its byte limit.",
                                        ),
                                        &mut state.text,
                                    );
                                    return Some((Err(failure), state));
                                }
                                state.text.push_str(text);
                                continue;
                            }
                            EngineTextStreamEvent::MessageStop {
                                class: StopClass::ToolUse,
                                ..
                            } => {
                                state.text.clear();
                                state.stop = Some(StopClass::ToolUse);
                            }
                            EngineTextStreamEvent::MessageStop { class, native } => {
                                state.done = true;
                                if *class != StopClass::Complete {
                                    let failure = terminal_failure(
                                        &state.provider,
                                        *class,
                                        native.clone(),
                                        std::mem::take(&mut state.text),
                                    );
                                    return Some((Err(failure), state));
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
                                        Err(error) => {
                                            let failure = held(error, &mut state.text);
                                            return Some((Err(failure), state));
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                        Ok(event)
                    }
                    Some(Err(error)) => {
                        state.done = true;
                        Err(held(error, &mut state.text))
                    }
                    None => {
                        state.done = true;
                        Err(held(
                            Failure::invalid_response(
                                &state.provider,
                                "Provider stream ended without completing its final message.",
                            ),
                            &mut state.text,
                        ))
                    }
                };
                return Some((event, state));
            }
        },
    ))
}

/// Moves buffered structured text into a failure that carries no partial output.
fn held(mut failure: Failure, text: &mut String) -> Failure {
    if !text.is_empty() && failure.partial == crate::Detail::Absent {
        failure = failure.with_partial(crate::PartialOutput {
            text: std::mem::take(text),
            blocks: vec![],
        });
    }
    failure
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
                class: StopClass::ToolUse,
                native: Value::Null,
            },
            EngineTextStreamEvent::MessageStart { turn_index: 1 },
            EngineTextStreamEvent::TextDelta {
                text: expected.to_string(),
            },
            EngineTextStreamEvent::MessageStop {
                class: StopClass::Complete,
                native: Value::Null,
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
            ("{\"ok\":true}", Some(StopClass::Refusal)),
            ("{\"ok\":true}", Some(StopClass::Length)),
            ("{", Some(StopClass::Complete)),
            ("{\"ok\":3}", Some(StopClass::Complete)),
        ] {
            let mut input = vec![Ok(EngineTextStreamEvent::TextDelta { text: text.into() })];
            if let Some(class) = reason {
                input.push(Ok(EngineTextStreamEvent::MessageStop {
                    class,
                    native: json!({"stop_reason": "fixture"}),
                }));
            }
            let output = validate_output(
                Box::pin(futures_util::stream::iter(input)),
                "fixture".into(),
                Some(format()),
                vec![crate::Modality::Text],
            )
            .collect::<Vec<_>>()
            .await;
            let failure = output.last().unwrap().as_ref().unwrap_err();
            assert!(!output
                .iter()
                .any(|event| matches!(event, Ok(EngineTextStreamEvent::StructuredOutput { .. }))));
            // contracts/failures.md: a turn that does not complete fails with the provider's
            // terminal payload as native detail and the buffered text as partial output.
            let kind = match reason {
                Some(StopClass::Refusal) => Some(crate::ErrorKind::Refusal),
                Some(StopClass::Length) => Some(crate::ErrorKind::Incomplete),
                _ => None,
            };
            if let Some(kind) = kind {
                assert_eq!(failure.kind, kind);
                assert_eq!(failure.reporter, crate::Reporter::Provider);
                assert_eq!(
                    failure.native.present(),
                    Some(&json!({"stop_reason": "fixture"}))
                );
            }
            // Text that fails parsing or validation, or that its stream abandons, is held
            // output too.
            assert_eq!(
                failure
                    .partial
                    .present()
                    .map(|partial| partial.text.as_str()),
                Some(text)
            );
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
