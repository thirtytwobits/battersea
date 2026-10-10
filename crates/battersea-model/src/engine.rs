//! Copyright (c) Scott A Dixon
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
/// How ChatAPI flows execute tool calls. The engine runs the tool loop
/// in-process for every supported provider; this enum is single-variant
/// so flows that serialise `tool_execution: engine-orchestrated` round-
/// trip cleanly. Any unrecognised value rejects at deserialise time.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EngineToolExecutionMode {
    EngineOrchestrated,
}

impl EngineToolExecutionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EngineOrchestrated => "engine-orchestrated",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum EngineChatToolChoiceMode {
    #[default]
    Auto,
    None,
    Required,
    Any,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EngineChatToolChoice {
    pub mode: EngineChatToolChoiceMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

impl Default for EngineChatToolChoice {
    fn default() -> Self {
        Self {
            mode: EngineChatToolChoiceMode::Auto,
            tool_name: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EngineReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
    Max,
}

impl EngineReasoningEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EngineReasoningSummary {
    None,
    Auto,
    Concise,
    Detailed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EngineThinkingVisibility {
    Summarized,
    Omitted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EngineResponseVerbosity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EngineContextOverflow {
    Error,
    ProviderTruncate,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EnginePromptCacheRetention {
    InMemory,
    #[serde(rename = "24h")]
    TwentyFourHours,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EngineChatParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<crate::ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_modalities: Option<Vec<crate::Modality>>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    pub temperature: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(default)]
    pub stop_sequences: Vec<String>,
    pub tool_execution: EngineToolExecutionMode,
    pub max_tool_rounds: u32,
    pub tool_choice: EngineChatToolChoice,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict_tool_inputs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_metadata: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<EngineReasoningEffort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_summary: Option<EngineReasoningSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_budget_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_visibility: Option<EngineThinkingVisibility>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_verbosity: Option<EngineResponseVerbosity>,
    pub context_overflow: EngineContextOverflow,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_retention: Option<EnginePromptCacheRetention>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_response: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_provider_tool_calls: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EngineTemperatureDispatch {
    Always,
    Never,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EngineAnthropicThinkingCapabilities {
    pub mode: String,
    pub disable_supported: bool,
    pub manual_budget_supported: bool,
    #[serde(default)]
    pub output_effort: HashMap<String, String>,
    #[serde(default)]
    pub budget_tokens_by_effort: HashMap<String, u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EngineGoogleThinkingCapabilities {
    pub effort_field: String,
    #[serde(default)]
    pub effort_values: HashMap<String, Value>,
}

/// Engine-INTERNAL backend capability record. Loaded from `engine.yaml` and
/// consumed inside the engine only — chiefly by the chat adapters, which read
/// `anthropic_thinking` / `google_thinking` to translate the provider-neutral
/// `reasoningEffort` control onto each provider's native thinking API.
///
/// This type is deliberately NOT part of the RPC contract: RPC clients see
/// [`EngineBackendSummaryCapabilities`], which carries only provider-neutral
/// fields. Model-specific detail must never cross the engine boundary — the
/// stable "model API" is the whole point. If you add a provider-specific
/// field here, keep it out of the summary.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EngineBackendCapabilities {
    pub content: crate::ContentCapabilities,
    #[serde(default)]
    pub supported_chat_parameters: Vec<String>,
    #[serde(default)]
    pub supported_tool_execution_modes: Vec<String>,
    pub supported_tool_choices: Vec<EngineChatToolChoiceMode>,
    pub supported_reasoning_efforts: Vec<EngineReasoningEffort>,
    pub temperature_dispatch: EngineTemperatureDispatch,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort_when_unset: Option<EngineReasoningEffort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic_thinking: Option<EngineAnthropicThinkingCapabilities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub google_thinking: Option<EngineGoogleThinkingCapabilities>,
}

/// Provider-neutral backend capabilities exposed to RPC clients on
/// backend summaries. This is the stable "model API": a client programs
/// against these fields without knowing (or caring) which provider or model
/// backs an id. Provider-specific translation detail — e.g. how a model maps
/// the neutral `reasoningEffort` control onto its native thinking controls —
/// stays inside the engine and never appears here.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EngineBackendSummaryCapabilities {
    pub content: crate::ContentCapabilities,
    #[serde(default)]
    pub supported_chat_parameters: Vec<String>,
    #[serde(default)]
    pub supported_tool_execution_modes: Vec<String>,
    pub supported_tool_choices: Vec<EngineChatToolChoiceMode>,
    pub supported_reasoning_efforts: Vec<EngineReasoningEffort>,
    pub temperature_dispatch: EngineTemperatureDispatch,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort_when_unset: Option<EngineReasoningEffort>,
}

impl EngineBackendCapabilities {
    /// Projects the engine-internal capability record onto the
    /// provider-neutral surface exposed to RPC clients, dropping every
    /// provider-specific field.
    pub fn to_summary(&self) -> EngineBackendSummaryCapabilities {
        EngineBackendSummaryCapabilities {
            content: self.content.clone(),
            supported_chat_parameters: self.supported_chat_parameters.clone(),
            supported_tool_execution_modes: self.supported_tool_execution_modes.clone(),
            supported_tool_choices: self.supported_tool_choices.clone(),
            supported_reasoning_efforts: self.supported_reasoning_efforts.clone(),
            temperature_dispatch: self.temperature_dispatch,
            reasoning_effort_when_unset: self.reasoning_effort_when_unset,
        }
    }
}

impl EngineBackendCapabilities {
    pub fn validate_reasoning_effort(
        &self,
        effort: Option<EngineReasoningEffort>,
    ) -> Result<(), String> {
        let Some(effort) = effort else {
            return Ok(());
        };
        if !self
            .supported_chat_parameters
            .iter()
            .any(|key| key == "reasoningEffort")
            || !self.supported_reasoning_efforts.contains(&effort)
        {
            return Err(format!(
                "Backend does not support reasoning effort '{}'. Supported values: {}.",
                effort.as_str(),
                self.supported_reasoning_efforts
                    .iter()
                    .map(|value| value.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(thinking) = &self.google_thinking {
            if !thinking.effort_values.contains_key(effort.as_str()) {
                return Err(format!(
                    "Missing thinking mapping for reasoning effort '{}'.",
                    effort.as_str()
                ));
            }
        }
        if let Some(thinking) = &self.anthropic_thinking {
            let mapped = if effort == EngineReasoningEffort::None {
                thinking.disable_supported
            } else if thinking.mode == "adaptive" {
                thinking.output_effort.contains_key(effort.as_str())
            } else {
                thinking
                    .budget_tokens_by_effort
                    .contains_key(effort.as_str())
            };
            if !mapped {
                return Err(format!(
                    "Missing thinking mapping for reasoning effort '{}'.",
                    effort.as_str()
                ));
            }
        }
        Ok(())
    }

    pub fn validate_chat_parameters(&self, chat: &EngineChatParameters) -> Result<(), String> {
        if chat.response_format.is_some()
            && !self
                .supported_chat_parameters
                .iter()
                .any(|name| name == "responseFormat")
        {
            return Err("Backend does not advertise responseFormat.".into());
        }
        if chat.output_modalities.is_some()
            && !self
                .supported_chat_parameters
                .iter()
                .any(|name| name == "outputModalities")
        {
            return Err("Backend does not advertise outputModalities.".into());
        }
        if let Some(format) = &chat.response_format {
            if !self.content.structured_output.contains(&format.mode()) {
                return Err(
                    "Backend does not support the requested structured output mode.".into(),
                );
            }
            format.validate()?;
        }
        if let Some(modalities) = &chat.output_modalities {
            if modalities.is_empty()
                || modalities
                    .iter()
                    .any(|m| !self.content.output_modalities.contains(m))
            {
                return Err("Backend does not support the requested output modalities.".into());
            }
            if chat.response_format.is_some() && modalities != &[crate::Modality::Text] {
                return Err("Structured output requires text output.".into());
            }
        }
        self.validate_tool_choice(&chat.tool_choice)?;
        self.validate_reasoning_effort(chat.reasoning_effort)
    }

    pub fn validate_tool_choice(&self, choice: &EngineChatToolChoice) -> Result<(), String> {
        if !self.supported_tool_choices.contains(&choice.mode) {
            return Err(format!(
                "Backend does not support tool choice mode {:?}.",
                choice.mode
            ));
        }
        if choice.mode == EngineChatToolChoiceMode::Tool
            && choice
                .tool_name
                .as_deref()
                .is_none_or(|name| name.trim().is_empty())
        {
            return Err("Named tool choice requires a tool name.".to_string());
        }
        Ok(())
    }

    pub fn mock() -> Self {
        Self {
            content: crate::ContentCapabilities::text(),
            supported_chat_parameters: vec![
                "responseFormat".to_string(),
                "outputModalities".to_string(),
                "stream".to_string(),
                "maxOutputTokens".to_string(),
                "temperature".to_string(),
                "topP".to_string(),
                "topK".to_string(),
                "stopSequences".to_string(),
                "toolExecution".to_string(),
                "maxToolRounds".to_string(),
                "toolChoice".to_string(),
                "parallelToolCalls".to_string(),
                "strictToolInputs".to_string(),
                "serviceTier".to_string(),
                "safetyIdentifier".to_string(),
                "requestMetadata".to_string(),
                "reasoningEffort".to_string(),
                "reasoningSummary".to_string(),
                "thinkingBudgetTokens".to_string(),
                "thinkingVisibility".to_string(),
                "responseVerbosity".to_string(),
                "contextOverflow".to_string(),
                "promptCacheKey".to_string(),
                "promptCacheRetention".to_string(),
                "storeResponse".to_string(),
                "maxProviderToolCalls".to_string(),
                "logprobs".to_string(),
                "topLogprobs".to_string(),
            ],
            supported_tool_execution_modes: vec!["engine-orchestrated".to_string()],
            supported_tool_choices: vec![
                EngineChatToolChoiceMode::Auto,
                EngineChatToolChoiceMode::None,
                EngineChatToolChoiceMode::Required,
                EngineChatToolChoiceMode::Any,
                EngineChatToolChoiceMode::Tool,
            ],
            supported_reasoning_efforts: vec![
                EngineReasoningEffort::None,
                EngineReasoningEffort::Minimal,
                EngineReasoningEffort::Low,
                EngineReasoningEffort::Medium,
                EngineReasoningEffort::High,
                EngineReasoningEffort::XHigh,
                EngineReasoningEffort::Max,
            ],
            temperature_dispatch: EngineTemperatureDispatch::Always,
            reasoning_effort_when_unset: None,
            anthropic_thinking: None,
            google_thinking: None,
        }
    }
}

impl EngineChatParameters {
    /// Neutral placeholder parameters for a backend that carries no real
    /// model — the disabled "unavailable backend" sentinel and in-crate
    /// tests. This is NOT a config fallback: the live config loader
    /// (`resolve_chat_parameters`) never calls it and instead hard-errors
    /// when a backend omits a required chat property. The `provider`
    /// argument is retained only so existing test call-sites compile; it
    /// no longer selects any provider- or model-specific value.
    pub fn default_for_provider(_provider: &str) -> Self {
        Self {
            response_format: None,
            output_modalities: None,
            stream: true,
            max_output_tokens: None,
            temperature: 1.0,
            top_p: None,
            top_k: None,
            stop_sequences: Vec::new(),
            tool_execution: EngineToolExecutionMode::EngineOrchestrated,
            max_tool_rounds: 8,
            tool_choice: EngineChatToolChoice::default(),
            parallel_tool_calls: None,
            strict_tool_inputs: None,
            service_tier: None,
            safety_identifier: None,
            request_metadata: None,
            reasoning_effort: None,
            reasoning_summary: None,
            thinking_budget_tokens: None,
            thinking_visibility: None,
            response_verbosity: None,
            context_overflow: EngineContextOverflow::Error,
            prompt_cache_key: None,
            prompt_cache_retention: None,
            store_response: None,
            max_provider_tool_calls: None,
            logprobs: None,
            top_logprobs: None,
        }
    }

    pub fn max_tool_rounds(&self) -> usize {
        self.max_tool_rounds.clamp(1, 32) as usize
    }
}
