//! Deterministic text transport using the same tool runner and events as live providers.
use super::tool_loop::*;
use super::*;
use async_trait::async_trait;
use battersea_model::{EngineAdapterRequestError, Role};
use std::sync::Arc;
use tokio_stream::wrappers::ReceiverStream;

pub(crate) fn create_mock_adapter(
    backend: EngineBackendConfig,
    logger: Option<Arc<dyn EngineAdapterLogger>>,
) -> Result<Arc<dyn EngineAdapter>, EngineAdapterRequestError> {
    Ok(Arc::new(MockAdapter { backend, logger }))
}
struct MockAdapter {
    backend: EngineBackendConfig,
    logger: Option<Arc<dyn EngineAdapterLogger>>,
}
#[async_trait]
impl EngineAdapter for MockAdapter {
    fn describe_debug_context(&self) -> EngineAdapterDebugContext {
        EngineAdapterDebugContext {
            provider: "mock".into(),
            backend: self.backend.id.clone(),
            model: self.backend.model.clone(),
            sdk: "local-mock".into(),
            base_url: None,
            timeout_ms: self.backend.options.timeout_ms,
            max_retries: Some(0),
        }
    }
    async fn count_text_stream_input_tokens(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<u64, EngineAdapterRequestError> {
        validate_chat_request(&self.backend, &request)?;
        Ok(request
            .shared
            .messages
            .iter()
            .map(|message| estimate_mock_token_count(&message.text_content()))
            .sum())
    }
    async fn stream_text(
        &self,
        request: EngineTextStreamRequest,
    ) -> Result<EngineTextStream, EngineAdapterRequestError> {
        validate_chat_request(&self.backend, &request)?;
        let failure = self.backend.options.mock_failure.clone();
        if let Some(failure) = failure
            .as_ref()
            .filter(|failure| failure.after_chunks.is_none())
        {
            return Err(EngineAdapterRequestError::new(
                "mock",
                failure.message.clone(),
                failure.classification.clone(),
            ));
        }
        let context = self.describe_debug_context();
        emit_request(self.logger.as_ref(), &context, &request.shared, None, None).await;
        emit_response(
            self.logger.as_ref(),
            &context,
            request.shared.operation,
            200,
            EngineAdapterResponseDetail {
                ok: true,
                request_id: None,
                response_id: None,
                output_chars: None,
                usage: None,
                streaming: Some(true),
                stop_reason: None,
            },
        )
        .await;
        let conversation = MockConversation {
            options: self.backend.options.clone(),
            prompt: request.shared.text_for_role(Role::User),
            round: 0,
            logger: self.logger.clone(),
            context: context.clone(),
        };
        let executor = request
            .local_tool_executor
            .clone()
            .unwrap_or_else(|| Arc::new(battersea_model::ToolRegistry::default()));
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let logger = self.logger.clone();
        tokio::spawn(run_tool_loop(
            conversation,
            request,
            executor,
            logger,
            context,
            tx,
        ));
        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}
struct MockConversation {
    options: EngineBackendOptions,
    prompt: String,
    round: u32,
    logger: Option<Arc<dyn EngineAdapterLogger>>,
    context: EngineAdapterDebugContext,
}
#[async_trait]
impl ToolConversation for MockConversation {
    async fn next_turn(
        &mut self,
        request: &EngineTextStreamRequest,
        results: Vec<ExecutedToolCall>,
        events: &ToolEventSender,
    ) -> Result<Vec<EngineLocalToolCall>, EngineAdapterRequestError> {
        for result in results {
            let serialized = result.result.content.to_string();
            let collapsed = serialized.split_whitespace().collect::<Vec<_>>().join(" ");
            let summary = if collapsed.chars().count() > 160 {
                format!("{}...", collapsed.chars().take(157).collect::<String>())
            } else {
                collapsed
            };
            self.prompt = format!(
                "{} | tool {} => {}",
                self.prompt.trim(),
                result.call.name,
                summary
            );
        }
        let next_round = self
            .options
            .mock_tool_calls
            .iter()
            .flatten()
            .filter(|call| call.round > self.round)
            .map(|call| call.round)
            .min();
        if let Some(round) = next_round {
            self.round = round;
            return Ok(self
                .options
                .mock_tool_calls
                .iter()
                .flatten()
                .enumerate()
                .filter(|(_, call)| call.round == round)
                .map(|(index, call)| EngineLocalToolCall {
                    id: format!("mock-tool-r{round}-{index}"),
                    name: local_tool_name(&call.name),
                    arguments: if call.arguments.is_null() {
                        serde_json::json!({})
                    } else {
                        call.arguments.clone()
                    },
                })
                .collect());
        }
        let response = match &request.mock_response {
            Some(render) => render(&self.prompt),
            None => self.prompt.clone(),
        };
        for text in self
            .options
            .mock_streaming_reasoning_chunks()
            .unwrap_or_default()
        {
            send_event(events, EngineTextStreamEvent::ReasoningDelta { text }).await?;
        }
        let input_tokens: u64 = request
            .shared
            .messages
            .iter()
            .map(|message| estimate_mock_token_count(&message.text_content()))
            .sum();
        let mut output_tokens = 0;
        for (index, text) in chunk_text_for_mock_stream(&response)
            .into_iter()
            .enumerate()
        {
            let delay =
                mock_stream_delay_ms(&text, index, self.options.mock_stream_delay_multiplier());
            if delay > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
            emit_stream_event(
                self.logger.as_ref(),
                &self.context,
                request.shared.operation,
                "delta",
                serde_json::json!({"chars": text.chars().count()}),
            )
            .await;
            output_tokens += estimate_mock_token_count(&text);
            send_event(events, EngineTextStreamEvent::TextDelta { text }).await?;
            send_event(
                events,
                EngineTextStreamEvent::TokenUsage {
                    usage: EngineTokenUsage {
                        input_tokens: Some(input_tokens),
                        output_tokens: Some(output_tokens),
                        total_tokens: Some(input_tokens + output_tokens),
                    },
                },
            )
            .await?;
            if let Some(failure) = self
                .options
                .mock_failure
                .as_ref()
                .filter(|failure| failure.after_chunks == Some(index as u32 + 1))
            {
                return Err(EngineAdapterRequestError::new(
                    "mock",
                    failure.message.clone(),
                    failure.classification.clone(),
                ));
            }
        }
        Ok(Vec::new())
    }
}

const MOCK_STREAM_CHUNK_TARGETS: [usize; 6] = [12, 20, 16, 24, 14, 18];
const MOCK_STREAM_DELAY_JITTER_MS: [u64; 6] = [0, 18, 7, 22, 11, 15];
const MOCK_STREAM_INITIAL_DELAY_MS: u64 = 160;
const MOCK_STREAM_BASE_DELAY_MS: u64 = 36;
const MOCK_STREAM_PER_CHAR_DELAY_MS: u64 = 7;
const MOCK_STREAM_CLAUSE_PAUSE_MS: u64 = 45;
const MOCK_STREAM_SENTENCE_PAUSE_MS: u64 = 95;
const MOCK_STREAM_PARAGRAPH_PAUSE_MS: u64 = 160;
const MOCK_STREAM_MAX_DELAY_MS: u64 = 280;

pub fn estimate_mock_token_count(text: &str) -> u64 {
    if text.is_empty() {
        0
    } else {
        text.chars().count().div_ceil(4) as u64
    }
}

/// Splits mock streamed text into human-sized chunks while preserving the
/// exact original text when re-concatenated.
///
/// Chunk boundaries prefer paragraph, sentence, clause, and whitespace
/// breaks, and no empty chunks are emitted.
pub fn chunk_text_for_mock_stream(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);

        let target = MOCK_STREAM_CHUNK_TARGETS[chunks.len() % MOCK_STREAM_CHUNK_TARGETS.len()];
        let trimmed = current.trim_end_matches(char::is_whitespace);
        let last_character = trimmed.chars().last().unwrap_or_default();
        let ends_paragraph = current.ends_with("\n\n");
        let ends_sentence = matches!(last_character, '.' | '!' | '?');
        let ends_clause = matches!(last_character, ',' | ';')
            && trimmed.chars().count() >= target.saturating_sub(4);
        let break_at_whitespace = current.chars().count() >= target && ch.is_whitespace();

        if ends_paragraph || ends_sentence || ends_clause || break_at_whitespace {
            chunks.push(current.clone());
            current.clear();
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Computes the synthetic delay applied before a mock stream chunk is emitted.
///
/// The formula is deterministic so integration tests can reason about event
/// order, while still feeling closer to a real provider stream than a fixed
/// sleep between every chunk.
pub fn mock_stream_delay_ms(chunk: &str, index: usize, delay_multiplier: f64) -> u64 {
    let trimmed = chunk.split_whitespace().collect::<Vec<_>>().join(" ");
    if trimmed.is_empty() {
        return 0;
    }

    let mut delay = if index == 0 {
        MOCK_STREAM_INITIAL_DELAY_MS
    } else {
        MOCK_STREAM_BASE_DELAY_MS
    };
    delay += (trimmed.chars().count() as u64 * MOCK_STREAM_PER_CHAR_DELAY_MS).min(120);
    delay += MOCK_STREAM_DELAY_JITTER_MS[index % MOCK_STREAM_DELAY_JITTER_MS.len()];

    if chunk.contains("\n\n") {
        delay += MOCK_STREAM_PARAGRAPH_PAUSE_MS;
    } else if trimmed.ends_with('.') || trimmed.ends_with('!') || trimmed.ends_with('?') {
        delay += MOCK_STREAM_SENTENCE_PAUSE_MS;
    } else if trimmed.ends_with(',') || trimmed.ends_with(';') {
        delay += MOCK_STREAM_CLAUSE_PAUSE_MS;
    }

    ((delay.min(MOCK_STREAM_MAX_DELAY_MS)) as f64 * delay_multiplier)
        .round()
        .clamp(0.0, u64::MAX as f64) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mock_stream_chunking_preserves_text_and_groups_words() {
        let text =
            "Narrator: The room holds its breath.\n\nIris Vale: We move now, before the lamps go out.";
        let chunks = chunk_text_for_mock_stream(text);

        assert_eq!(chunks.concat(), text);
        assert!(chunks.len() > 1);
        assert!(chunks
            .iter()
            .any(|chunk| chunk.split_whitespace().count() > 1));
    }

    #[test]
    fn mock_stream_chunking_preserves_exact_text_without_empty_chunks() {
        let text = "Clause one, clause two; sentence three.\n\nParagraph four.";
        let chunks = chunk_text_for_mock_stream(text);

        assert_eq!(chunks.concat(), text);
        assert!(chunks.iter().all(|chunk| !chunk.is_empty()));
        assert!(chunks.iter().any(|chunk| chunk.ends_with("\n\n")));
    }

    #[test]
    fn mock_stream_delay_adds_extra_pause_for_sentence_endings() {
        assert!(
            mock_stream_delay_ms("The room waits.", 1, 1.0)
                > mock_stream_delay_ms("The room waits", 1, 1.0)
        );
        assert!(
            mock_stream_delay_ms("The room waits.\n\n", 1, 1.0)
                > mock_stream_delay_ms("The room waits.", 1, 1.0)
        );
    }

    #[test]
    fn mock_stream_delay_returns_zero_for_whitespace_and_delays_first_chunk_more() {
        assert_eq!(mock_stream_delay_ms("   \n\t", 0, 1.0), 0);
        assert!(
            mock_stream_delay_ms("Lantern lit", 0, 1.0)
                > mock_stream_delay_ms("Lantern lit", 1, 1.0)
        );
    }

    #[test]
    fn mock_stream_delay_multiplier_scales_delays() {
        let baseline = mock_stream_delay_ms("The room waits.", 1, 1.0);
        assert_eq!(mock_stream_delay_ms("The room waits.", 1, 0.0), 0);
        assert!(mock_stream_delay_ms("The room waits.", 1, 0.5) < baseline);
        assert!(mock_stream_delay_ms("The room waits.", 1, 2.0) > baseline);
    }
}
